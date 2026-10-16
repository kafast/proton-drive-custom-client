//! Upload jobs: copy files from a source folder to Proton regularly
//! (e.g. Home Assistant backups), with a retention rule and verification.

mod cron;
mod glob;

pub use cron::{CronSchedule, LocalDateTime};
pub use glob::GlobPattern;

use std::path::PathBuf;

use crate::DomainError;
use crate::path::RelativePath;
use crate::time::Timestamp;

/// Id of an upload job in the configuration (`a-z`, `0-9`, `-`, `_`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(String);

impl JobId {
    /// Validates and creates an id.
    ///
    /// # Errors
    /// [`DomainError::InvalidId`] for empty values or disallowed characters.
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        let allowed =
            |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_';
        if value.is_empty() || !value.chars().all(allowed) {
            return Err(DomainError::InvalidId(value.to_owned()));
        }
        Ok(Self(value.to_owned()))
    }

    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// When a job runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobTrigger {
    /// On a schedule.
    Schedule(CronSchedule),
    /// As soon as new matching files are in the source folder (and have settled).
    OnNewFile,
}

/// How an upload is verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Verification {
    /// The size of the file in Proton matches.
    Size,
    /// Size and SHA-1 checksum of the content match (default).
    #[default]
    Hash,
}

/// How many uploaded files stay in the target folder.
///
/// Without settings everything stays. Otherwise a file counts as expired if it
/// is no longer among the `keep_last` newest **or** is older than
/// `max_age_days`. The newest file always stays so that, e.g. after the
/// backups stop, not everything disappears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Retention {
    keep_last: Option<u32>,
    max_age_days: Option<u32>,
}

impl Retention {
    /// Keep everything.
    #[must_use]
    pub const fn keep_all() -> Self {
        Self {
            keep_last: None,
            max_age_days: None,
        }
    }

    /// Creates a rule.
    ///
    /// # Errors
    /// [`DomainError::InvalidRetention`] if a value is 0.
    pub fn new(keep_last: Option<u32>, max_age_days: Option<u32>) -> Result<Self, DomainError> {
        if keep_last == Some(0) {
            return Err(DomainError::InvalidRetention(
                "keep_last must be at least 1",
            ));
        }
        if max_age_days == Some(0) {
            return Err(DomainError::InvalidRetention(
                "max_age_days must be at least 1",
            ));
        }
        Ok(Self {
            keep_last,
            max_age_days,
        })
    }

    /// Number of newest files that stay.
    #[must_use]
    pub fn keep_last(&self) -> Option<u32> {
        self.keep_last
    }

    /// Maximum age in days.
    #[must_use]
    pub fn max_age_days(&self) -> Option<u32> {
        self.max_age_days
    }

    /// `true` if nothing ever expires.
    #[must_use]
    pub fn keeps_everything(&self) -> bool {
        self.keep_last.is_none() && self.max_age_days.is_none()
    }

    /// Selects the expired entries from `files` (key and modification time).
    ///
    /// The order of the result is the order in `files`.
    #[must_use]
    pub fn expired<'a, K: Ord>(&self, files: &'a [(K, Timestamp)], now: Timestamp) -> Vec<&'a K> {
        if self.keeps_everything() || files.is_empty() {
            return Vec::new();
        }
        // Newest first; for equal times the key decides, so that the
        // result does not depend on the input order.
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

/// An upload job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadJob {
    /// Id.
    pub id: JobId,
    /// Source folder (local or mounted network drive). Subfolders are not read.
    pub source: PathBuf,
    /// Which file names in the source folder are uploaded.
    pub pattern: GlobPattern,
    /// Target folder below "My files".
    pub target: RelativePath,
    /// When the job runs.
    pub trigger: JobTrigger,
    /// Retention in the target folder (only files matching the pattern).
    pub retention: Retention,
    /// Verification after the upload.
    pub verification: Verification,
    /// Attempts per run before the error is reported (at least 1).
    pub max_attempts: u32,
    /// A file must be unchanged for this many seconds before it is uploaded
    /// (prevents uploads of half-written backups).
    pub settle_seconds: u32,
}

/// Outcome of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobOutcome {
    /// Dry run: only counted what would happen.
    Planned {
        /// Files that would be uploaded.
        uploads: u32,
        /// Files that the retention would move to the trash.
        expired: u32,
    },
    /// Run succeeded.
    Succeeded {
        /// Uploaded and verified.
        uploaded: u32,
        /// Already present and unchanged.
        unchanged: u32,
        /// Moved to the trash by the retention.
        expired: u32,
    },
    /// Run failed.
    Failed {
        /// Error message.
        message: String,
        /// Which attempt this was.
        attempt: u32,
    },
}

/// An entry in a job's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRun {
    /// Start.
    pub started_at: Timestamp,
    /// End.
    pub finished_at: Timestamp,
    /// Outcome.
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
        let old = [("old", days_ago(50)), ("older", days_ago(60))];
        assert_eq!(retention.expired(&old, NOW), [&"older"]);
    }

    #[test]
    fn no_rule_keeps_everything_and_zero_is_rejected() {
        assert_eq!(
            Retention::keep_all().expired(&files(), NOW),
            Vec::<&&str>::new()
        );
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
