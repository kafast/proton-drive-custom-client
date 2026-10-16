//! Datentypen der API für Dateimanager: Zustand einzelner Pfade (ADR 0013).

use serde::{Deserialize, Serialize};

/// Zustand eines Pfads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathStatusInfo {
    /// Der angefragte Pfad.
    pub path: String,
    /// Zustand.
    pub state: PathState,
    /// Ob der Pfad in einem Laufwerk liegt (dort gibt es Anheften und Freigeben).
    pub in_drive: bool,
    /// Ob „Immer lokal halten“ gilt (nur in Laufwerken).
    pub pinned: bool,
}

/// Was protonsync über einen Pfad weiß.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathState {
    /// Liegt in keinem Laufwerk, Sync- oder Fotoordner (oder ist intern, z. B. der Papierkorb).
    Outside,
    /// Ein Ordner in einem Laufwerk, Sync- oder Fotoordner.
    Folder,
    /// Laufwerk: nur Platzhalter, der Inhalt lädt beim Öffnen.
    OnlineOnly,
    /// Laufwerk: teilweise geladen.
    Partial,
    /// Laufwerk: vollständig lokal.
    Local,
    /// Laufwerk: lokal geändert, noch nicht hochgeladen.
    PendingUpload,
    /// Sync- oder Fotoordner: seit dem letzten Abgleich unverändert.
    Synced,
    /// Sync- oder Fotoordner: neu oder geändert; der nächste Abgleich überträgt es.
    Pending,
    /// Sync-Ordner: offener Konflikt.
    Conflict,
}
