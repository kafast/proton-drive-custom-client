//! Upload-Jobs in der Konfigurationsdatei (`[[job]]`).
//!
//! Beispiel:
//!
//! ```toml
//! [[job]]
//! id = "home-assistant"
//! source = "/mnt/ha-backups"     # lokaler Ordner oder gemountetes SMB/NFS
//! pattern = "*.tar"              # Standard "*"
//! target = "Backups/HomeAssistant"
//! schedule = "0 3 * * *"         # Cron-Ausdruck oder "on_new_file"
//! keep_last = 14                 # optional
//! max_age_days = 90              # optional
//! verify = "hash"                # oder "size"
//! ```

use std::path::PathBuf;

use protonsync_domain::job::{
    CronSchedule, GlobPattern, JobId, JobTrigger, Retention, UploadJob, Verification,
};
use serde::Deserialize;

use crate::config::{ConfigError, parse_remote};

/// Schlüsselwort für „bei neuer Datei“ statt eines Cron-Ausdrucks.
pub const ON_NEW_FILE: &str = "on_new_file";

const DEFAULT_PATTERN: &str = "*";
const DEFAULT_MAX_ATTEMPTS: u32 = 5;
const DEFAULT_SETTLE_SECONDS: u32 = 60;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawJob {
    id: String,
    source: PathBuf,
    pattern: Option<String>,
    target: String,
    schedule: String,
    keep_last: Option<u32>,
    max_age_days: Option<u32>,
    #[serde(default)]
    verify: RawVerification,
    max_attempts: Option<u32>,
    settle_seconds: Option<u32>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawVerification {
    Size,
    #[default]
    Hash,
}

impl TryFrom<RawJob> for UploadJob {
    type Error = ConfigError;

    fn try_from(raw: RawJob) -> Result<Self, Self::Error> {
        let id = JobId::parse(&raw.id).map_err(|_| {
            ConfigError::Invalid(format!(
                "Ungültige Job-Kennung {:?} (erlaubt: a-z, 0-9, -, _)",
                raw.id
            ))
        })?;
        // Ab hier beginnt jede Meldung mit dem Job, damit man sie in großen
        // Dateien wiederfindet.
        let invalid = |message: String| ConfigError::Invalid(format!("Job {}: {message}", raw.id));
        if !raw.source.is_absolute() {
            return Err(invalid(
                "Quellordner muss ein absoluter Pfad sein".to_owned(),
            ));
        }
        let pattern = GlobPattern::parse(raw.pattern.as_deref().unwrap_or(DEFAULT_PATTERN))
            .map_err(|e| invalid(e.to_string()))?;
        let target = parse_remote(&raw.target).map_err(|m| invalid(format!("Zielordner: {m}")))?;
        if target.is_root() {
            // Aufbewahrung würde sonst in „Meine Dateien“ selbst aufräumen.
            return Err(invalid(
                "Zielordner darf nicht „Meine Dateien“ selbst sein".to_owned(),
            ));
        }
        let trigger = parse_trigger(&raw.schedule).map_err(|e| invalid(e.to_string()))?;
        let retention =
            Retention::new(raw.keep_last, raw.max_age_days).map_err(|e| invalid(e.to_string()))?;
        let max_attempts = raw.max_attempts.unwrap_or(DEFAULT_MAX_ATTEMPTS);
        if max_attempts == 0 {
            return Err(invalid("max_attempts muss mindestens 1 sein".to_owned()));
        }
        Ok(Self {
            id,
            source: raw.source,
            pattern,
            target,
            trigger,
            retention,
            verification: match raw.verify {
                RawVerification::Size => Verification::Size,
                RawVerification::Hash => Verification::Hash,
            },
            max_attempts,
            settle_seconds: raw.settle_seconds.unwrap_or(DEFAULT_SETTLE_SECONDS),
        })
    }
}

fn parse_trigger(schedule: &str) -> Result<JobTrigger, protonsync_domain::DomainError> {
    if schedule.trim() == ON_NEW_FILE {
        Ok(JobTrigger::OnNewFile)
    } else {
        CronSchedule::parse(schedule).map(JobTrigger::Schedule)
    }
}

/// Prüft, dass Job-Kennungen eindeutig sind.
pub(crate) fn ensure_unique_job_ids(jobs: &[UploadJob]) -> Result<(), ConfigError> {
    for (index, job) in jobs.iter().enumerate() {
        if jobs[..index].iter().any(|other| other.id == job.id) {
            return Err(ConfigError::Invalid(format!(
                "Job-Kennung {} doppelt",
                job.id.as_str()
            )));
        }
    }
    Ok(())
}
