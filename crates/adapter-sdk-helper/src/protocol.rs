//! Wire format between protonsync and the SDK helper process (ADR 0025).
//!
//! Every message is one line of JSON (UTF-8, ending with `\n`):
//!
//! - request: `{"id": 7, "method": "node", "params": {"node": "…"}}`
//! - success: `{"id": 7, "ok": …}`
//! - failure: `{"id": 7, "error": {"kind": "not_found", "node": "…"}}`
//!
//! Requests may overlap; the `id` pairs a response with its request.
//! File contents travel as base64 in chunks of at most [`CHUNK_BYTES`].
//! The full description lives in `docs/sdk-helfer-protokoll.md`.
//!
//! The types are public so that tests (and maybe later other tools) can
//! implement the helper side with the same definitions.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use protonsync_application::ports::{EventCursor, RemoteError, RemoteEvent};
use protonsync_domain::node::{Fingerprint, Node, NodeId, NodeKind, NodeName, RevisionId};
use protonsync_domain::time::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Version of this protocol. The helper must answer `hello` with the same number.
pub const PROTOCOL_VERSION: u32 = 1;

/// Largest piece of file content per message (1 MiB before base64).
pub const CHUNK_BYTES: usize = 1024 * 1024;

/// A request to the helper.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Number chosen by the caller; repeated in the response.
    pub id: u64,
    /// Name of the operation, e.g. `list_children`.
    pub method: String,
    /// Parameters of the operation (an object, possibly empty).
    #[serde(default)]
    pub params: Value,
}

/// A response from the helper: exactly one of `ok` and `error` is set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// Id of the request this answers.
    pub id: u64,
    /// Result on success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<Value>,
    /// Error on failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
}

impl Response {
    /// A successful response.
    #[must_use]
    pub fn success(id: u64, ok: Value) -> Self {
        Self {
            id,
            ok: Some(ok),
            error: None,
        }
    }

    /// A failed response.
    #[must_use]
    pub fn failure(id: u64, error: WireError) -> Self {
        Self {
            id,
            ok: None,
            error: Some(error),
        }
    }
}

/// Error kinds on the wire; they mirror [`RemoteError`] one to one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WireError {
    /// See [`RemoteError::NotFound`].
    NotFound {
        /// The missing node.
        node: String,
    },
    /// See [`RemoteError::AlreadyExists`].
    AlreadyExists {
        /// The name that is taken.
        name: String,
    },
    /// See [`RemoteError::NotAFolder`].
    NotAFolder {
        /// The node that is a file.
        node: String,
    },
    /// See [`RemoteError::NotAFile`].
    NotAFile {
        /// The node that is a folder.
        node: String,
    },
    /// See [`RemoteError::InvalidRange`].
    InvalidRange {
        /// Requested start.
        offset: u64,
        /// Actual file size.
        size: u64,
    },
    /// See [`RemoteError::InvalidMove`].
    InvalidMove {
        /// Explanation.
        message: String,
    },
    /// See [`RemoteError::Unauthorized`].
    Unauthorized,
    /// See [`RemoteError::Transfer`].
    Transfer {
        /// Explanation.
        message: String,
    },
    /// See [`RemoteError::Rejected`].
    Rejected {
        /// Explanation.
        message: String,
    },
    /// See [`RemoteError::Unavailable`].
    Unavailable {
        /// Explanation.
        message: String,
    },
}

impl WireError {
    /// Converts into the port's error. A name the domain rejects is reported
    /// as [`RemoteError::Rejected`] because it cannot come from protonsync.
    #[must_use]
    pub fn into_remote_error(self) -> RemoteError {
        match self {
            Self::NotFound { node } => RemoteError::NotFound(NodeId::new(node)),
            Self::AlreadyExists { name } => match NodeName::new(name.clone()) {
                Ok(valid) => RemoteError::AlreadyExists(valid),
                Err(_) => RemoteError::Rejected(format!("{name} already exists")),
            },
            Self::NotAFolder { node } => RemoteError::NotAFolder(NodeId::new(node)),
            Self::NotAFile { node } => RemoteError::NotAFile(NodeId::new(node)),
            Self::InvalidRange { offset, size } => RemoteError::InvalidRange { offset, size },
            Self::InvalidMove { message } => RemoteError::InvalidMove(message),
            Self::Unauthorized => RemoteError::Unauthorized,
            Self::Transfer { message } => RemoteError::Transfer(message),
            Self::Rejected { message } => RemoteError::Rejected(message),
            Self::Unavailable { message } => RemoteError::Unavailable(message),
        }
    }
}

