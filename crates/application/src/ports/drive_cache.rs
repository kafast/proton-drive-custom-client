//! Ports for the virtual drive: contents in the cache and their index.

use std::io::Read;

use protonsync_domain::cache::BlockMap;
use protonsync_domain::node::{NodeId, RevisionId};
use protonsync_domain::path::RelativePath;
use protonsync_domain::pin::{PinRules, PinState};
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::time::Timestamp;

use super::{LocalError, StateError};

/// Identifier of a file in the cache (assigned by the drive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CacheKey(pub u64);

/// Contents of the cache: one (sparse) file per [`CacheKey`].
///
/// Synchronous like [`super::LocalStorage`]: local access is fast.
pub trait ContentCache: Send + Sync {
    /// Readable content (for the upload).
    type Reader: Read + Send;

    /// Reads up to `length` bytes from `offset`. Beyond the end
    /// correspondingly fewer bytes come; a missing file counts as empty.
    ///
    /// # Errors
    /// [`LocalError::Io`].
    fn read(&self, key: CacheKey, offset: u64, length: usize) -> Result<Vec<u8>, LocalError>;

    /// Writes `data` at `offset`; the file is created and extended if needed.
    ///
    /// # Errors
    /// [`LocalError::Io`], e.g. when the disk is full.
    fn write(&self, key: CacheKey, offset: u64, data: &[u8]) -> Result<(), LocalError>;

    /// Sets the length (truncating, or extending with zeros).
    ///
    /// # Errors
    /// [`LocalError::Io`].
    fn set_len(&self, key: CacheKey, size: u64) -> Result<(), LocalError>;

    /// Opens the content for reading from the start.
    ///
    /// # Errors
    /// [`LocalError::NotFound`] or [`LocalError::Io`].
    fn open(&self, key: CacheKey) -> Result<Self::Reader, LocalError>;

    /// Removes the content. If it is already gone, that is not an error.
    ///
    /// # Errors
    /// [`LocalError::Io`].
    fn remove(&self, key: CacheKey) -> Result<(), LocalError>;
}

/// Stored state of a file in the cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheRecord {
    /// Identifier of the content.
    pub key: CacheKey,
    /// Node in Proton; `None` for a locally created file not uploaded yet.
    pub node: Option<NodeId>,
    /// Path relative to the root of the drive (for pin rules and for
    /// resuming an upload after a restart).
    pub path: RelativePath,
    /// Revision in Proton the content is based on; `None` for new files.
    pub revision: Option<RevisionId>,
    /// Present blocks (includes the file size).
    pub blocks: BlockMap,
    /// Last access (for freeing).
    pub last_access: Timestamp,
    /// Modification time of the content.
    pub modified: Timestamp,
    /// `true` as long as local changes are not in Proton.
    pub changed: bool,
}

/// Cache index and pin rules per drive (in SQLite).
pub trait DriveRepo: Send + Sync {
    /// All entries of a drive.
    ///
    /// # Errors
    /// [`StateError`].
    fn cache_records(&self, drive: &SyncPairId) -> Result<Vec<CacheRecord>, StateError>;

    /// Creates an entry or replaces the one with the same key.
    ///
    /// # Errors
    /// [`StateError`].
    fn save_cache_record(&self, drive: &SyncPairId, record: &CacheRecord)
    -> Result<(), StateError>;

    /// Removes an entry. Unknown keys are not an error.
    ///
    /// # Errors
    /// [`StateError`].
    fn remove_cache_record(&self, drive: &SyncPairId, key: CacheKey) -> Result<(), StateError>;

    /// The pin rules of a drive.
    ///
    /// # Errors
    /// [`StateError`].
    fn pin_rules(&self, drive: &SyncPairId) -> Result<PinRules, StateError>;

    /// Sets (`Some`) or removes (`None`) the rule for exactly `path`.
    ///
    /// # Errors
    /// [`StateError`].
    fn save_pin_rule(
        &self,
        drive: &SyncPairId,
        path: &RelativePath,
        state: Option<PinState>,
    ) -> Result<(), StateError>;
}

/// A shared index (`Arc`) is itself an index.
impl<T: DriveRepo + ?Sized> DriveRepo for std::sync::Arc<T> {
    fn cache_records(&self, drive: &SyncPairId) -> Result<Vec<CacheRecord>, StateError> {
        (**self).cache_records(drive)
    }

    fn save_cache_record(
        &self,
        drive: &SyncPairId,
        record: &CacheRecord,
    ) -> Result<(), StateError> {
        (**self).save_cache_record(drive, record)
    }

    fn remove_cache_record(&self, drive: &SyncPairId, key: CacheKey) -> Result<(), StateError> {
        (**self).remove_cache_record(drive, key)
    }

    fn pin_rules(&self, drive: &SyncPairId) -> Result<PinRules, StateError> {
        (**self).pin_rules(drive)
    }

    fn save_pin_rule(
        &self,
        drive: &SyncPairId,
        path: &RelativePath,
        state: Option<PinState>,
    ) -> Result<(), StateError> {
        (**self).save_pin_rule(drive, path, state)
    }
}

/// A shared cache (`Arc`) is itself a cache.
impl<T: ContentCache + ?Sized> ContentCache for std::sync::Arc<T> {
    type Reader = T::Reader;

    fn read(&self, key: CacheKey, offset: u64, length: usize) -> Result<Vec<u8>, LocalError> {
        (**self).read(key, offset, length)
    }

    fn write(&self, key: CacheKey, offset: u64, data: &[u8]) -> Result<(), LocalError> {
        (**self).write(key, offset, data)
    }

    fn set_len(&self, key: CacheKey, size: u64) -> Result<(), LocalError> {
        (**self).set_len(key, size)
    }

    fn open(&self, key: CacheKey) -> Result<Self::Reader, LocalError> {
        (**self).open(key)
    }

    fn remove(&self, key: CacheKey) -> Result<(), LocalError> {
        (**self).remove(key)
    }
}
