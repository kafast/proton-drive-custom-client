//! Datentypen der API für FUSE-Laufwerke (Platzhalter-Modus).

use serde::{Deserialize, Serialize};

/// Präfix, unter dem Papierkorbeinträge eines Laufwerks in [`crate::TrashInfo::pair`]
/// stehen, z. B. `mount:home`.
pub const MOUNT_TRASH_PREFIX: &str = "mount:";

/// Ein Laufwerk mit Zustand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountInfo {
    /// Kennung aus der Konfiguration.
    pub id: String,
    /// Einhängepunkt.
    pub mountpoint: String,
    /// Ordner in „Meine Dateien“.
    pub remote_path: String,
    /// Ob das Laufwerk gerade eingehängt ist.
    pub mounted: bool,
    /// Voreinstellung „Immer lokal halten“.
    pub keep_local: bool,
    /// Belegung des Zwischenspeichers; fehlt ohne Anmeldung.
    pub cache: Option<CacheInfo>,
    /// Ausdrücklich gesetzte Regeln für „Immer lokal halten“.
    pub pins: Vec<PinInfo>,
}

/// Belegung des Zwischenspeichers eines Laufwerks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheInfo {
    /// Dateien mit lokal vorhandenem Inhalt.
    pub files: u64,
    /// Belegte Bytes.
    pub bytes: u64,
    /// Dateien mit lokalen Änderungen, die noch hochgeladen werden.
    pub pending_uploads: u64,
}

/// Eine Regel für „Immer lokal halten“.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinInfo {
    /// Pfad relativ zum Einhängepunkt.
    pub path: String,
    /// Wie die Regel lautet.
    pub state: PinChoice,
}

/// „Immer lokal halten“ oder „nur bei Bedarf“.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinChoice {
    /// Immer lokal halten (wird vollständig geladen, nie freigegeben).
    Pinned,
    /// Nur bei Bedarf laden (darf freigegeben werden).
    Unpinned,
}

/// Ergebnis einer Freigabe für ein Laufwerk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreeResult {
    /// Laufwerk.
    pub mount: String,
    /// `true`: nur geplant, nichts entfernt.
    pub dry_run: bool,
    /// Freigegebene (bzw. freizugebende) Dateien, relativ zum Einhängepunkt.
    pub files: Vec<String>,
    /// Dadurch frei werdende Bytes.
    pub bytes: u64,
    /// Übersprungene Dateien (angeheftet, geändert, geöffnet oder nicht bestätigt).
    pub skipped: u64,
}
