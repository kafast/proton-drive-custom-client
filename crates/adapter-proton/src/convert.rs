//! Translation between Proton types and domain types.

use proton_drive_rs::DriveEvent;
use proton_drive_rs::Node as ProtonNode;
use proton_drive_rs::NodeKind as ProtonKind;
use proton_drive_rs::proton_sdk::api::ResponseCode;
use proton_drive_rs::proton_sdk::error::ProtonError;
use proton_drive_rs::proton_sdk::ids::{LinkId, NodeUid, VolumeId};
use protonsync_application::ports::{EventCursor, RemoteError, RemoteEvent};
use protonsync_domain::node::{Fingerprint, Node, NodeId, NodeKind, NodeName, RevisionId};
use protonsync_domain::time::Timestamp;

/// Separator in the text form of a `NodeUid` (`volume~link`).
const UID_SEPARATOR: char = '~';
const HTTP_UNAUTHORIZED: u16 = 401;
const HTTP_NOT_FOUND: u16 = 404;
const HTTP_TOO_MANY_REQUESTS: u16 = 429;
const HTTP_SERVER_ERRORS: u16 = 500;

/// Domain ID from a Proton ID.
pub(crate) fn node_id(uid: &NodeUid) -> NodeId {
    NodeId::new(uid.to_string())
}

/// Proton ID from a domain ID.
pub(crate) fn node_uid(id: &NodeId) -> Result<NodeUid, RemoteError> {
    let (volume, link) = id
        .as_str()
        .split_once(UID_SEPARATOR)
        .ok_or_else(|| RemoteError::NotFound(id.clone()))?;
    Ok(NodeUid::new(VolumeId::new(volume), LinkId::new(link)))
}

/// Proton node as a domain node. `None` if the name cannot be represented here.
pub(crate) fn node(proton: &ProtonNode) -> Option<Node> {
    let name = NodeName::new(proton.name.clone()).ok()?;
    let kind = match &proton.kind {
        ProtonKind::Folder => NodeKind::Folder,
        ProtonKind::File {
            total_size_on_storage,
            active_revision_id,
            claimed_size,
            content_sha1,
            ..
        } => {
            let size = claimed_size.unwrap_or(*total_size_on_storage);
            NodeKind::File {
                // Without a revision ID (unsealed draft) the modification time
                // serves as a substitute, so that changes are detected.
                revision: RevisionId::new(
                    active_revision_id
                        .clone()
                        .unwrap_or_else(|| format!("mtime-{}", proton.modification_time)),
                ),
                fingerprint: Fingerprint {
                    size: u64::try_from(size).unwrap_or_default(),
                    modified: Timestamp::from_unix_seconds(proton.modification_time),
                    content_hash: content_sha1.clone(),
                },
            }
        }
    };
    Some(Node {
        id: node_id(&proton.uid),
        parent: proton.parent_uid.as_ref().map(node_id),
        name,
        kind,
    })
}

/// Checks the result of a batch operation for exactly one node.
pub(crate) fn single_outcome(
    id: &NodeId,
    outcome: proton_drive_rs::proton_sdk::error::Result<
        Vec<(NodeUid, proton_drive_rs::proton_sdk::error::Result<()>)>,
    >,
) -> Result<(), RemoteError> {
    let results = outcome.map_err(|e| error(&e, Some(id), None))?;
    match results.into_iter().next() {
        Some((_, Ok(()))) => Ok(()),
        Some((_, Err(failure))) => Err(error(&failure, Some(id), None)),
        None => Err(RemoteError::NotFound(id.clone())),
    }
}

/// Counts the written bytes, because the downloads of the crates do not report them.
pub(crate) struct CountingWriter<'a, W> {
    inner: &'a mut W,
    written: u64,
}

impl<'a, W> CountingWriter<'a, W> {
    pub(crate) fn new(inner: &'a mut W) -> Self {
        Self { inner, written: 0 }
    }

    pub(crate) fn written(&self) -> u64 {
        self.written
    }
}

impl<W: std::io::Write> std::io::Write for CountingWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Counts the read bytes to detect changes of the source during the upload.
pub(crate) struct CountingReader<R> {
    inner: R,
    read: u64,
}

impl<R> CountingReader<R> {
    pub(crate) fn new(inner: R) -> Self {
        Self { inner, read: 0 }
    }

    pub(crate) fn read(&self) -> u64 {
        self.read
    }
}

impl<R: std::io::Read> std::io::Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.read += read as u64;
        Ok(read)
    }
}

