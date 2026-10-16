//! Port zum gespeicherten Sync-Zustand (Basis, Ereignis-Cursor, Konflikte, Papierkorb).
//!
//! Die Basis ist der zuletzt auf beiden Seiten übereinstimmende Stand je Pfad.
//! Sie wird erst nach einer erfolgreich abgeschlossenen Operation geschrieben;
//! bricht ein Lauf ab, erkennt der nächste Lauf die offenen Änderungen erneut.

use protonsync_domain::conflict::Conflict;
use protonsync_domain::node::{Fingerprint, NodeId, RevisionId};
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::trash::TrashEntry;
use thiserror::Error;

use super::remote_storage::EventCursor;

/// Fehler des Zustandsspeichers.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StateError {
    /// Speicher nicht les- oder schreibbar.
    #[error("Zustandsspeicher nicht verfügbar: {0}")]
    Unavailable(String),
    /// Gespeicherte Daten sind ungültig.
    #[error("Zustand beschädigt: {0}")]
    Corrupt(String),
}

/// Gemeinsamer Stand eines Pfades nach dem letzten erfolgreichen Abgleich.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseEntry {
    /// Pfad relativ zum Sync-Paar.
    pub path: RelativePath,
    /// Zugehöriger Knoten in Proton.
    pub remote_id: NodeId,
    /// Revision der entfernten Datei; `None` bei Ordnern.
    pub remote_revision: Option<RevisionId>,
    /// Merkmale der lokalen Datei; `None` bei Ordnern.
    pub local: Option<Fingerprint>,
}

impl BaseEntry {
    /// `true`, wenn der Eintrag ein Ordner ist.
    #[must_use]
    pub fn is_folder(&self) -> bool {
        self.remote_revision.is_none()
    }
}

/// Kennung eines Konflikts in der Warteschlange.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConflictId(pub i64);

/// Kennung eines Papierkorbeintrags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TrashId(pub i64);

/// Gespeicherter Sync-Zustand.
pub trait StateRepo: Send + Sync {
    /// Alle Basiseinträge eines Sync-Paares, sortiert nach Pfad.
    ///
    /// # Errors
    /// [`StateError`].
    fn base_entries(&self, pair: &SyncPairId) -> Result<Vec<BaseEntry>, StateError>;

    /// Der Basiseintrag für genau `path`, falls es einen gibt. Die Vorgabe sucht
    /// in [`Self::base_entries`]; Speicher mit Index sollten das schneller können.
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

    /// Legt einen Basiseintrag an oder ersetzt den für denselben Pfad.
    ///
    /// # Errors
    /// [`StateError`].
    fn save_base(&self, pair: &SyncPairId, entry: &BaseEntry) -> Result<(), StateError>;

    /// Entfernt den Basiseintrag für `path` **und alle darunter**.
    ///
    /// # Errors
    /// [`StateError`].
    fn remove_base_tree(&self, pair: &SyncPairId, path: &RelativePath) -> Result<(), StateError>;

    /// Zuletzt verarbeiteter Ereignis-Cursor.
    ///
    /// # Errors
    /// [`StateError`].
    fn event_cursor(&self, pair: &SyncPairId) -> Result<Option<EventCursor>, StateError>;

    /// Speichert den Ereignis-Cursor.
    ///
    /// # Errors
    /// [`StateError`].
    fn save_event_cursor(&self, pair: &SyncPairId, cursor: &EventCursor) -> Result<(), StateError>;

    /// Stellt einen Konflikt in die Warteschlange.
    ///
    /// # Errors
    /// [`StateError`].
    fn add_conflict(
        &self,
        pair: &SyncPairId,
        conflict: &Conflict,
    ) -> Result<ConflictId, StateError>;

    /// Offene Konflikte eines Sync-Paares, älteste zuerst.
    ///
    /// # Errors
    /// [`StateError`].
    fn conflicts(&self, pair: &SyncPairId) -> Result<Vec<(ConflictId, Conflict)>, StateError>;

    /// Entfernt einen erledigten Konflikt. Unbekannte Kennungen sind kein Fehler.
    ///
    /// # Errors
    /// [`StateError`].
    fn remove_conflict(&self, id: ConflictId) -> Result<(), StateError>;

    /// Vermerkt einen Eintrag im Papierkorb.
    ///
    /// # Errors
    /// [`StateError`].
    fn add_trash(&self, pair: &SyncPairId, entry: &TrashEntry) -> Result<TrashId, StateError>;

    /// Papierkorbeinträge eines Sync-Paares, älteste zuerst.
    ///
    /// # Errors
    /// [`StateError`].
    fn trash_entries(&self, pair: &SyncPairId) -> Result<Vec<(TrashId, TrashEntry)>, StateError>;

    /// Entfernt einen Papierkorbeintrag. Unbekannte Kennungen sind kein Fehler.
    ///
    /// # Errors
    /// [`StateError`].
    fn remove_trash(&self, id: TrashId) -> Result<(), StateError>;
}

/// Ein geteilter Zustand (`Arc`) ist selbst ein Zustandsspeicher; jede Methode leitet nur weiter.
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
