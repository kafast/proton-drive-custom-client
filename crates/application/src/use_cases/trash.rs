//! Papierkorb: anzeigen, wiederherstellen und abgelaufene Einträge endgültig löschen.

use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::trash::{RetentionPeriod, TrashEntry, TrashOrigin};

use super::UseCaseError;
use crate::ports::{Clock, LocalError, LocalStorage, RemoteBin, RemoteError, StateRepo, TrashId};

/// Papierkorb eines Sync-Paares.
pub struct TrashService<'a, R, L, S, C> {
    /// Entfernter Speicher (bzw. Fotobibliothek über [`crate::ports::PhotoBin`]).
    pub remote: &'a R,
    /// Lokaler Ordner des Sync-Paares.
    pub local: &'a L,
    /// Gespeicherter Zustand.
    pub state: &'a S,
    /// Uhr.
    pub clock: &'a C,
    /// Kennung des Sync-Paares.
    pub pair: &'a SyncPairId,
}

impl<R, L, S, C> TrashService<'_, R, L, S, C>
where
    R: RemoteBin,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    /// Alle Einträge, älteste zuerst.
    ///
    /// # Errors
    /// [`UseCaseError::State`].
    pub fn list(&self) -> Result<Vec<(TrashId, TrashEntry)>, UseCaseError> {
        Ok(self.state.trash_entries(self.pair)?)
    }

    /// Stellt einen Eintrag an seinem ursprünglichen Ort wieder her. Der nächste
    /// Abgleich überträgt ihn dann auf die andere Seite.
    ///
    /// # Errors
    /// [`UseCaseError::UnknownTrashEntry`], oder [`UseCaseError::Local`] mit
    /// `AlreadyExists`, wenn am ursprünglichen Ort inzwischen etwas liegt.
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

    /// Löscht alle Einträge endgültig, deren Frist abgelaufen ist, und liefert sie.
    /// Mit `dry_run = true` wird nur gemeldet, was gelöscht würde.
    ///
    /// # Errors
    /// [`UseCaseError`] beim ersten Fehler; bereits Gelöschtes bleibt gelöscht.
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
        // Ist der Inhalt schon weg (z. B. von Hand gelöscht), ist das Ziel erreicht.
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

    /// Legt fehlende Elternordner an, damit die Wiederherstellung gelingt.
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
