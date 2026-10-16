//! Datentypen der API (Data Transfer Objects).
//!
//! Bewusst unabhängig von den Domänentypen: Die API bleibt stabil, auch wenn
//! sich das Innere ändert. Die Übersetzung macht der Daemon.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Ein Passwort in einer Anfrage. `Debug` zeigt den Inhalt nicht.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Password(String);

impl Password {
    /// Übernimmt ein Passwort.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Zugriff auf den Klartext; nur dort aufrufen, wo er gebraucht wird.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(<verborgen>)")
    }
}

/// Gesamtzustand des Daemons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusInfo {
    /// Version des Daemons.
    pub daemon_version: String,
    /// Angemeldeter Benutzer, falls angemeldet.
    pub account: Option<String>,
    /// Ob gerade eine Anmeldung begonnen, aber nicht abgeschlossen ist.
    pub login_pending: bool,
    /// Ob automatische Abgleiche nur planen (Probelauf-Schutz beim ersten Einsatz).
    pub dry_run: bool,
    /// Sync-Paare.
    pub pairs: Vec<PairStatus>,
}

/// Zustand eines Sync-Paares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairStatus {
    /// Kennung aus der Konfiguration.
    pub id: String,
    /// Lokaler Ordner.
    pub local_root: String,
    /// Ordner in „Meine Dateien“.
    pub remote_path: String,
    /// Richtung.
    pub direction: Direction,
    /// Ob gerade abgeglichen wird.
    pub syncing: bool,
    /// Ergebnis des letzten Abgleichs.
    pub last_sync: Option<PairSyncResult>,
    /// Offene Konflikte.
    pub open_conflicts: usize,
    /// Einträge im Papierkorb.
    pub trash_entries: usize,
}

/// Sync-Richtung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// In beide Richtungen.
    TwoWay,
    /// Nur von Proton nach lokal.
    DownloadOnly,
    /// Nur von lokal nach Proton.
    UploadOnly,
}

/// Fortschritt der Anmeldung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginStep {
    /// Als Nächstes wird ein 2FA-Code verlangt.
    pub needs_second_factor: bool,
    /// Das Konto hat ein eigenes Datenpasswort.
    pub needs_data_password: bool,
}

/// Ergebnis eines Abgleichs für ein Sync-Paar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairSyncResult {
    /// Sync-Paar.
    pub pair: String,
    /// Zeitpunkt (Unix-Sekunden).
    pub finished_at: i64,
    /// Ausgang.
    pub outcome: SyncOutcomeInfo,
}

/// Ausgang eines Abgleichs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncOutcomeInfo {
    /// Probelauf: geplante Schritte, nichts geändert.
    Planned {
        /// Geplante Schritte.
        steps: Vec<PlannedStepInfo>,
        /// Übersprungene Pfade.
        skipped: Vec<SkippedInfo>,
    },
    /// Ausgeführt.
    Applied(SyncSummary),
    /// Der Lauf ist gescheitert.
    Failed {
        /// Fehlermeldung.
        message: String,
    },
}

/// Ein geplanter Schritt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedStepInfo {
    /// Pfad relativ zum Sync-Ordner.
    pub path: String,
    /// Aktion.
    pub action: StepAction,
}

/// Aktion eines Schritts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepAction {
    /// Hochladen bzw. entfernt anlegen.
    Upload,
    /// Herunterladen bzw. lokal anlegen.
    Download,
    /// Entfernt in den Papierkorb.
    TrashRemote,
    /// Lokal in den Papierkorb.
    TrashLocal,
    /// Konflikt; beide Versionen werden behalten.
    Conflict,
    /// Lokale Änderung wird wegen der Richtung nicht übertragen.
    IgnoreLocalChange,
    /// Entfernte Änderung wird wegen der Richtung nicht übertragen.
    IgnoreRemoteChange,
    /// Beide Seiten sind gleich; nur der Zustand wird vermerkt.
    RecordConverged,
}

/// Ein übersprungener Pfad.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedInfo {
    /// Pfad.
    pub path: String,
    /// Grund.
    pub reason: String,
}

/// Zahlen eines ausgeführten Abgleichs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSummary {
    /// Hochgeladen.
    pub uploaded: usize,
    /// Heruntergeladen.
    pub downloaded: usize,
    /// Entfernt in den Papierkorb.
    pub trashed_remote: usize,
    /// Lokal in den Papierkorb.
    pub trashed_local: usize,
    /// Neue Konflikte.
    pub conflicts: usize,
    /// Wegen der Richtung nicht übertragen.
    pub ignored: usize,
    /// Übersprungene Pfade.
    pub skipped: Vec<SkippedInfo>,
    /// Fehlgeschlagene Schritte.
    pub failures: Vec<FailureInfo>,
}

/// Ein fehlgeschlagener Schritt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureInfo {
    /// Pfad.
    pub path: String,
    /// Fehlermeldung.
    pub message: String,
}

/// Ein offener Konflikt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictInfo {
    /// Sync-Paar.
    pub pair: String,
    /// Kennung zum Entscheiden.
    pub id: i64,
    /// Betroffener Pfad.
    pub path: String,
    /// Art.
    pub kind: ConflictKindInfo,
    /// Pfad der Konfliktkopie, falls eine angelegt wurde.
    pub conflict_copy: Option<String>,
    /// Erkannt (Unix-Sekunden).
    pub detected_at: i64,
}

/// Art eines Konflikts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKindInfo {
    /// Beide Seiten geändert.
    BothModified,
    /// Lokal geändert, entfernt gelöscht.
    LocalModifiedRemoteDeleted,
    /// Lokal gelöscht, entfernt geändert.
    LocalDeletedRemoteModified,
}

/// Entscheidung über einen Konflikt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionChoice {
    /// Lokale Version behalten.
    KeepLocal,
    /// Entfernte Version behalten.
    KeepRemote,
    /// Beide behalten.
    KeepBoth,
}

/// Ein Papierkorbeintrag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrashInfo {
    /// Sync-Paar.
    pub pair: String,
    /// Kennung zum Wiederherstellen.
    pub id: i64,
    /// Ursprünglicher Pfad.
    pub path: String,
    /// Wo die gelöschte Version liegt.
    pub side: TrashSide,
    /// Gelöscht (Unix-Sekunden).
    pub trashed_at: i64,
    /// Endgültige Löschung ab (Unix-Sekunden).
    pub expires_at: i64,
}

/// Seite eines Papierkorbeintrags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrashSide {
    /// Im lokalen Papierkorb des Sync-Ordners.
    Local,
    /// Im Proton-Papierkorb.
    Remote,
}

/// Fehlerantwort.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    /// Maschinenlesbarer Code.
    pub code: ErrorCode,
    /// Meldung für Menschen.
    pub message: String,
}

impl ApiError {
    /// Erzeugt eine Fehlerantwort.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Fehlercodes; die CLI leitet daraus ihre Exit-Codes ab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Client und Daemon sprechen verschiedene Protokollversionen.
    VersionMismatch,
    /// Die Anfrage war nicht lesbar oder ungültig.
    InvalidRequest,
    /// Nicht angemeldet.
    NotLoggedIn,
    /// Anmeldung abgelehnt (falsches Passwort oder falscher Code).
    LoginRejected,
    /// Unbekanntes Sync-Paar, Konflikt oder Papierkorbeintrag.
    NotFound,
    /// Proton oder das Dateisystem ist vorübergehend nicht erreichbar.
    Unavailable,
    /// Sonstiger Fehler.
    Internal,
}
