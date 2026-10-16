//! Port to the remote storage (Proton Drive or a fake for tests).
//!
//! The methods are asynchronous. Instead of `async fn`, this spells out
//! `fn ... -> impl Future<Output = ...> + Send`: it means the same, but also
//! requires that the result may move between threads (`Send`).
//! The daemon, which works with several threads, needs that.
//!
//! All implementations must pass the contract tests in `protonsync-testkit`.
//! The expected behavior is specified there exactly.

use std::future::Future;
use std::io::{Read, Write};

use protonsync_domain::node::{Node, NodeId, NodeName};
use protonsync_domain::time::Timestamp;
use thiserror::Error;

/// Error when accessing the remote storage.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RemoteError {
    /// Node does not exist (or is in the trash).
    #[error("node {0} not found")]
    NotFound(NodeId),
    /// The target folder already contains a node with this name.
    #[error("{0} already exists in the target folder")]
    AlreadyExists(NodeName),
    /// A folder was expected.
    #[error("{0} is not a folder")]
    NotAFolder(NodeId),
    /// A file was expected.
    #[error("{0} is not a file")]
    NotAFile(NodeId),
    /// The requested range starts beyond the end of the file.
    #[error("range from byte {offset} lies beyond the end of the file ({size} bytes)")]
    InvalidRange {
        /// Requested starting point.
        offset: u64,
        /// Actual file size.
        size: u64,
    },
    /// Moving is not possible like this (e.g. a folder into itself).
    #[error("invalid move: {0}")]
    InvalidMove(String),
    /// Login missing or expired.
    #[error("not logged in or session expired")]
    Unauthorized,
    /// Reading the source or writing the target failed.
    #[error("transfer failed: {0}")]
    Transfer(String),
    /// Proton rejected the request permanently (retrying does not help).
    #[error("rejected by Proton: {0}")]
    Rejected(String),
    /// Temporary error (network, rate limiting); try again later.
    #[error("temporarily unavailable: {0}")]
    Unavailable(String),
}

/// Position in the event stream of the remote storage.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventCursor(String);

impl EventCursor {
    /// Creates a cursor from the storage's value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The cursor as text (for storing in the state).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A remote change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteEvent {
    /// Node was created.
    Created(NodeId),
    /// Content or metadata (name, location) changed.
    Updated(NodeId),
    /// Node was moved to the trash.
    Trashed(NodeId),
    /// Gap in the event stream (e.g. cursor too old): which nodes
    /// changed is unknown, the tree must be read again completely.
    Gap,
}

/// Abstract remote storage.
pub trait RemoteStorage: Send + Sync {
    /// The root folder of the storage.
    fn root_folder(&self) -> impl Future<Output = Result<Node, RemoteError>> + Send;

    /// Reads the metadata of a node.
    fn node(&self, id: &NodeId) -> impl Future<Output = Result<Node, RemoteError>> + Send;

    /// Lists the children of a folder (without the trash), sorted by name.
    fn list_children(
        &self,
        folder: &NodeId,
    ) -> impl Future<Output = Result<Vec<Node>, RemoteError>> + Send;

    /// Creates a folder.
    fn create_folder(
        &self,
        parent: &NodeId,
        name: &NodeName,
    ) -> impl Future<Output = Result<Node, RemoteError>> + Send;

    /// Uploads a new file. If the name already exists: [`RemoteError::AlreadyExists`].
    ///
    /// The content is read from `content` (e.g. an open file), so that
    /// even large files do not have to fit into memory completely. `size` is the
    /// expected size in bytes.
    fn upload_file<R: Read + Send>(
        &self,
        parent: &NodeId,
        name: &NodeName,
        content: R,
        size: u64,
        modified: Timestamp,
    ) -> impl Future<Output = Result<Node, RemoteError>> + Send;

    /// Uploads a new revision of an existing file.
    fn upload_revision<R: Read + Send>(
        &self,
        file: &NodeId,
        content: R,
        size: u64,
        modified: Timestamp,
    ) -> impl Future<Output = Result<Node, RemoteError>> + Send;

    /// Writes the whole content of the current revision to `target` and
    /// returns the number of bytes written.
    fn download_to<W: Write + Send>(
        &self,
        file: &NodeId,
        target: &mut W,
    ) -> impl Future<Output = Result<u64, RemoteError>> + Send;

    /// Downloads part of the content (for the FUSE mount). At the end of the file
    /// correspondingly fewer bytes are returned; a start beyond the end is an error.
    fn download_range(
        &self,
        file: &NodeId,
        offset: u64,
        length: u64,
    ) -> impl Future<Output = Result<Vec<u8>, RemoteError>> + Send;

    /// Renames a node.
    fn rename(
        &self,
        id: &NodeId,
        new_name: &NodeName,
    ) -> impl Future<Output = Result<Node, RemoteError>> + Send;

    /// Moves a node into another folder.
    fn move_node(
        &self,
        id: &NodeId,
        new_parent: &NodeId,
    ) -> impl Future<Output = Result<Node, RemoteError>> + Send;

