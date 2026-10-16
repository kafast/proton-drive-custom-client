//! Use Cases neben dem eigentlichen Abgleich: Papierkorb und Konflikte.

mod conflicts;
mod trash;

pub use conflicts::ConflictService;
pub use trash::TrashService;

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
