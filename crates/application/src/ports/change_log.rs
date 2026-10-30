//! Port for the change log: every change a sync made, kept for a number of days.

use protonsync_domain::change_log::ChangeEntry;
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::time::Timestamp;

use super::StateError;

/// A stored change together with the sync pair it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggedChange {
    /// The sync pair that made the change.
    pub pair: SyncPairId,
    /// The change itself.
    pub change: ChangeEntry,
}

/// Which changes to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeQuery {
    /// Only changes at or after this point in time.
    pub since: Timestamp,
    /// Only changes of this sync pair; `None` for all.
    pub pair: Option<SyncPairId>,
    /// At most this many changes (the newest).
    pub limit: usize,
}

/// Change log (separate from [`super::StateRepo`]: the sync engine only reports
/// changes, the daemon stores and reads them).
pub trait ChangeLog: Send + Sync {
    /// Appends the changes of one run of `pair`, all or none.
    ///
    /// # Errors
    /// [`StateError`] if the store is unreachable.
    fn record_changes(&self, pair: &SyncPairId, changes: &[ChangeEntry]) -> Result<(), StateError>;

    /// The changes matching `query`, newest first.
    ///
    /// # Errors
    /// [`StateError`] if the store is unreachable or damaged.
    fn changes(&self, query: &ChangeQuery) -> Result<Vec<LoggedChange>, StateError>;

    /// Removes all changes older than `before`; returns how many were removed.
    ///
    /// # Errors
    /// [`StateError`] if the store is unreachable.
    fn prune_changes(&self, before: Timestamp) -> Result<usize, StateError>;
}

/// A shared log (`Arc`) is itself a log.
impl<T: ChangeLog + ?Sized> ChangeLog for std::sync::Arc<T> {
    fn record_changes(&self, pair: &SyncPairId, changes: &[ChangeEntry]) -> Result<(), StateError> {
        (**self).record_changes(pair, changes)
    }

    fn changes(&self, query: &ChangeQuery) -> Result<Vec<LoggedChange>, StateError> {
        (**self).changes(query)
    }

    fn prune_changes(&self, before: Timestamp) -> Result<usize, StateError> {
        (**self).prune_changes(before)
    }
}
