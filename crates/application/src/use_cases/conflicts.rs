//! Konflikte: anzeigen und auflösen.

use protonsync_domain::conflict::{Conflict, ConflictKind, Resolution};
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::trash::{TrashEntry, TrashOrigin};

use super::UseCaseError;
use crate::ports::{Clock, ConflictId, LocalError, LocalStorage, StateRepo};

/// Konfliktwarteschlange eines Sync-Paares.
///
/// Beim Erkennen hat die Sync-Engine bereits beide Versionen behalten. Die
/// Auflösung räumt nur noch lokal auf; der nächste Abgleich überträgt das
/// Ergebnis. Was dabei verschwindet, landet im Papierkorb.
pub struct ConflictService<'a, L, S, C> {
    /// Lokaler Ordner des Sync-Paares.
    pub local: &'a L,
    /// Gespeicherter Zustand.
    pub state: &'a S,
    /// Uhr.
    pub clock: &'a C,
    /// Kennung des Sync-Paares.
    pub pair: &'a SyncPairId,
}

impl<L, S, C> ConflictService<'_, L, S, C>
where
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    /// Offene Konflikte, älteste zuerst.
    ///
    /// # Errors
    /// [`UseCaseError::State`].
    pub fn list(&self) -> Result<Vec<(ConflictId, Conflict)>, UseCaseError> {
        Ok(self.state.conflicts(self.pair)?)
    }

    /// Löst einen Konflikt auf.
    ///
    /// - [`Resolution::KeepLocal`]: die lokale Version gilt; die andere geht in den Papierkorb.
    /// - [`Resolution::KeepRemote`]: die entfernte Version gilt; die andere geht in den Papierkorb.
    /// - [`Resolution::KeepBoth`]: beide bleiben, der Konflikt wird nur geschlossen.
    ///
    /// # Errors
    /// [`UseCaseError::UnknownConflict`] oder Fehler beim Aufräumen.
    pub fn resolve(&self, id: ConflictId, resolution: Resolution) -> Result<(), UseCaseError> {
        let conflict = self
            .list()?
            .into_iter()
            .find(|(candidate, _)| *candidate == id)
            .map(|(_, conflict)| conflict)
            .ok_or(UseCaseError::UnknownConflict(id))?;

        match (conflict.kind, resolution, &conflict.conflict_copy) {
            (_, Resolution::KeepBoth, _) => {}
            (ConflictKind::BothModified, Resolution::KeepLocal, Some(copy)) => {
                self.trash(&conflict.relative_path)?;
                self.local.rename(copy, &conflict.relative_path)?;
            }
            (ConflictKind::BothModified, Resolution::KeepRemote, Some(copy)) => self.trash(copy)?,
            (ConflictKind::LocalModifiedRemoteDeleted, Resolution::KeepRemote, _)
            | (ConflictKind::LocalDeletedRemoteModified, Resolution::KeepLocal, _) => {
                // Die „gewinnende“ Seite ist die Löschung.
                self.trash(&conflict.relative_path)?;
            }
            // Die gewünschte Version liegt schon an ihrem Platz.
            _ => {}
        }
        self.state.remove_conflict(id)?;
        Ok(())
    }

    /// Legt `path` in den lokalen Papierkorb; fehlt er schon, ist das in Ordnung.
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
