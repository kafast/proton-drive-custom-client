//! API data types for upload jobs.

use serde::{Deserialize, Serialize};

use crate::FailureInfo;

/// Prefix under which a job's trash entries appear in [`crate::TrashInfo::pair`],
/// e.g. `job:home-assistant`.
pub const JOB_TRASH_PREFIX: &str = "job:";

/// An upload job with its status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobInfo {
    /// Id from the configuration.
    pub id: String,
    /// Source folder.
    pub source: String,
    /// File pattern.
    pub pattern: String,
    /// Target folder in "My files".
    pub target: String,
    /// Cron expression or `on_new_file`.
    pub schedule: String,
    /// Number of newest files that stay in the target.
    pub keep_last: Option<u32>,
    /// Maximum age in the target in days.
    pub max_age_days: Option<u32>,
    /// Verification after the upload.
    pub verify: JobVerification,
    /// Whether the job is running.
    pub running: bool,
    /// Last run.
    pub last_run: Option<JobRunInfo>,
    /// Next scheduled run (Unix seconds); absent for `on_new_file`.
    pub next_run: Option<i64>,
}

/// Verification after the upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobVerification {
    /// Only the size.
    Size,
    /// Size and SHA-1 checksum.
    Hash,
}

/// A run of a job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRunInfo {
    /// Job.
    pub job: String,
    /// Start (Unix seconds).
    pub started_at: i64,
    /// End (Unix seconds).
    pub finished_at: i64,
    /// Outcome.
    pub outcome: JobOutcomeInfo,
}

/// Outcome of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JobOutcomeInfo {
    /// Dry run: only counted.
    Planned {
        /// Files that would be uploaded.
        uploads: u32,
        /// Files that the retention would move to the trash.
        expired: u32,
    },
    /// Succeeded.
    Succeeded {
        /// Uploaded and verified.
        uploaded: u32,
        /// Already present.
        unchanged: u32,
        /// Moved to the trash.
        expired: u32,
    },
    /// Failed.
    Failed {
        /// Error message.
        message: String,
        /// Which attempt.
        attempt: u32,
    },
}

/// Details of a run just carried out (not stored in the history).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobDetails {
    /// Uploaded files or (dry run) files to upload.
    pub uploads: Vec<String>,
    /// Files whose settle time is still running.
    pub waiting: Vec<String>,
    /// Files moved to the trash or (dry run) to be moved.
    pub expired: Vec<String>,
    /// Failed files.
    pub failures: Vec<FailureInfo>,
}
