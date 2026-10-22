//! Conflicts: list and resolve.

use protonsync_domain::conflict::{Conflict, ConflictKind, Resolution};
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::trash::{TrashEntry, TrashOrigin};

use super::UseCaseError;
use crate::ports::{Clock, ConflictId, LocalError, LocalStorage, StateRepo};

/// Conflict queue of a sync pair.
///
/// On detection the sync engine has already kept both versions. The
/// resolution only cleans up locally; the next sync transfers the
/// result. Whatever disappears in the process ends up in the trash.
pub struct ConflictService<'a, L, S, C> {
    /// Local folder of the sync pair.
    pub local: &'a L,
    /// Stored state.
    pub state: &'a S,
    /// Clock.
    pub clock: &'a C,
    /// Identifier of the sync pair.
    pub pair: &'a SyncPairId,
}

impl<L, S, C> ConflictService<'_, L, S, C>
where
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    /// Open conflicts, oldest first.
    ///
    /// # Errors
    /// [`UseCaseError::State`].
    pub fn list(&self) -> Result<Vec<(ConflictId, Conflict)>, UseCaseError> {
        Ok(self.state.conflicts(self.pair)?)
    }

    /// Resolves a conflict.
    ///
    /// - [`Resolution::KeepLocal`]: the local version wins; the other goes to the trash.
    /// - [`Resolution::KeepRemote`]: the remote version wins; the other goes to the trash.
    /// - [`Resolution::KeepBoth`]: both stay, the conflict is only closed.
    ///
    /// With [`ConflictKind::LocalModifiedRemoteProtected`] the local version cannot win,
    /// because the sync would not replace the protected remote file anyway.
    ///
    /// # Errors
    /// [`UseCaseError::UnknownConflict`], [`UseCaseError::RemoteProtected`] or errors
    /// while cleaning up.
    pub fn resolve(&self, id: ConflictId, resolution: Resolution) -> Result<(), UseCaseError> {
        let conflict = self
            .list()?
            .into_iter()
            .find(|(candidate, _)| *candidate == id)
            .map(|(_, conflict)| conflict)
            .ok_or(UseCaseError::UnknownConflict(id))?;

        match (conflict.kind, resolution, &conflict.conflict_copy) {
            (_, Resolution::KeepBoth, _) => {}
            (ConflictKind::LocalModifiedRemoteProtected, Resolution::KeepLocal, _) => {
                return Err(UseCaseError::RemoteProtected(conflict.relative_path));
            }
            (ConflictKind::BothModified, Resolution::KeepLocal, Some(copy)) => {
                self.trash(&conflict.relative_path)?;
                self.local.rename(copy, &conflict.relative_path)?;
            }
            (
                ConflictKind::BothModified | ConflictKind::LocalModifiedRemoteProtected,
                Resolution::KeepRemote,
                Some(copy),
            ) => self.trash(copy)?,
            (ConflictKind::LocalModifiedRemoteDeleted, Resolution::KeepRemote, _)
            | (ConflictKind::LocalDeletedRemoteModified, Resolution::KeepLocal, _) => {
                // The "winning" side is the deletion.
                self.trash(&conflict.relative_path)?;
            }
            // The desired version is already in its place.
            _ => {}
        }
        self.state.remove_conflict(id)?;
        Ok(())
    }

    /// Moves `path` to the local trash; if it is already gone, that is fine.
    fn trash(&self, path: &RelativePath) -> Result<(), UseCaseError> {
        match self.local.move_to_trash(path) {
            Ok(stored_as) => {
                let entry = TrashEntry {
                    original_path: path.clone(),
                    origin: TrashOrigin::Local { stored_as },
                    trashed_at: self.clock.now(),
                };
                self.state.add_trash(self.pair, &entry)?;
                Ok(())
            }
            Err(LocalError::NotFound(_)) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}
