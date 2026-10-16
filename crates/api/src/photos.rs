//! Datentypen der API für den Foto-Abgleich und die Duplikaterkennung (ADR 0012).

use serde::{Deserialize, Serialize};

use crate::{Direction, FailureInfo, SkippedInfo};

/// Unter diesem Namen stehen Papierkorbeinträge der Fotos in [`crate::TrashInfo::pair`].
pub const PHOTOS_TRASH_OWNER: &str = "photos:library";

/// Zustand des Foto-Abgleichs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoStatus {
    /// Lokaler Fotoordner.
    pub folder: String,
    /// Richtung.
    pub direction: Direction,
    /// Ob gerade abgeglichen wird.
    pub syncing: bool,
    /// Anzahl Fotos in Proton; fehlt, solange die Bibliothek nicht gelesen ist.
    pub photos: Option<usize>,
    /// Ergebnis des letzten Laufs.
    pub last_run: Option<PhotoRunInfo>,
    /// Einträge im Papierkorb.
    pub trash_entries: usize,
}

/// Ein Lauf des Foto-Abgleichs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoRunInfo {
    /// Ende (Unix-Sekunden).
    pub finished_at: i64,
    /// Ausgang.
    pub outcome: PhotoOutcomeInfo,
}

/// Ausgang eines Laufs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PhotoOutcomeInfo {
    /// Probelauf: was geschehen würde.
    Planned(PhotoCounts),
    /// Ausgeführt.
    Applied(PhotoCounts),
    /// Gescheitert, bevor etwas geschah.
    Failed {
        /// Fehlermeldung.
        message: String,
    },
}

/// Zahlen eines (geplanten oder ausgeführten) Laufs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoCounts {
    /// Heruntergeladen.
    pub downloaded: usize,
    /// Hochgeladen.
    pub uploaded: usize,
    /// Nur verknüpft (Inhalt auf beiden Seiten gleich).
    pub linked: usize,
    /// Lokal in den Papierkorb.
    pub trashed_local: usize,
    /// In Proton in den Papierkorb.
    pub trashed_remote: usize,
    /// Wegen der Richtung nicht übertragen.
    pub ignored: usize,
    /// Übersprungene Pfade.
    pub skipped: Vec<SkippedInfo>,
    /// Fehlgeschlagene Schritte (nur bei ausgeführten Läufen).
    pub failures: Vec<FailureInfo>,
}

/// Warum Fotos als Duplikate gelten.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateReasonInfo {
    /// Gleicher Inhalt: sicher dasselbe Foto.
    SameContent,
    /// Gleicher Name und gleiche Aufnahmesekunde: wahrscheinlich dasselbe Foto.
    SameNameAndCaptureTime,
}

/// Eine Gruppe von Duplikaten; nur ein Vorschlag, gelöscht wird nichts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateGroupInfo {
    /// Grund.
    pub reason: DuplicateReasonInfo,
    /// Aufnahmezeit (Unix-Sekunden).
    pub captured: i64,
    /// Vorschlag: dieses Foto behalten.
    pub keep: PhotoInfo,
    /// Die übrigen Fotos (Kandidaten zum Löschen).
    pub others: Vec<PhotoInfo>,
}

/// Ein Foto in Proton.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoInfo {
    /// Kennung; damit wird ein Duplikat gelöscht.
    pub id: String,
    /// Dateiname.
    pub name: String,
    /// Größe in Bytes.
    pub size: u64,
    /// Hochgeladen (Unix-Sekunden).
    pub uploaded: i64,
    /// Als Favorit markiert.
    pub favorite: bool,
    /// In so vielen Alben.
    pub albums: usize,
}

/// Ein Duplikat, das in den Papierkorb gelegt wurde (oder würde).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrashedPhotoInfo {
    /// Kennung.
    pub id: String,
    /// Pfad im Fotoordner.
    pub path: String,
}
