//! Nodes (files and folders) and their versions.

use crate::DomainError;
use crate::time::Timestamp;

/// Maximum length of a name in bytes (limit of the common Linux file systems).
pub const MAX_NAME_BYTES: usize = 255;

/// Unique id of a node, assigned by the storage.
///
/// A "newtype": a struct with exactly one field. This way the compiler cannot mix up a
/// `NodeId` with any other `String`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(String);

impl NodeId {
    /// Creates an id from the value the storage provided.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A validated file or folder name (a single path segment).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeName(String);

impl NodeName {
    /// Validates and takes over a name.
    ///
    /// # Errors
    /// [`DomainError::InvalidNodeName`] if the name is empty, `.` or `..`,
    /// contains `/` or a null byte, or is longer than [`MAX_NAME_BYTES`].
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let name = value.into();
        let reason = if name.is_empty() {
            Some("name is empty")
        } else if name == "." || name == ".." {
            Some("`.` and `..` are reserved")
        } else if name.contains('/') {
            Some("name contains `/`")
        } else if name.contains('\0') {
            Some("name contains a null byte")
        } else if name.len() > MAX_NAME_BYTES {
            Some("name is longer than 255 bytes")
        } else {
            None
        };
        match reason {
            Some(reason) => Err(DomainError::InvalidNodeName { name, reason }),
            None => Ok(Self(name)),
        }
    }

    /// The name as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for NodeName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Id of a file revision, assigned by the storage.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RevisionId(String);

impl RevisionId {
    /// Creates a revision id.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Content of a file at one point in time: enough to detect changes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    /// Size in bytes.
    pub size: u64,
    /// Last modification according to the source.
    pub modified: Timestamp,
    /// SHA-1 of the content as lowercase hex, if known. SHA-1 because
    /// Proton stores exactly this checksum per revision.
    pub content_hash: Option<String>,
}

/// Kind of a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    /// Folder without content of its own.
    Folder,
    /// File with its current revision.
    File {
        /// Current revision.
        revision: RevisionId,
        /// Content characteristics of this revision.
        fingerprint: Fingerprint,
    },
}

/// A node in the remote or local tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Unique id.
    pub id: NodeId,
    /// Parent folder; `None` only for the root folder.
    pub parent: Option<NodeId>,
    /// Name in the parent folder.
    pub name: NodeName,
    /// Folder or file.
    pub kind: NodeKind,
}

impl Node {
    /// `true` if the node is a folder.
    #[must_use]
    pub fn is_folder(&self) -> bool {
        matches!(self.kind, NodeKind::Folder)
    }

    /// Content characteristics, only present for files.
    #[must_use]
    pub fn fingerprint(&self) -> Option<&Fingerprint> {
        match &self.kind {
            NodeKind::File { fingerprint, .. } => Some(fingerprint),
            NodeKind::Folder => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_names() {
        for name in ["report.pdf", "Photos 2026", "ümlaut", ".hidden", "a..b"] {
            assert!(NodeName::new(name).is_ok(), "{name} should be valid");
        }
    }

    #[test]
    fn rejects_invalid_names() {
        let too_long = "x".repeat(MAX_NAME_BYTES + 1);
        for name in ["", ".", "..", "a/b", "nul\0byte", too_long.as_str()] {
            assert!(
                matches!(
                    NodeName::new(name),
                    Err(DomainError::InvalidNodeName { .. })
                ),
                "{name:?} should be rejected"
            );
        }
    }

    #[test]
    fn accepts_name_of_exactly_max_length() {
        assert!(NodeName::new("x".repeat(MAX_NAME_BYTES)).is_ok());
    }

    #[test]
    fn only_files_have_a_fingerprint() {
        let folder = Node {
            id: NodeId::new("1"),
            parent: None,
            name: NodeName::new("root").unwrap(),
            kind: NodeKind::Folder,
        };
        assert!(folder.is_folder());
        assert!(folder.fingerprint().is_none());
    }
}
