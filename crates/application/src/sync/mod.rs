//! Sync engine: synchronizes a sync pair fully once.
//!
//! Steps of a run:
//! 1. Update the remote tree: via events or fully (`remote_index`).
//! 2. Snapshot of local, remote and base (`snapshot`).
//! 3. Plan per path via [`protonsync_domain::conflict::decide`] (`plan`).
//! 4. Execution, unless it is a dry run (`apply`).
//! 5. Store the event cursor so that the daemon only runs again on new events.

mod apply;
mod plan;
mod remote_index;
mod snapshot;

use protonsync_domain::conflict::SyncAction;
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncPair;
use thiserror::Error;

pub use plan::{PlannedStep, SkippedPath, SyncPlan};
pub use remote_index::{Refresh, RemoteCache};

use crate::ports::{
    Clock, LocalError, LocalStorage, RemoteError, RemoteStorage, StateError, StateRepo,
};

/// Error that prevents or aborts a whole run.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SyncError {
    /// Proton unreachable or not logged in.
    #[error(transparent)]
    Remote(#[from] RemoteError),
    /// Local folder not readable.
    #[error(transparent)]
    Local(#[from] LocalError),
    /// State store unavailable.
    #[error(transparent)]
    State(#[from] StateError),
}

/// A failed step; it is retried in the next run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepFailure {
    /// Affected path.
    pub path: RelativePath,
    /// Planned action.
    pub action: SyncAction,
    /// Error message.
    pub message: String,
}

/// Result of a run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Uploaded files and created remote folders.
    pub uploaded: usize,
    /// Downloaded files and created local folders.
    pub downloaded: usize,
    /// Moved to the Proton trash.
    pub trashed_remote: usize,
    /// Moved to the local trash.
    pub trashed_local: usize,
    /// Moved into the local version archive (option `archive_versions`).
    pub archived_local: usize,
    /// Moved into the version archive in Proton (option `archive_versions`).
    pub archived_remote: usize,
    /// New conflicts (both versions were kept).
    pub conflicts: usize,
    /// Changes not transferred because of the sync direction.
    pub ignored: usize,
    /// Skipped paths.
    pub skipped: Vec<SkippedPath>,
    /// Failed steps.
    pub failures: Vec<StepFailure>,
}

impl SyncReport {
    fn count(&mut self, action: SyncAction) {
        match action {
            SyncAction::Upload | SyncAction::ArchiveRemoteAndUpload => self.uploaded += 1,
            SyncAction::Download | SyncAction::ArchiveAndDownload => self.downloaded += 1,
            SyncAction::TrashRemote => self.trashed_remote += 1,
            SyncAction::TrashLocal => self.trashed_local += 1,
            SyncAction::ArchiveLocal => self.archived_local += 1,
            SyncAction::ArchiveRemote => self.archived_remote += 1,
            SyncAction::Conflict => self.conflicts += 1,
            SyncAction::IgnoreLocalChange | SyncAction::IgnoreRemoteChange => self.ignored += 1,
            SyncAction::Nothing | SyncAction::RecordConverged | SyncAction::KeepLocalCopy => {}
        }
    }
}

/// What a run delivered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// Dry run: only the plan, nothing was changed.
    Planned(SyncPlan),
    /// The plan was executed.
    Applied(SyncReport),
}

/// All ports a run needs, for exactly one sync pair.
///
/// The ports come as references (`&'a R`): the engine does not own them,
/// it only borrows them for the duration of the run (similar to dependency
/// injection via constructor in Java).
pub struct SyncContext<'a, R, L, S, C> {
    /// Remote storage.
    pub remote: &'a R,
    /// Local folder of the sync pair.
    pub local: &'a L,
    /// Stored state.
    pub state: &'a S,
    /// Clock.
    pub clock: &'a C,
    /// The sync pair.
    pub pair: &'a SyncPair,
}

impl<R, L, S, C> SyncContext<'_, R, L, S, C>
where
    R: RemoteStorage,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    /// Synchronizes the sync pair once, reading the remote tree
    /// fully for it. With `dry_run = true` it only plans.
    ///
    /// # Errors
    /// [`SyncError`] if a side cannot be read at all or the login is missing.
    /// Errors of individual files are in [`SyncReport::failures`] instead.
    pub async fn run_once(&self, dry_run: bool) -> Result<SyncOutcome, SyncError> {
        self.run(&mut RemoteCache::new(), dry_run).await
    }

    /// Like [`Self::run_once`], but with a cached remote tree:
    /// if `cache` is filled, only the Proton events since the last
    /// run are fetched instead of reading all folders.
    ///
    /// # Errors
    /// Like [`Self::run_once`]; after an error `cache` is empty.
    pub async fn run(
        &self,
        cache: &mut RemoteCache,
        dry_run: bool,
    ) -> Result<SyncOutcome, SyncError> {
        let refresh = cache.refresh(self.remote, &self.pair.remote_root).await?;
        let (Some(remote), Some(cursor)) = (cache.tree().cloned(), cache.cursor().cloned()) else {
            // `refresh` always fills the cache. Without a tree, rather abort
            // than treat it as empty and delete everything locally.
            return Err(SyncError::Remote(RemoteError::Unavailable(
                "remote tree missing".into(),
            )));
        };
        let mut snapshot = snapshot::Snapshot {
            remote,
            local: snapshot::local_tree(self.local)?,
            base: snapshot::base_tree(self.state, &self.pair.id)?,
        };
        if self.pair.archive_versions {
            snapshot.drop_archives();
        }
        let plan = plan::make_plan(&snapshot, self.local, self.pair);
        if dry_run {
            self.state.save_event_cursor(&self.pair.id, &cursor)?;
            return Ok(SyncOutcome::Planned(plan));
        }
        let mut view = apply::RemoteView {
            cache,
            verify_deletions: refresh == Refresh::CaughtUp,
        };
        let report = apply::apply_plan(self, plan, &mut view).await?;
        self.state.save_event_cursor(&self.pair.id, &cursor)?;
        Ok(SyncOutcome::Applied(report))
    }
}