impl From<&RemoteError> for WireError {
    fn from(error: &RemoteError) -> Self {
        match error {
            RemoteError::NotFound(node) => Self::NotFound {
                node: node.to_string(),
            },
            RemoteError::AlreadyExists(name) => Self::AlreadyExists {
                name: name.to_string(),
            },
            RemoteError::NotAFolder(node) => Self::NotAFolder {
                node: node.to_string(),
            },
            RemoteError::NotAFile(node) => Self::NotAFile {
                node: node.to_string(),
            },
            RemoteError::InvalidRange { offset, size } => Self::InvalidRange {
                offset: *offset,
                size: *size,
            },
            RemoteError::InvalidMove(message) => Self::InvalidMove {
                message: message.clone(),
            },
            RemoteError::Unauthorized => Self::Unauthorized,
            RemoteError::Transfer(message) => Self::Transfer {
                message: message.clone(),
            },
            RemoteError::Rejected(message) => Self::Rejected {
                message: message.clone(),
            },
            RemoteError::Unavailable(message) => Self::Unavailable {
                message: message.clone(),
            },
        }
    }
}

/// A node as the helper describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireNode {
    /// Node id (the SDK's node UID).
    pub id: String,
    /// Parent folder; missing only for the root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Decrypted name.
    pub name: String,
    /// Folder or file.
    #[serde(flatten)]
    pub kind: WireKind,
}

/// Kind of a [`WireNode`]; on the wire the field `type` tells which one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireKind {
    /// A folder.
    Folder,
    /// A file with its active revision.
    File {
        /// Id of the active revision.
        revision: String,
        /// Size of the plain content in bytes.
        size: u64,
        /// Modification time in Unix seconds.
        modified: i64,
        /// SHA-1 of the plain content as lowercase hex, if known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha1: Option<String>,
    },
}

impl WireNode {
    /// Converts into a domain node.
    ///
    /// # Errors
    /// [`RemoteError::Rejected`] if the name is not valid in the domain.
    pub fn into_node(self) -> Result<Node, RemoteError> {
        let name = NodeName::new(self.name).map_err(|e| RemoteError::Rejected(e.to_string()))?;
        let kind = match self.kind {
            WireKind::Folder => NodeKind::Folder,
            WireKind::File {
                revision,
                size,
                modified,
                sha1,
            } => NodeKind::File {
                revision: RevisionId::new(revision),
                fingerprint: Fingerprint {
                    size,
                    modified: Timestamp::from_unix_seconds(modified),
                    content_hash: sha1,
                },
            },
        };
        Ok(Node {
            id: NodeId::new(self.id),
            parent: self.parent.map(NodeId::new),
            name,
            kind,
        })
    }
}

impl From<&Node> for WireNode {
    fn from(node: &Node) -> Self {
        let kind = match &node.kind {
            NodeKind::Folder => WireKind::Folder,
            NodeKind::File {
                revision,
                fingerprint,
            } => WireKind::File {
                revision: revision.as_str().to_owned(),
                size: fingerprint.size,
                modified: fingerprint.modified.unix_seconds(),
                sha1: fingerprint.content_hash.clone(),
            },
        };
        Self {
            id: node.id.to_string(),
            parent: node.parent.as_ref().map(ToString::to_string),
            name: node.name.to_string(),
            kind,
        }
    }
}

/// A remote change on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireEvent {
    /// See [`RemoteEvent::Created`].
    Created {
        /// The node.
        node: String,
    },
    /// See [`RemoteEvent::Updated`].
    Updated {
        /// The node.
        node: String,
    },
    /// See [`RemoteEvent::Trashed`].
    Trashed {
        /// The node.
        node: String,
    },
    /// See [`RemoteEvent::Gap`].
    Gap,
}

impl From<WireEvent> for RemoteEvent {
    fn from(event: WireEvent) -> Self {
        match event {
            WireEvent::Created { node } => Self::Created(NodeId::new(node)),
            WireEvent::Updated { node } => Self::Updated(NodeId::new(node)),
            WireEvent::Trashed { node } => Self::Trashed(NodeId::new(node)),
            WireEvent::Gap => Self::Gap,
        }
    }
}

impl From<&RemoteEvent> for WireEvent {
    fn from(event: &RemoteEvent) -> Self {
        match event {
            RemoteEvent::Created(node) => Self::Created {
                node: node.to_string(),
            },
            RemoteEvent::Updated(node) => Self::Updated {
                node: node.to_string(),
            },
            RemoteEvent::Trashed(node) => Self::Trashed {
                node: node.to_string(),
            },
            RemoteEvent::Gap => Self::Gap,
        }
    }
}

/// Answer to `events_since`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireEvents {
    /// Changes after the given cursor, oldest first.
    pub events: Vec<WireEvent>,
    /// Cursor for the next call.
    pub cursor: String,
}

impl WireEvents {
    /// Converts into the port's types.
    #[must_use]
    pub fn into_port(self) -> (Vec<RemoteEvent>, EventCursor) {
        (
            self.events.into_iter().map(RemoteEvent::from).collect(),
            EventCursor::new(self.cursor),
        )
    }
}

