//! Port to the stored sync state (base, event cursor, conflicts, trash).
//!
//! The base is the last state per path that matched on both sides.
//! It is only written after an operation has completed successfully;
//! if a run aborts, the next run detects the open changes again.

use protonsync_domain::conflict::Conflict;
use protonsync_domain::node::{Fingerprint, NodeId, RevisionId};
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::trash::TrashEntry;
use thiserror::Error;

use super::remote_storage::EventCursor;

/// Errors of the state store.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StateError {
    /// Store not readable or writable.
    #[error("state store unavailable: {0}")]
    Unavailable(String),
    /// Stored data is invalid.
    #[error("state corrupted: {0}")]
    Corrupt(String),
}

/// Common state of a path after the last successful sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseEntry {
    /// Path relative to the sync pair.
    pub path: RelativePath,
    /// Corresponding node in Proton.
    pub remote_id: NodeId,
    /// Revision of the remote file; `None` for folders.
    pub remote_revision: Option<RevisionId>,
    /// Fingerprint of the local file; `None` for folders.
    pub local: Option<Fingerprint>,
}

impl BaseEntry {
    /// `true` if the entry is a folder.
    #[must_use]
    pub fn is_folder(&self) -> bool {
        self.remote_revision.is_none()
    }
}

/// Identifier of a conflict in the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConflictId(pub i64);

/// Identifier of a trash entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TrashId(pub i64);

/// Stored sync state.
pub trait StateRepo: Send + Sync {
    /// All base entries of a sync pair, sorted by path.
    ///
    /// # Errors
    /// [`StateError`].
    fn base_entries(&self, pair: &SyncPairId) -> Result<Vec<BaseEntry>, StateError>;

    /// The base entry for exactly `path`, if there is one. The default searches
    /// in [`Self::base_entries`]; stores with an index should be able to do it faster.
    ///
    /// # Errors
    /// [`StateError`].
    fn base_entry(
        &self,
        pair: &SyncPairId,
        path: &RelativePath,
    ) -> Result<Option<BaseEntry>, StateError> {
        Ok(self
            .base_entries(pair)?
            .into_iter()
            .find(|entry| &entry.path == path))
    }

    /// Creates a base entry or replaces the one for the same path.
    ///
    /// # Errors
    /// [`StateError`].
    fn save_base(&self, pair: &SyncPairId, entry: &BaseEntry) -> Result<(), StateError>;

    /// Removes the base entry for `path` **and all below it**.
    ///
    /// # Errors
    /// [`StateError`].
    fn remove_base_tree(&self, pair: &SyncPairId, path: &RelativePath) -> Result<(), StateError>;

    /// Last processed event cursor.
    ///
    /// # Errors
    /// [`StateError`].
    fn event_cursor(&self, pair: &SyncPairId) -> Result<Option<EventCursor>, StateError>;

    /// Stores the event cursor.
    ///
    /// # Errors
    /// [`StateError`].
    fn save_event_cursor(&self, pair: &SyncPairId, cursor: &EventCursor) -> Result<(), StateError>;

    /// Puts a conflict into the queue.
    ///
    /// # Errors
    /// [`StateError`].
    fn add_conflict(
        &self,
        pair: &SyncPairId,
        conflict: &Conflict,
    ) -> Result<ConflictId, StateError>;

    /// Open conflicts of a sync pair, oldest first.
    ///
    /// # Errors
    /// [`StateError`].
    fn conflicts(&self, pair: &SyncPairId) -> Result<Vec<(ConflictId, Conflict)>, StateError>;

    /// Removes a resolved conflict. Unknown identifiers are not an error.
    ///
    /// # Errors
    /// [`StateError`].
    fn remove_conflict(&self, id: ConflictId) -> Result<(), StateError>;

    /// Records an entry in the trash.
    ///
    /// # Errors
    /// [`StateError`].
    fn add_trash(&self, pair: &SyncPairId, entry: &TrashEntry) -> Result<TrashId, StateError>;

    /// Trash entries of a sync pair, oldest first.
    ///
    /// # Errors
    /// [`StateError`].
    fn trash_entries(&self, pair: &SyncPairId) -> Result<Vec<(TrashId, TrashEntry)>, StateError>;

    /// Removes a trash entry. Unknown identifiers are not an error.
    ///
    /// # Errors
    /// [`StateError`].
    fn remove_trash(&self, id: TrashId) -> Result<(), StateError>;
}

/// A shared state (`Arc`) is itself a state store; each method only forwards.
impl<T: StateRepo + ?Sized> StateRepo for std::sync::Arc<T> {
    fn base_entries(&self, pair: &SyncPairId) -> Result<Vec<BaseEntry>, StateError> {
        (**self).base_entries(pair)
    }

    fn base_entry(
        &self,
        pair: &SyncPairId,
        path: &RelativePath,
    ) -> Result<Option<BaseEntry>, StateError> {
        (**self).base_entry(pair, path)
    }

    fn save_base(&self, pair: &SyncPairId, entry: &BaseEntry) -> Result<(), StateError> {
        (**self).save_base(pair, entry)
    }

    fn remove_base_tree(&self, pair: &SyncPairId, path: &RelativePath) -> Result<(), StateError> {
        (**self).remove_base_tree(pair, path)
    }

    fn event_cursor(&self, pair: &SyncPairId) -> Result<Option<EventCursor>, StateError> {
        (**self).event_cursor(pair)
    }

    fn save_event_cursor(&self, pair: &SyncPairId, cursor: &EventCursor) -> Result<(), StateError> {
        (**self).save_event_cursor(pair, cursor)
    }

    fn add_conflict(
        &self,
        pair: &SyncPairId,
        conflict: &Conflict,
    ) -> Result<ConflictId, StateError> {
        (**self).add_conflict(pair, conflict)
    }

    fn conflicts(&self, pair: &SyncPairId) -> Result<Vec<(ConflictId, Conflict)>, StateError> {
        (**self).conflicts(pair)
    }

    fn remove_conflict(&self, id: ConflictId) -> Result<(), StateError> {
        (**self).remove_conflict(id)
    }

    fn add_trash(&self, pair: &SyncPairId, entry: &TrashEntry) -> Result<TrashId, StateError> {
        (**self).add_trash(pair, entry)
    }

    fn trash_entries(&self, pair: &SyncPairId) -> Result<Vec<(TrashId, TrashEntry)>, StateError> {
        (**self).trash_entries(pair)
    }

    fn remove_trash(&self, id: TrashId) -> Result<(), StateError> {
        (**self).remove_trash(id)
    }
}
