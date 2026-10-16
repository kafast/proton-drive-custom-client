//! Ports für das virtuelle Laufwerk: Inhalte im Zwischenspeicher und ihr Index.

use std::io::Read;

use protonsync_domain::cache::BlockMap;
use protonsync_domain::node::{NodeId, RevisionId};
use protonsync_domain::path::RelativePath;
use protonsync_domain::pin::{PinRules, PinState};
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::time::Timestamp;

use super::{LocalError, StateError};

/// Kennung einer Datei im Zwischenspeicher (vom Laufwerk vergeben).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CacheKey(pub u64);

/// Inhalte des Zwischenspeichers: eine (dünn belegte) Datei je [`CacheKey`].
///
/// Synchron wie [`super::LocalStorage`]: lokale Zugriffe sind schnell.
pub trait ContentCache: Send + Sync {
    /// Lesbarer Inhalt (für den Upload).
    type Reader: Read + Send;

    /// Liest bis zu `length` Bytes ab `offset`. Hinter dem Ende kommen
    /// entsprechend weniger Bytes; eine fehlende Datei gilt als leer.
    ///
    /// # Errors
    /// [`LocalError::Io`].
    fn read(&self, key: CacheKey, offset: u64, length: usize) -> Result<Vec<u8>, LocalError>;

    /// Schreibt `data` ab `offset`; die Datei wird bei Bedarf angelegt und verlängert.
    ///
    /// # Errors
    /// [`LocalError::Io`], z. B. bei vollem Datenträger.
    fn write(&self, key: CacheKey, offset: u64, data: &[u8]) -> Result<(), LocalError>;

    /// Setzt die Länge (Abschneiden oder Verlängern mit Nullen).
    ///
    /// # Errors
    /// [`LocalError::Io`].
    fn set_len(&self, key: CacheKey, size: u64) -> Result<(), LocalError>;

    /// Öffnet den Inhalt zum Lesen von vorn.
    ///
    /// # Errors
    /// [`LocalError::NotFound`] oder [`LocalError::Io`].
    fn open(&self, key: CacheKey) -> Result<Self::Reader, LocalError>;

    /// Entfernt den Inhalt. Fehlt er schon, ist das kein Fehler.
    ///
    /// # Errors
    /// [`LocalError::Io`].
    fn remove(&self, key: CacheKey) -> Result<(), LocalError>;
}

/// Gespeicherter Stand einer Datei im Zwischenspeicher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheRecord {
    /// Kennung des Inhalts.
    pub key: CacheKey,
    /// Knoten in Proton; `None` bei einer lokal angelegten, noch nicht hochgeladenen Datei.
    pub node: Option<NodeId>,
    /// Pfad relativ zur Wurzel des Laufwerks (für Anheft-Regeln und zum
    /// Wiederaufnehmen eines Uploads nach einem Neustart).
    pub path: RelativePath,
    /// Revision in Proton, auf der der Inhalt beruht; `None` bei neuen Dateien.
    pub revision: Option<RevisionId>,
    /// Vorhandene Blöcke (enthält die Dateigröße).
    pub blocks: BlockMap,
    /// Letzter Zugriff (für die Freigabe).
    pub last_access: Timestamp,
    /// Änderungszeit des Inhalts.
    pub modified: Timestamp,
    /// `true`, solange lokale Änderungen nicht in Proton sind.
    pub changed: bool,
}

/// Index des Zwischenspeichers und Anheft-Regeln je Laufwerk (in SQLite).
pub trait DriveRepo: Send + Sync {
    /// Alle Einträge eines Laufwerks.
    ///
    /// # Errors
    /// [`StateError`].
    fn cache_records(&self, drive: &SyncPairId) -> Result<Vec<CacheRecord>, StateError>;

    /// Legt einen Eintrag an oder ersetzt den mit demselben Schlüssel.
    ///
    /// # Errors
    /// [`StateError`].
    fn save_cache_record(&self, drive: &SyncPairId, record: &CacheRecord)
    -> Result<(), StateError>;

    /// Entfernt einen Eintrag. Unbekannte Schlüssel sind kein Fehler.
    ///
    /// # Errors
    /// [`StateError`].
    fn remove_cache_record(&self, drive: &SyncPairId, key: CacheKey) -> Result<(), StateError>;

    /// Die Anheft-Regeln eines Laufwerks.
    ///
    /// # Errors
    /// [`StateError`].
    fn pin_rules(&self, drive: &SyncPairId) -> Result<PinRules, StateError>;

    /// Setzt (`Some`) oder entfernt (`None`) die Regel für genau `path`.
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

/// Ein geteilter Index (`Arc`) ist selbst ein Index.
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

/// Ein geteilter Zwischenspeicher (`Arc`) ist selbst ein Zwischenspeicher.
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
