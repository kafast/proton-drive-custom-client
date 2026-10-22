//! Data types of the API (data transfer objects).
//!
//! Deliberately independent of the domain types: the API stays stable even when
//! the internals change. The daemon does the conversion.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A password in a request. `Debug` does not show the content.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Password(String);

impl Password {
    /// Takes over a password.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Access to the plain text; only call it where it is needed.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(<hidden>)")
    }
}

/// Overall status of the daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusInfo {
    /// Version of the daemon.
    pub daemon_version: String,
    /// Logged-in user, if logged in.
    pub account: Option<String>,
    /// Whether a login has been started but not finished.
    pub login_pending: bool,
    /// Whether automatic syncs only plan (dry-run protection on first use).
    pub dry_run: bool,
    /// Sync pairs.
    pub pairs: Vec<PairStatus>,
}

/// Status of a sync pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairStatus {
    /// Id from the configuration.
    pub id: String,
    /// Local folder.
    pub local_root: String,
    /// Folder in "My files".
    pub remote_path: String,
    /// Direction.
    pub direction: Direction,
    /// Whether a sync is running.
    pub syncing: bool,
    /// Result of the last sync.
    pub last_sync: Option<PairSyncResult>,
    /// Open conflicts.
    pub open_conflicts: usize,
    /// Entries in the trash.
    pub trash_entries: usize,
}

/// Sync direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// In both directions.
    TwoWay,
    /// Only from Proton to local.
    DownloadOnly,
    /// Only from local to Proton.
    UploadOnly,
}

/// Progress of the login.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginStep {
    /// A 2FA code is required next.
    pub needs_second_factor: bool,
    /// The account has a separate data password.
    pub needs_data_password: bool,
}

/// Result of a sync for a sync pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairSyncResult {
    /// Sync pair.
    pub pair: String,
    /// Point in time (Unix seconds).
    pub finished_at: i64,
    /// Outcome.
    pub outcome: SyncOutcomeInfo,
}

/// Outcome of a sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncOutcomeInfo {
    /// Dry run: planned steps, nothing changed.
    Planned {
        /// Planned steps.
        steps: Vec<PlannedStepInfo>,
        /// Skipped paths.
        skipped: Vec<SkippedInfo>,
    },
    /// Applied.
    Applied(SyncSummary),
    /// The run failed.
    Failed {
        /// Error message.
        message: String,
    },
}

/// A planned step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedStepInfo {
    /// Path relative to the sync folder.
    pub path: String,
    /// Action.
    pub action: StepAction,
}

/// Action of a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepAction {
    /// Upload or create remotely.
    Upload,
    /// Download or create locally.
    Download,
    /// To the trash remotely.
    TrashRemote,
    /// To the trash locally.
    TrashLocal,
    /// Conflict; both versions are kept.
    Conflict,
    /// Local change is not transferred because of the direction.
    IgnoreLocalChange,
    /// Remote change is not transferred because of the direction.
    IgnoreRemoteChange,
    /// Both sides are equal; only the state is recorded.
    RecordConverged,
}

/// A skipped path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedInfo {
    /// Path.
    pub path: String,
    /// Reason.
    pub reason: String,
}

/// Figures of an applied sync.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSummary {
    /// Uploaded.
    pub uploaded: usize,
    /// Downloaded.
    pub downloaded: usize,
    /// To the trash remotely.
    pub trashed_remote: usize,
    /// To the trash locally.
    pub trashed_local: usize,
    /// New conflicts.
    pub conflicts: usize,
    /// Not transferred because of the direction.
    pub ignored: usize,
    /// Skipped paths.
    pub skipped: Vec<SkippedInfo>,
    /// Failed steps.
    pub failures: Vec<FailureInfo>,
}

/// A failed step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureInfo {
    /// Path.
    pub path: String,
    /// Error message.
    pub message: String,
}

/// An open conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictInfo {
    /// Sync pair.
    pub pair: String,
    /// Id for deciding.
    pub id: i64,
    /// Affected path.
    pub path: String,
    /// Kind.
    pub kind: ConflictKindInfo,
    /// Path of the conflict copy, if one was created.
    pub conflict_copy: Option<String>,
    /// Detected (Unix seconds).
    pub detected_at: i64,
}

/// Kind of a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKindInfo {
    /// Both sides changed.
    BothModified,
    /// Changed locally, deleted remotely.
    LocalModifiedRemoteDeleted,
    /// Deleted locally, changed remotely.
    LocalDeletedRemoteModified,
    /// Changed locally, the remote file is protected (`protect_remote`).
    LocalModifiedRemoteProtected,
}

/// Decision on a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionChoice {
    /// Keep the local version.
    KeepLocal,
    /// Keep the remote version.
    KeepRemote,
    /// Keep both.
    KeepBoth,
}

/// A trash entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrashInfo {
    /// Sync pair.
    pub pair: String,
    /// Id for restoring.
    pub id: i64,
    /// Original path.
    pub path: String,
    /// Where the deleted version is.
    pub side: TrashSide,
    /// Deleted (Unix seconds).
    pub trashed_at: i64,
    /// Permanent deletion from (Unix seconds).
    pub expires_at: i64,
}

/// Side of a trash entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrashSide {
    /// In the local trash of the sync folder.
    Local,
    /// In the Proton trash.
    Remote,
}

/// Error response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    /// Machine-readable code.
    pub code: ErrorCode,
    /// Message for humans.
    pub message: String,
}

impl ApiError {
    /// Creates an error response.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Error codes; the CLI derives its exit codes from them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Client and daemon speak different protocol versions.
    VersionMismatch,
    /// The request was unreadable or invalid.
    InvalidRequest,
    /// Not logged in.
    NotLoggedIn,
    /// Login rejected (wrong password or wrong code).
    LoginRejected,
    /// Unknown sync pair, conflict or trash entry.
    NotFound,
    /// Proton or the file system is temporarily unavailable.
    Unavailable,
    /// Other error.
    Internal,
}
