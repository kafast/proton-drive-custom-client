//! Datentypen der API für Upload-Jobs.

use serde::{Deserialize, Serialize};

use crate::FailureInfo;

/// Präfix, unter dem Papierkorbeinträge eines Jobs in [`crate::TrashInfo::pair`] stehen,
/// z. B. `job:home-assistant`.
pub const JOB_TRASH_PREFIX: &str = "job:";

/// Ein Upload-Job mit Zustand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobInfo {
    /// Kennung aus der Konfiguration.
    pub id: String,
    /// Quellordner.
    pub source: String,
    /// Dateimuster.
    pub pattern: String,
    /// Zielordner in „Meine Dateien“.
    pub target: String,
    /// Cron-Ausdruck oder `on_new_file`.
    pub schedule: String,
    /// Anzahl der neuesten Dateien, die im Ziel bleiben.
    pub keep_last: Option<u32>,
    /// Höchstalter im Ziel in Tagen.
    pub max_age_days: Option<u32>,
    /// Prüfung nach dem Upload.
    pub verify: JobVerification,
    /// Ob der Job gerade läuft.
    pub running: bool,
    /// Letzter Lauf.
    pub last_run: Option<JobRunInfo>,
    /// Nächster geplanter Lauf (Unix-Sekunden); fehlt bei `on_new_file`.
    pub next_run: Option<i64>,
}

/// Prüfung nach dem Upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobVerification {
    /// Nur die Größe.
    Size,
    /// Größe und SHA-1-Prüfsumme.
    Hash,
}

/// Ein Lauf eines Jobs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRunInfo {
    /// Job.
    pub job: String,
    /// Beginn (Unix-Sekunden).
    pub started_at: i64,
    /// Ende (Unix-Sekunden).
    pub finished_at: i64,
    /// Ausgang.
    pub outcome: JobOutcomeInfo,
}

/// Ausgang eines Laufs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JobOutcomeInfo {
    /// Probelauf: nur gezählt.
    Planned {
        /// Dateien, die hochgeladen würden.
        uploads: u32,
        /// Dateien, die die Aufbewahrung in den Papierkorb legen würde.
        expired: u32,
    },
    /// Erfolgreich.
    Succeeded {
        /// Hochgeladen und geprüft.
        uploaded: u32,
        /// Schon vorhanden.
        unchanged: u32,
        /// In den Papierkorb gelegt.
        expired: u32,
    },
    /// Fehlgeschlagen.
    Failed {
        /// Fehlermeldung.
        message: String,
        /// Der wievielte Versuch.
        attempt: u32,
    },
}

/// Einzelheiten eines gerade ausgeführten Laufs (nicht im Verlauf gespeichert).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobDetails {
    /// Hochgeladene bzw. (Probelauf) hochzuladende Dateien.
    pub uploads: Vec<String>,
    /// Dateien, deren Ruhezeit noch läuft.
    pub waiting: Vec<String>,
    /// In den Papierkorb gelegte bzw. (Probelauf) zu legende Dateien.
    pub expired: Vec<String>,
    /// Fehlgeschlagene Dateien.
    pub failures: Vec<FailureInfo>,
}
