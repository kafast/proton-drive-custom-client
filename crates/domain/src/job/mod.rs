//! Upload-Jobs: Dateien aus einem Quellordner regelmäßig nach Proton kopieren
//! (z. B. Home-Assistant-Backups), mit Aufbewahrungsregel und Prüfung.

mod cron;
mod glob;

pub use cron::{CronSchedule, LocalDateTime};
pub use glob::GlobPattern;

use std::path::PathBuf;

use crate::DomainError;
use crate::path::RelativePath;
use crate::time::Timestamp;

/// Kennung eines Upload-Jobs in der Konfiguration (`a-z`, `0-9`, `-`, `_`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(String);

impl JobId {
    /// Prüft und erzeugt eine Kennung.
    ///
    /// # Errors
    /// [`DomainError::InvalidId`] bei leeren oder unerlaubten Zeichen.
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        let allowed =
            |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_';
        if value.is_empty() || !value.chars().all(allowed) {
            return Err(DomainError::InvalidId(value.to_owned()));
        }
        Ok(Self(value.to_owned()))
    }

    /// Die Kennung als Text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Wann ein Job läuft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobTrigger {
    /// Nach Zeitplan.
    Schedule(CronSchedule),
    /// Sobald neue passende Dateien im Quellordner liegen (und zur Ruhe gekommen sind).
    OnNewFile,
}

/// Wie ein Upload geprüft wird.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Verification {
    /// Größe der Datei in Proton stimmt.
    Size,
    /// Größe und SHA-1-Prüfsumme des Inhalts stimmen (Standard).
    #[default]
    Hash,
}

/// Wie viele hochgeladene Dateien im Zielordner bleiben.
///
/// Ohne Angaben bleibt alles. Sonst gilt eine Datei als abgelaufen, wenn sie
/// nicht mehr zu den `keep_last` neuesten gehört **oder** älter als
/// `max_age_days` ist. Die neueste Datei bleibt immer, damit z. B. nach einem
/// Ausfall der Backups nicht alles verschwindet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Retention {
    keep_last: Option<u32>,
    max_age_days: Option<u32>,
}

impl Retention {
    /// Alles behalten.
    #[must_use]
    pub const fn keep_all() -> Self {
        Self {
            keep_last: None,
            max_age_days: None,
        }
    }

    /// Erzeugt eine Regel.
    ///
    /// # Errors
    /// [`DomainError::InvalidRetention`], wenn ein Wert 0 ist.
    pub fn new(keep_last: Option<u32>, max_age_days: Option<u32>) -> Result<Self, DomainError> {
        if keep_last == Some(0) {
            return Err(DomainError::InvalidRetention(
                "keep_last muss mindestens 1 sein",
            ));
        }
        if max_age_days == Some(0) {
            return Err(DomainError::InvalidRetention(
                "max_age_days muss mindestens 1 sein",
            ));
        }
        Ok(Self {
            keep_last,
            max_age_days,
        })
    }

    /// Anzahl der neuesten Dateien, die bleiben.
    #[must_use]
    pub fn keep_last(&self) -> Option<u32> {
        self.keep_last
    }

    /// Höchstalter in Tagen.
    #[must_use]
    pub fn max_age_days(&self) -> Option<u32> {
        self.max_age_days
    }

    /// `true`, wenn nie etwas abläuft.
    #[must_use]
    pub fn keeps_everything(&self) -> bool {
        self.keep_last.is_none() && self.max_age_days.is_none()
    }

    /// Wählt die abgelaufenen Einträge aus `files` (Schlüssel und Änderungszeit).
    ///
    /// Die Reihenfolge des Ergebnisses ist die Reihenfolge in `files`.
    #[must_use]
    pub fn expired<'a, K: Ord>(&self, files: &'a [(K, Timestamp)], now: Timestamp) -> Vec<&'a K> {
        if self.keeps_everything() || files.is_empty() {
            return Vec::new();
        }
        // Neueste zuerst; bei gleicher Zeit entscheidet der Schlüssel, damit das
        // Ergebnis nicht von der Eingabereihenfolge abhängt.
        let mut by_age: Vec<usize> = (0..files.len()).collect();
        by_age.sort_by(|&a, &b| {
            files[b]
                .1
                .cmp(&files[a].1)
                .then_with(|| files[b].0.cmp(&files[a].0))
        });
        let mut expired = vec![false; files.len()];
        for (rank, &index) in by_age.iter().enumerate().skip(1) {
            let too_many = self
                .keep_last
                .is_some_and(|keep| rank >= usize::try_from(keep).unwrap_or(usize::MAX));
            let too_old = self
                .max_age_days
                .is_some_and(|days| files[index].1.plus_days(days) < now);
            expired[index] = too_many || too_old;
        }
        files
            .iter()
            .zip(expired)
            .filter_map(|((key, _), expired)| expired.then_some(key))
            .collect()
    }
}

