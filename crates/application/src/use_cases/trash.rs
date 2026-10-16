//! Trash: list, restore and permanently delete expired entries.

use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::trash::{RetentionPeriod, TrashEntry, TrashOrigin};

use super::UseCaseError;
use crate::ports::{Clock, LocalError, LocalStorage, RemoteBin, RemoteError, StateRepo, TrashId};

/// Trash of a sync pair.
pub struct TrashService<'a, R, L, S, C> {
    /// Remote storage (or the photo library via [`crate::ports::PhotoBin`]).
    pub remote: &'a R,
    /// Local folder of the sync pair.
    pub local: &'a L,
    /// Stored state.
    pub state: &'a S,
    /// Clock.
    pub clock: &'a C,
    /// Identifier of the sync pair.
    pub pair: &'a SyncPairId,
}

impl<R, L, S, C> TrashService<'_, R, L, S, C>
where
    R: RemoteBin,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    /// All entries, oldest first.
    ///
    /// # Errors
    /// [`UseCaseError::State`].
    pub fn list(&self) -> Result<Vec<(TrashId, TrashEntry)>, UseCaseError> {
        Ok(self.state.trash_entries(self.pair)?)
    }

    /// Restores an entry at its original location. The next
    /// sync then transfers it to the other side.
    ///
    /// # Errors
    /// [`UseCaseError::UnknownTrashEntry`], or [`UseCaseError::Local`] with
    /// `AlreadyExists` if something has appeared at the original location meanwhile.
    pub async fn restore(&self, id: TrashId) -> Result<(), UseCaseError> {
        let entry = self.find(id)?;
        match &entry.origin {
            TrashOrigin::Local { stored_as } => {
                self.create_parents(&entry.original_path)?;
                self.local
                    .restore_from_trash(stored_as, &entry.original_path)?;
            }
            TrashOrigin::Remote { node } => {
                self.remote.restore_from_trash(node).await?;
            }
        }
        self.state.remove_trash(id)?;
        Ok(())
    }

    /// Permanently deletes all entries whose retention period has expired, and returns them.
    /// With `dry_run = true` it only reports what would be deleted.
    ///
    /// # Errors
    /// [`UseCaseError`] on the first error; what is already deleted stays deleted.
    pub async fn purge_expired(
        &self,
        retention: RetentionPeriod,
        dry_run: bool,
    ) -> Result<Vec<(TrashId, TrashEntry)>, UseCaseError> {
        let now = self.clock.now();
        let mut purged = Vec::new();
        for (id, entry) in self.state.trash_entries(self.pair)? {
            if !entry.is_expired(retention, now) {
                continue;
            }
            if !dry_run {
                self.purge(&entry).await?;
                self.state.remove_trash(id)?;
            }
            purged.push((id, entry));
        }
        Ok(purged)
    }

    async fn purge(&self, entry: &TrashEntry) -> Result<(), UseCaseError> {
        // If the content is already gone (e.g. deleted by hand), the goal is reached.
        match &entry.origin {
            TrashOrigin::Local { stored_as } => match self.local.purge_trash(stored_as) {
                Ok(()) | Err(LocalError::NotFound(_)) => Ok(()),
                Err(error) => Err(error.into()),
            },
            TrashOrigin::Remote { node } => match self.remote.delete_from_trash(node).await {
                Ok(()) | Err(RemoteError::NotFound(_)) => Ok(()),
                Err(error) => Err(error.into()),
            },
        }
    }

    fn find(&self, id: TrashId) -> Result<TrashEntry, UseCaseError> {
        self.state
            .trash_entries(self.pair)?
            .into_iter()
            .find(|(candidate, _)| *candidate == id)
            .map(|(_, entry)| entry)
            .ok_or(UseCaseError::UnknownTrashEntry(id))
    }

    /// Creates missing parent folders so that the restore succeeds.
    fn create_parents(&self, path: &RelativePath) -> Result<(), UseCaseError> {
        let mut missing = Vec::new();
        let mut parent = path.parent();
        while let Some(folder) = parent.filter(|folder| !folder.is_root()) {
            if self.local.entry(&folder)?.is_none() {
                missing.push(folder.clone());
            }
            parent = folder.parent();
        }
        for folder in missing.iter().rev() {
            self.local.create_folder(folder)?;
        }
        Ok(())
    }
}
