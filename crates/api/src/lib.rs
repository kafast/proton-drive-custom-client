//! Versioned API between the daemon and its front ends (CLI, web, GUI).
//!
//! Transport: one message per line as JSON over a Unix socket
//! ([`wire`]). Every message carries the [`PROTOCOL_VERSION`]; the daemon rejects
//! other versions with [`ErrorCode::VersionMismatch`].
//!
//! Flow: the client sends a [`ClientMessage`] and receives exactly one
//! [`ServerMessage::Response`]. After [`Request::Subscribe`] the
//! connection stays open and the daemon sends [`ServerMessage::Event`]s.
//!
//! Changes to these types are contract changes: the tests in
//! `tests/contract.rs` pin down the JSON format.

mod dto;
mod files;
mod jobs;
mod mounts;
mod photos;
pub mod wire;

pub use dto::*;
pub use files::*;
pub use jobs::*;
pub use mounts::*;
pub use photos::*;

use serde::{Deserialize, Serialize};

/// Protocol version. Increase on incompatible changes.
pub const PROTOCOL_VERSION: u32 = 1;

/// Message from the client to the daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientMessage {
    /// Protocol version of the client.
    pub version: u32,
    /// The request.
    pub request: Request,
}

impl ClientMessage {
    /// Wraps a request with the current protocol version.
    #[must_use]
    pub fn new(request: Request) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request,
        }
    }
}

/// Message from the daemon to the client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "body", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Response to the last request.
    Response(Response),
    /// Event after [`Request::Subscribe`].
    Event(Event),
}

/// Requests to the daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Query the overall status.
    Status,
    /// Start the login.
    LoginStart {
        /// Proton user name or email.
        username: String,
        /// Login password.
        password: Password,
    },
    /// Submit the 2FA code (TOTP).
    LoginSecondFactor {
        /// Six-digit code from the authenticator app.
        code: String,
    },
    /// Finish the login. `data_password` only in two-password mode.
    LoginFinish {
        /// Mailbox/data password; if absent, the login password applies.
        data_password: Option<Password>,
    },
    /// Log out and delete the stored session.
    Logout,
    /// Sync right away. Without `apply` only plan (dry run).
    Sync {
        /// Only this sync pair; otherwise all.
        pair: Option<String>,
        /// Actually apply the changes.
        apply: bool,
    },
    /// Open conflicts of all sync pairs.
    Conflicts,
    /// Decide a conflict.
    ResolveConflict {
        /// Sync pair.
        pair: String,
        /// Id from [`ConflictInfo::id`].
        id: i64,
        /// Decision.
        resolution: ResolutionChoice,
    },
    /// Trash of all sync pairs.
    Trash,
    /// Restore a trash entry.
    RestoreTrash {
        /// Sync pair.
        pair: String,
        /// Id from [`TrashInfo::id`].
        id: i64,
    },
    /// Permanently delete expired trash entries. Without `apply` only list them.
    PurgeTrash {
        /// Actually delete.
        apply: bool,
    },
    /// Subscribe to events; the connection stays open afterwards.
    Subscribe,
    /// All upload jobs.
    Jobs,
    /// Run an upload job right away. Without `apply` only plan.
    RunJob {
        /// Job.
        job: String,
        /// Actually upload and clean up.
        apply: bool,
    },
    /// History of a job, newest runs first.
    JobHistory {
        /// Job.
        job: String,
        /// At most this many entries.
        limit: u32,
    },
    /// All FUSE drives.
    Mounts,
    /// Set or clear "Always keep local" for a file or folder.
    /// Rules below it are dropped; the new one applies to everything inside.
    Pin {
        /// Absolute path inside a mount point.
        path: String,
        /// New rule.
        state: PinChoice,
    },
    /// Free up space. With `path` everything below it is freed right away
    /// (except pinned items); without `path` the eviction rule (age, size limit)
    /// applies to all drives. Without `apply` only plan.
    Free {
        /// Absolute path inside a mount point.
        path: Option<String>,
        /// Actually remove.
        apply: bool,
    },
    /// Status of the photo sync.
    Photos,
    /// Sync photos right away. Without `apply` only plan.
    SyncPhotos {
        /// Actually apply the changes.
        apply: bool,
    },
    /// Search for duplicates in the photo library (suggestions only).
    PhotoDuplicates,
    /// Move selected duplicates to the trash. Without `apply` only check.
    /// At least one photo of every group stays.
    TrashPhotoDuplicates {
        /// Ids from [`PhotoInfo::id`].
        ids: Vec<String>,
        /// Actually move to the trash.
        apply: bool,
    },
    /// Status of paths for file managers; does not query Proton.
    PathStatus {
        /// Absolute paths.
        paths: Vec<String>,
    },
}

