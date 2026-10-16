//! Übersetzung zwischen Proton-Typen und Domänentypen.

use proton_drive_rs::Node as ProtonNode;
use proton_drive_rs::NodeKind as ProtonKind;
use proton_drive_rs::proton_sdk::api::ResponseCode;
use proton_drive_rs::proton_sdk::error::ProtonError;
use proton_drive_rs::proton_sdk::ids::{LinkId, NodeUid, VolumeId};
use protonsync_application::ports::RemoteError;
use protonsync_domain::node::{Fingerprint, Node, NodeId, NodeKind, NodeName, RevisionId};
use protonsync_domain::time::Timestamp;

/// Trennzeichen in der Textform einer `NodeUid` (`volume~link`).
const UID_SEPARATOR: char = '~';
const HTTP_UNAUTHORIZED: u16 = 401;
const HTTP_NOT_FOUND: u16 = 404;
const HTTP_TOO_MANY_REQUESTS: u16 = 429;
const HTTP_SERVER_ERRORS: u16 = 500;

/// Domänen-Kennung aus einer Proton-Kennung.
pub(crate) fn node_id(uid: &NodeUid) -> NodeId {
    NodeId::new(uid.to_string())
}

/// Proton-Kennung aus einer Domänen-Kennung.
pub(crate) fn node_uid(id: &NodeId) -> Result<NodeUid, RemoteError> {
    let (volume, link) = id
        .as_str()
        .split_once(UID_SEPARATOR)
        .ok_or_else(|| RemoteError::NotFound(id.clone()))?;
    Ok(NodeUid::new(VolumeId::new(volume), LinkId::new(link)))
}

/// Proton-Knoten als Domänenknoten. `None`, wenn der Name hier nicht darstellbar ist.
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
                // Ohne Revisionskennung (unversiegelter Entwurf) dient die
                // Änderungszeit als Ersatz, damit Änderungen erkannt werden.
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

/// Übersetzt einen Fehler der Proton-Crates. `id` und `name` geben dabei Kontext,
/// soweit bekannt.
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
                _ if api.http_status == HTTP_NOT_FOUND && id.is_some() => {
                    RemoteError::NotFound(id.cloned().unwrap_or_else(|| NodeId::new("")))
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

    #[test]
    fn node_ids_roundtrip() {
        let uid = NodeUid::new(VolumeId::new("vol"), LinkId::new("link"));
        let id = node_id(&uid);
        assert_eq!(id.as_str(), "vol~link");
        assert_eq!(node_uid(&id).unwrap(), uid);
    }

    #[test]
    fn malformed_ids_are_not_found() {
        let id = NodeId::new("ohne-trenner");
        assert_eq!(node_uid(&id), Err(RemoteError::NotFound(id)));
    }

    #[test]
    fn locked_keys_mean_unauthorized() {
        assert_eq!(
            error(&ProtonError::KeysLocked, None, None),
            RemoteError::Unauthorized
        );
    }
}