/// Answer to `hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// Protocol version the helper speaks.
    pub protocol: u32,
    /// Version of the official SDK inside the helper (for logs).
    pub sdk_version: String,
}

/// Answer to `upload_open` and `revision_open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadHandle {
    /// Handle for `upload_write`, `upload_commit` and `upload_abort`.
    pub upload: String,
}

/// Answer to `download_open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadHandle {
    /// Handle for `download_read` and `download_close`.
    pub download: String,
}

/// A piece of file content (`download_read`, `download_range`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    /// The bytes as base64.
    pub data: String,
    /// `true` if the end of the file is reached (only `download_read`).
    #[serde(default)]
    pub eof: bool,
}

/// Encodes bytes for the wire.
#[must_use]
pub fn encode(bytes: &[u8]) -> String {
    BASE64.encode(bytes)
}

/// Decodes bytes from the wire.
///
/// # Errors
/// [`RemoteError::Transfer`] if the text is not valid base64.
pub fn decode(text: &str) -> Result<Vec<u8>, RemoteError> {
    BASE64
        .decode(text)
        .map_err(|e| RemoteError::Transfer(format!("invalid base64 from helper: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn file_nodes_roundtrip_through_json() {
        let wire: WireNode = serde_json::from_value(json!({
            "id": "n1", "parent": "root", "name": "a.txt",
            "type": "file", "revision": "r1", "size": 3, "modified": 10, "sha1": "abc"
        }))
        .unwrap();
        let node = wire.clone().into_node().unwrap();
        assert_eq!(node.name.as_str(), "a.txt");
        assert_eq!(node.fingerprint().unwrap().size, 3);
        assert_eq!(WireNode::from(&node), wire);
    }

    #[test]
    fn folder_without_parent_is_the_root() {
        let wire: WireNode =
            serde_json::from_value(json!({"id": "r", "name": "x", "type": "folder"})).unwrap();
        let node = wire.into_node().unwrap();
        assert!(node.is_folder());
        assert_eq!(node.parent, None);
    }

    #[test]
    fn invalid_names_are_rejected() {
        let wire = WireNode {
            id: "n".into(),
            parent: None,
            name: "a/b".into(),
            kind: WireKind::Folder,
        };
        assert!(matches!(wire.into_node(), Err(RemoteError::Rejected(_))));
    }

    #[test]
    fn every_remote_error_survives_the_wire() {
        let errors = [
            RemoteError::NotFound(NodeId::new("x")),
            RemoteError::AlreadyExists(NodeName::new("a").unwrap()),
            RemoteError::NotAFolder(NodeId::new("x")),
            RemoteError::NotAFile(NodeId::new("x")),
            RemoteError::InvalidRange { offset: 5, size: 2 },
            RemoteError::InvalidMove("loop".into()),
            RemoteError::Unauthorized,
            RemoteError::Transfer("t".into()),
            RemoteError::Rejected("r".into()),
            RemoteError::Unavailable("u".into()),
        ];
        for error in errors {
            let json = serde_json::to_string(&WireError::from(&error)).unwrap();
            let back: WireError = serde_json::from_str(&json).unwrap();
            assert_eq!(back.into_remote_error(), error);
        }
    }

    #[test]
    fn invalid_existing_name_becomes_rejected() {
        let error = WireError::AlreadyExists { name: "..".into() };
        assert!(matches!(
            error.into_remote_error(),
            RemoteError::Rejected(_)
        ));
    }

    #[test]
    fn events_keep_their_order() {
        let wire: WireEvents = serde_json::from_value(json!({
            "events": [{"type": "created", "node": "a"}, {"type": "gap"},
                       {"type": "updated", "node": "b"}, {"type": "trashed", "node": "c"}],
            "cursor": "42"
        }))
        .unwrap();
        let (events, cursor) = wire.into_port();
        assert_eq!(cursor.as_str(), "42");
        let back: Vec<WireEvent> = events.iter().map(WireEvent::from).collect();
        assert_eq!(back.len(), 4);
        assert_eq!(events[1], RemoteEvent::Gap);
    }

    #[test]
    fn base64_roundtrip_and_garbage() {
        assert_eq!(decode(&encode(b"hello")).unwrap(), b"hello");
        assert!(matches!(decode("%%%"), Err(RemoteError::Transfer(_))));
    }

    #[test]
    fn responses_carry_either_ok_or_error() {
        let ok = serde_json::to_value(Response::success(1, json!(null))).unwrap();
        assert_eq!(ok, json!({"id": 1, "ok": null}));
        let parsed: Response =
            serde_json::from_str(r#"{"id":2,"error":{"kind":"unauthorized"}}"#).unwrap();
        assert_eq!(parsed.error, Some(WireError::Unauthorized));
    }
}
