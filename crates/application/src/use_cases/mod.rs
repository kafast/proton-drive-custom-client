//! Use Cases neben dem eigentlichen Abgleich: Papierkorb, Konflikte und Upload-Jobs.

mod conflicts;
mod remote_folder;
mod trash;
mod upload_job;

pub use conflicts::ConflictService;
pub use remote_folder::ensure_folder;
pub use trash::TrashService;
pub use upload_job::{JobFailure, JobPlan, JobReport, JobResult, PlannedUpload, UploadJobRunner};

use thiserror::Error;

use crate::ports::{ConflictId, LocalError, RemoteError, StateError, TrashId};

/// Fehler der Use Cases.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UseCaseError {
    /// Fehler beim entfernten Speicher.
    #[error(transparent)]
    Remote(#[from] RemoteError),
    /// Fehler im lokalen Dateisystem.
    #[error(transparent)]
    Local(#[from] LocalError),
    /// Fehler im Zustandsspeicher.
    #[error(transparent)]
    State(#[from] StateError),
    /// Kein Konflikt mit dieser Kennung.
    #[error("Konflikt {0:?} nicht gefunden")]
    UnknownConflict(ConflictId),
    /// Kein Papierkorbeintrag mit dieser Kennung.
    #[error("Papierkorbeintrag {0:?} nicht gefunden")]
    UnknownTrashEntry(TrashId),
}
