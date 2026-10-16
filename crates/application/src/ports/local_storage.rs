//! Port to the local file system of a sync pair.
//!
//! An instance stands for **one** root folder; all paths are relative to it.
//! The methods are synchronous: local file access is fast enough, and the
//! daemon can move them to a separate thread if needed.

use std::io::{Read, Write};

use protonsync_domain::node::Fingerprint;
use protonsync_domain::path::RelativePath;
use protonsync_domain::time::Timestamp;
use thiserror::Error;

/// Error when accessing the local file system.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LocalError {
    /// Path does not exist.
    #[error("{0} not found")]
    NotFound(RelativePath),
    /// Something is already at the target.
    #[error("{0} already exists")]
    AlreadyExists(RelativePath),
    /// A folder was expected or is in the way.
    #[error("{0} is not a folder")]
    NotAFolder(RelativePath),
    /// Other input/output error (permissions, full disk, …).
    #[error("file system error: {0}")]
    Io(String),
}

/// Kind of a local entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalKind {
    /// Folder.
    Folder,
    /// File with size and modification time (without checksum, which is expensive).
    File(Fingerprint),
}

/// A local entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalEntry {
    /// Path relative to the root folder.
    pub path: RelativePath,
    /// Folder or file.
    pub kind: LocalKind,
}

/// Local file system of a sync pair.
///
/// `type Reader` and `type Staging` are "associated types": each
/// implementation decides which concrete type comes back (e.g. a
/// `std::fs::File`), without the caller having to know it.
pub trait LocalStorage: Send + Sync {
    /// Readable content of a file.
    type Reader: Read + Send;
    /// A file that is not visible yet and is being written to.
    type Staging: Write + Send;

    /// All entries below the root folder (without the root itself and without
    /// own helper files), sorted by path.
    ///
    /// # Errors
    /// [`LocalError::Io`] on read errors.
    fn scan(&self) -> Result<Vec<LocalEntry>, LocalError>;

    /// A single entry; `Ok(None)` if it does not exist.
    ///
    /// # Errors
    /// [`LocalError::Io`] on read errors.
    fn entry(&self, path: &RelativePath) -> Result<Option<LocalEntry>, LocalError>;

    /// Opens a file for reading.
    ///
    /// # Errors
    /// [`LocalError::NotFound`] or [`LocalError::Io`].
    fn open(&self, path: &RelativePath) -> Result<Self::Reader, LocalError>;

    /// SHA-1 of the content as lowercase hex (as with Proton).
    ///
    /// # Errors
    /// [`LocalError::NotFound`] or [`LocalError::Io`].
    fn content_hash(&self, path: &RelativePath) -> Result<String, LocalError>;

    /// Starts writing a file. Only [`LocalStorage::commit`] makes it
    /// visible under `path`; this way a half-written file never appears.
    ///
    /// # Errors
    /// [`LocalError::NotFound`] if the parent folder is missing, otherwise [`LocalError::Io`].
    fn stage(&self, path: &RelativePath) -> Result<Self::Staging, LocalError>;

    /// Atomically replaces `path` with the written file and sets the
    /// modification time. Returns the new fingerprint.
    ///
    /// # Errors
    /// [`LocalError::Io`].
    fn commit(
        &self,
        staging: Self::Staging,
        path: &RelativePath,
        modified: Timestamp,
    ) -> Result<Fingerprint, LocalError>;

    /// Creates a folder (parent folders must exist). If it already exists, that is not an error.
    ///
    /// # Errors
    /// [`LocalError::NotFound`], [`LocalError::NotAFolder`] or [`LocalError::Io`].
    fn create_folder(&self, path: &RelativePath) -> Result<(), LocalError>;

    /// Renames or moves within the root folder.
    ///
    /// # Errors
    /// [`LocalError::NotFound`], [`LocalError::AlreadyExists`] or [`LocalError::Io`].
    fn rename(&self, from: &RelativePath, to: &RelativePath) -> Result<(), LocalError>;

    /// Moves a file or folder to the local trash and returns the
    /// key under which the content is stored there.
    ///
    /// # Errors
    /// [`LocalError::NotFound`] or [`LocalError::Io`].
    fn move_to_trash(&self, path: &RelativePath) -> Result<String, LocalError>;

    /// Brings an entry back from the trash to `path`.
    ///
    /// # Errors
    /// [`LocalError::NotFound`], [`LocalError::AlreadyExists`] or [`LocalError::Io`].
    fn restore_from_trash(&self, stored_as: &str, path: &RelativePath) -> Result<(), LocalError>;

    /// Permanently deletes an entry from the trash.
    ///
    /// # Errors
    /// [`LocalError::NotFound`] or [`LocalError::Io`].
    fn purge_trash(&self, stored_as: &str) -> Result<(), LocalError>;
}

/// A shared storage (`Arc`) is itself a storage; each method only forwards.
impl<T: LocalStorage + ?Sized> LocalStorage for std::sync::Arc<T> {
    type Reader = T::Reader;
    type Staging = T::Staging;

    fn scan(&self) -> Result<Vec<LocalEntry>, LocalError> {
        (**self).scan()
    }

    fn entry(&self, path: &RelativePath) -> Result<Option<LocalEntry>, LocalError> {
        (**self).entry(path)
    }

    fn open(&self, path: &RelativePath) -> Result<Self::Reader, LocalError> {
        (**self).open(path)
    }

    fn content_hash(&self, path: &RelativePath) -> Result<String, LocalError> {
        (**self).content_hash(path)
    }

    fn stage(&self, path: &RelativePath) -> Result<Self::Staging, LocalError> {
        (**self).stage(path)
    }

    fn commit(
        &self,
        staging: Self::Staging,
        path: &RelativePath,
        modified: Timestamp,
    ) -> Result<Fingerprint, LocalError> {
        (**self).commit(staging, path, modified)
    }

    fn create_folder(&self, path: &RelativePath) -> Result<(), LocalError> {
        (**self).create_folder(path)
    }

    fn rename(&self, from: &RelativePath, to: &RelativePath) -> Result<(), LocalError> {
        (**self).rename(from, to)
    }

    fn move_to_trash(&self, path: &RelativePath) -> Result<String, LocalError> {
        (**self).move_to_trash(path)
    }

    fn restore_from_trash(&self, stored_as: &str, path: &RelativePath) -> Result<(), LocalError> {
        (**self).restore_from_trash(stored_as, path)
    }

    fn purge_trash(&self, stored_as: &str) -> Result<(), LocalError> {
        (**self).purge_trash(stored_as)
    }
}