/// Ein Upload-Job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadJob {
    /// Kennung.
    pub id: JobId,
    /// Quellordner (lokal oder gemountetes Netzlaufwerk). Unterordner werden nicht gelesen.
    pub source: PathBuf,
    /// Welche Dateinamen im Quellordner hochgeladen werden.
    pub pattern: GlobPattern,
    /// Zielordner unterhalb von „Meine Dateien“.
    pub target: RelativePath,
    /// Wann der Job läuft.
    pub trigger: JobTrigger,
    /// Aufbewahrung im Zielordner (nur Dateien, die zum Muster passen).
    pub retention: Retention,
    /// Prüfung nach dem Upload.
    pub verification: Verification,
    /// Versuche je Lauf, bevor der Fehler gemeldet wird (mindestens 1).
    pub max_attempts: u32,
    /// So viele Sekunden muss eine Datei unverändert sein, bevor sie hochgeladen
    /// wird (verhindert Uploads halb geschriebener Backups).
    pub settle_seconds: u32,
}

/// Ergebnis eines Laufs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobOutcome {
    /// Probelauf: nur gezählt, was passieren würde.
    Planned {
        /// Dateien, die hochgeladen würden.
        uploads: u32,
        /// Dateien, die die Aufbewahrung in den Papierkorb legen würde.
        expired: u32,
    },
    /// Lauf erfolgreich.
    Succeeded {
        /// Hochgeladen und geprüft.
        uploaded: u32,
        /// Schon vorhanden und unverändert.
        unchanged: u32,
        /// Durch die Aufbewahrung in den Papierkorb gelegt.
        expired: u32,
    },
    /// Lauf fehlgeschlagen.
    Failed {
        /// Fehlermeldung.
        message: String,
        /// Der wievielte Versuch dies war.
        attempt: u32,
    },
}

/// Ein Eintrag im Verlauf eines Jobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRun {
    /// Beginn.
    pub started_at: Timestamp,
    /// Ende.
    pub finished_at: Timestamp,
    /// Ergebnis.
    pub outcome: JobOutcome,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::SECONDS_PER_DAY;

    const NOW: Timestamp = Timestamp::from_unix_seconds(100 * SECONDS_PER_DAY);

    fn days_ago(days: i64) -> Timestamp {
        Timestamp::from_unix_seconds(NOW.unix_seconds() - days * SECONDS_PER_DAY)
    }

    fn files() -> Vec<(&'static str, Timestamp)> {
        vec![
            ("c", days_ago(3)),
            ("a", days_ago(10)),
            ("d", days_ago(1)),
            ("b", days_ago(5)),
        ]
    }

    #[test]
    fn job_ids_are_validated() {
        assert!(JobId::parse("ha-backup_1").is_ok());
        for bad in ["", "Groß", "a b", "a/b", "job:x"] {
            assert!(JobId::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn keep_last_keeps_the_newest() {
        let retention = Retention::new(Some(2), None).unwrap();
        assert_eq!(retention.expired(&files(), NOW), [&"a", &"b"]);
    }

    #[test]
    fn max_age_removes_old_files() {
        let retention = Retention::new(None, Some(4)).unwrap();
        assert_eq!(retention.expired(&files(), NOW), [&"a", &"b"]);
    }

    #[test]
    fn either_rule_expires_a_file() {
        let retention = Retention::new(Some(3), Some(4)).unwrap();
        assert_eq!(retention.expired(&files(), NOW), [&"a", &"b"]);
        let retention = Retention::new(Some(1), Some(30)).unwrap();
        assert_eq!(retention.expired(&files(), NOW), [&"c", &"a", &"b"]);
    }

    #[test]
    fn the_newest_file_always_stays() {
        let retention = Retention::new(None, Some(1)).unwrap();
        let old = [("alt", days_ago(50)), ("älter", days_ago(60))];
        assert_eq!(retention.expired(&old, NOW), [&"älter"]);
    }

    #[test]
    fn no_rule_keeps_everything_and_zero_is_rejected() {
        assert_eq!(Retention::keep_all().expired(&files(), NOW), Vec::<&&str>::new());
        assert!(Retention::default().keeps_everything());
        assert!(Retention::new(Some(0), None).is_err());
        assert!(Retention::new(None, Some(0)).is_err());
    }

    #[test]
    fn ties_are_broken_by_key() {
        let same = [("x", days_ago(1)), ("y", days_ago(1))];
        let retention = Retention::new(Some(1), None).unwrap();
        assert_eq!(retention.expired(&same, NOW), [&"x"]);
    }
}