/// Translates Proton events; returns them with the cursor after the last one.
pub(crate) fn events(
    events: Vec<DriveEvent>,
    cursor: &EventCursor,
) -> (Vec<RemoteEvent>, EventCursor) {
    let mut next = cursor.clone();
    let mut changes = Vec::new();
    for event in events {
        match event {
            DriveEvent::NodeUpdated {
                id,
                node_uid,
                is_trashed,
                ..
            } => {
                let node = node_id(&node_uid);
                changes.push(if is_trashed {
                    RemoteEvent::Trashed(node)
                } else {
                    RemoteEvent::Updated(node)
                });
                next = EventCursor::new(id.as_str());
            }
            DriveEvent::NodeDeleted { id, node_uid, .. } => {
                changes.push(RemoteEvent::Trashed(node_id(&node_uid)));
                next = EventCursor::new(id.as_str());
            }
            // Gap in the event stream: a full reconciliation is needed.
            DriveEvent::ContinuityLost { id } | DriveEvent::ScopeAccessLost { id } => {
                changes.push(RemoteEvent::Gap);
                next = EventCursor::new(id.as_str());
            }
            DriveEvent::CursorAdvanced { id } | DriveEvent::SharedWithMeUpdated { id } => {
                next = EventCursor::new(id.as_str());
            }
        }
    }
    (changes, next)
}

/// Translates an error of the Proton crates. `id` and `name` give context,
/// as far as known.
pub(crate) fn error(
    error: &ProtonError,
    id: Option<&NodeId>,
    name: Option<&NodeName>,
) -> RemoteError {
    match error {
        ProtonError::Api(api) => {
            if api.http_status == HTTP_UNAUTHORIZED {
                return RemoteError::Unauthorized;
            }
            match (&api.code, id, name) {
                (ResponseCode::AlreadyExists, _, Some(name)) => {
                    RemoteError::AlreadyExists(name.clone())
                }
                (ResponseCode::DoesNotExist, Some(id), _) => RemoteError::NotFound(id.clone()),
                (_, Some(id), _) if api.http_status == HTTP_NOT_FOUND => {
                    RemoteError::NotFound(id.clone())
                }
                _ if api.is_retriable()
                    || api.http_status == HTTP_TOO_MANY_REQUESTS
                    || api.http_status >= HTTP_SERVER_ERRORS =>
                {
                    RemoteError::Unavailable(api.to_string())
                }
                _ => RemoteError::Rejected(api.to_string()),
            }
        }
        ProtonError::Transport(transport) => RemoteError::Unavailable(transport.to_string()),
        ProtonError::KeysLocked => RemoteError::Unauthorized,
        other => RemoteError::Rejected(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proton_drive_rs::proton_sdk::error::ProtonApiError;

    #[test]
    fn node_ids_roundtrip() {
        let uid = NodeUid::new(VolumeId::new("vol"), LinkId::new("link"));
        let id = node_id(&uid);
        assert_eq!(id.as_str(), "vol~link");
        assert_eq!(node_uid(&id).unwrap(), uid);
    }

    #[test]
    fn malformed_ids_are_not_found() {
        let id = NodeId::new("no-separator");
        assert_eq!(node_uid(&id), Err(RemoteError::NotFound(id)));
    }

    fn api_error(code: ResponseCode, http_status: u16) -> ProtonError {
        ProtonError::Api(ProtonApiError {
            code,
            http_status,
            message: "test error".into(),
            details: None,
        })
    }

    #[test]
    fn api_errors_map_to_remote_errors() {
        let id = NodeId::new("vol~link");
        let name = NodeName::new("a").unwrap();
        let cases = [
            (
                api_error(ResponseCode::Unauthorized, 401),
                RemoteError::Unauthorized,
            ),
            (
                api_error(ResponseCode::AlreadyExists, 422),
                RemoteError::AlreadyExists(name.clone()),
            ),
            (
                api_error(ResponseCode::DoesNotExist, 422),
                RemoteError::NotFound(id.clone()),
            ),
            (
                api_error(ResponseCode::Unknown, 404),
                RemoteError::NotFound(id.clone()),
            ),
        ];
        for (proton, expected) in cases {
            assert_eq!(error(&proton, Some(&id), Some(&name)), expected);
        }
    }

    #[test]
    fn transient_and_other_api_errors_are_distinguished() {
        for status in [429, 500, 503] {
            let mapped = error(&api_error(ResponseCode::Unknown, status), None, None);
            assert!(
                matches!(mapped, RemoteError::Unavailable(_)),
                "{status}: {mapped:?}"
            );
        }
        let mapped = error(&api_error(ResponseCode::InvalidValue, 422), None, None);
        assert!(matches!(mapped, RemoteError::Rejected(_)));
    }

    #[test]
    fn locked_keys_mean_unauthorized() {
        assert_eq!(
            error(&ProtonError::KeysLocked, None, None),
            RemoteError::Unauthorized
        );
    }
}