    /// Moves a node (including its content) to the remote trash.
    fn trash(&self, id: &NodeId) -> impl Future<Output = Result<(), RemoteError>> + Send;

    /// Brings a node back from the remote trash.
    fn restore(&self, id: &NodeId) -> impl Future<Output = Result<Node, RemoteError>> + Send;

    /// Permanently deletes a node from the remote trash.
    /// Only allowed for nodes that are in the trash.
    fn delete_permanently(
        &self,
        id: &NodeId,
    ) -> impl Future<Output = Result<(), RemoteError>> + Send;

    /// Returns the events after `cursor` and the new cursor.
    ///
    /// With `None` there are no events, only the current cursor as a starting point.
    fn events_since(
        &self,
        cursor: Option<&EventCursor>,
    ) -> impl Future<Output = Result<(Vec<RemoteEvent>, EventCursor), RemoteError>> + Send;
}

/// The part of a storage that the trash needs: restoring and
/// permanent deletion. Every [`RemoteStorage`] is one; the photo library
/// becomes one via [`super::photo_library::PhotoBin`].
pub trait RemoteBin: Send + Sync {
    /// Brings a node back from the remote trash.
    fn restore_from_trash(
        &self,
        id: &NodeId,
    ) -> impl Future<Output = Result<(), RemoteError>> + Send;

    /// Permanently deletes a node from the remote trash.
    fn delete_from_trash(
        &self,
        id: &NodeId,
    ) -> impl Future<Output = Result<(), RemoteError>> + Send;
}

impl<T: RemoteStorage> RemoteBin for T {
    async fn restore_from_trash(&self, id: &NodeId) -> Result<(), RemoteError> {
        self.restore(id).await.map(drop)
    }

    fn delete_from_trash(
        &self,
        id: &NodeId,
    ) -> impl Future<Output = Result<(), RemoteError>> + Send {
        self.delete_permanently(id)
    }
}

/// A shared storage (`Arc`) is itself a storage; each method only
/// forwards. This way several owners can use the same storage.
impl<T: RemoteStorage + ?Sized> RemoteStorage for std::sync::Arc<T> {
    fn root_folder(&self) -> impl Future<Output = Result<Node, RemoteError>> + Send {
        (**self).root_folder()
    }

    fn node(&self, id: &NodeId) -> impl Future<Output = Result<Node, RemoteError>> + Send {
        (**self).node(id)
    }

    fn list_children(
        &self,
        folder: &NodeId,
    ) -> impl Future<Output = Result<Vec<Node>, RemoteError>> + Send {
        (**self).list_children(folder)
    }

    fn create_folder(
        &self,
        parent: &NodeId,
        name: &NodeName,
    ) -> impl Future<Output = Result<Node, RemoteError>> + Send {
        (**self).create_folder(parent, name)
    }

    fn upload_file<R: Read + Send>(
        &self,
        parent: &NodeId,
        name: &NodeName,
        content: R,
        size: u64,
        modified: Timestamp,
    ) -> impl Future<Output = Result<Node, RemoteError>> + Send {
        (**self).upload_file(parent, name, content, size, modified)
    }

    fn upload_revision<R: Read + Send>(
        &self,
        file: &NodeId,
        content: R,
        size: u64,
        modified: Timestamp,
    ) -> impl Future<Output = Result<Node, RemoteError>> + Send {
        (**self).upload_revision(file, content, size, modified)
    }

    fn download_to<W: Write + Send>(
        &self,
        file: &NodeId,
        target: &mut W,
    ) -> impl Future<Output = Result<u64, RemoteError>> + Send {
        (**self).download_to(file, target)
    }

    fn download_range(
        &self,
        file: &NodeId,
        offset: u64,
        length: u64,
    ) -> impl Future<Output = Result<Vec<u8>, RemoteError>> + Send {
        (**self).download_range(file, offset, length)
    }

    fn rename(
        &self,
        id: &NodeId,
        new_name: &NodeName,
    ) -> impl Future<Output = Result<Node, RemoteError>> + Send {
        (**self).rename(id, new_name)
    }

    fn move_node(
        &self,
        id: &NodeId,
        new_parent: &NodeId,
    ) -> impl Future<Output = Result<Node, RemoteError>> + Send {
        (**self).move_node(id, new_parent)
    }

    fn trash(&self, id: &NodeId) -> impl Future<Output = Result<(), RemoteError>> + Send {
        (**self).trash(id)
    }

    fn restore(&self, id: &NodeId) -> impl Future<Output = Result<Node, RemoteError>> + Send {
        (**self).restore(id)
    }

    fn delete_permanently(
        &self,
        id: &NodeId,
    ) -> impl Future<Output = Result<(), RemoteError>> + Send {
        (**self).delete_permanently(id)
    }

    fn events_since(
        &self,
        cursor: Option<&EventCursor>,
    ) -> impl Future<Output = Result<(Vec<RemoteEvent>, EventCursor), RemoteError>> + Send {
        (**self).events_since(cursor)
    }
}