/// Responses of the daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    /// Response to [`Request::Status`].
    Status(StatusInfo),
    /// Next step of the login.
    LoginStep(LoginStep),
    /// Login finished.
    LoggedIn {
        /// Logged-in user.
        username: String,
    },
    /// Result of [`Request::Sync`], one entry per sync pair.
    Sync {
        /// Results.
        results: Vec<PairSyncResult>,
    },
    /// Response to [`Request::Conflicts`].
    Conflicts {
        /// Open conflicts.
        conflicts: Vec<ConflictInfo>,
    },
    /// Response to [`Request::Trash`] and [`Request::PurgeTrash`].
    Trash {
        /// Entries.
        entries: Vec<TrashInfo>,
    },
    /// Response to [`Request::Jobs`].
    Jobs {
        /// Jobs.
        jobs: Vec<JobInfo>,
    },
    /// Response to [`Request::RunJob`].
    JobRun {
        /// The run.
        run: JobRunInfo,
        /// Details.
        details: JobDetails,
    },
    /// Response to [`Request::JobHistory`].
    JobHistory {
        /// Runs, newest first.
        runs: Vec<JobRunInfo>,
    },
    /// Response to [`Request::Mounts`].
    Mounts {
        /// Drives.
        mounts: Vec<MountInfo>,
    },
    /// Response to [`Request::Free`], one entry per drive.
    Freed {
        /// Results.
        results: Vec<FreeResult>,
    },
    /// Response to [`Request::Photos`].
    Photos(PhotoStatus),
    /// Response to [`Request::SyncPhotos`].
    PhotoRun(PhotoRunInfo),
    /// Response to [`Request::PhotoDuplicates`].
    PhotoDuplicates {
        /// Groups, newest captures first.
        groups: Vec<DuplicateGroupInfo>,
    },
    /// Response to [`Request::TrashPhotoDuplicates`].
    PhotosTrashed {
        /// `true`: only checked, nothing deleted.
        dry_run: bool,
        /// Affected photos.
        photos: Vec<TrashedPhotoInfo>,
    },
    /// Response to [`Request::PathStatus`], in the order of the request.
    PathStatus {
        /// One entry per path.
        entries: Vec<PathStatusInfo>,
    },
    /// Request carried out, nothing else to report.
    Done,
    /// Request failed.
    Error(ApiError),
}

/// Events for subscribers (GUI, web).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A sync has started.
    SyncStarted {
        /// Sync pair.
        pair: String,
    },
    /// A sync has finished.
    SyncFinished(PairSyncResult),
    /// A new conflict is waiting for a decision.
    ConflictDetected(ConflictInfo),
    /// The login has expired or is invalid; the user must log in again.
    LoginRequired {
        /// Reason.
        message: String,
    },
    /// An upload job has started.
    JobStarted {
        /// Job.
        job: String,
    },
    /// An upload job has finished (also on failure).
    JobFinished(JobRunInfo),
    /// An upload job failed after all attempts; notify the user.
    JobGaveUp {
        /// Job.
        job: String,
        /// Last error message.
        message: String,
        /// Number of attempts.
        attempts: u32,
    },
    /// The photo sync has started.
    PhotoSyncStarted,
    /// The photo sync has finished (also on failure).
    PhotoSyncFinished(PhotoRunInfo),
}
