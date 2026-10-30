//! Use cases besides the actual sync: trash, conflicts and upload jobs.

mod conflicts;
mod remote_folder;
mod trash;
mod upload_job;

pub use conflicts::ConflictService;
pub use remote_folder::{ensure_folder, find_folder};
pub use trash::TrashService;
pub use upload_job::{JobFailure, JobPlan, JobReport, JobResult, PlannedUpload, UploadJobRunner};

use thiserror::Error;

use crate::ports::{ConflictId, LocalError, RemoteError, StateError, TrashId};

/// Errors of the use cases.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UseCaseError {
    /// Error in the remote storage.
    #[error(transparent)]
    Remote(#[from] RemoteError),
    /// Error in the local file system.
    #[error(transparent)]
    Local(#[from] LocalError),
    /// Error in the state store.
    #[error(transparent)]
    State(#[from] StateError),
    /// No conflict with this identifier.
    #[error("conflict {0:?} not found")]
    UnknownConflict(ConflictId),
    /// The remote file is protected (`protect_remote`), the local version cannot replace it.
    #[error("{0} is protected in Proton; keep both versions or turn off protect_remote")]
    RemoteProtected(protonsync_domain::path::RelativePath),
    /// No trash entry with this identifier.
    #[error("trash entry {0:?} not found")]
    UnknownTrashEntry(TrashId),
}
