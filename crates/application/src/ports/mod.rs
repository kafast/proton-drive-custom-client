//! Port-Traits (Schnittstellen nach außen).

pub mod clock;
pub mod local_storage;
pub mod remote_storage;
pub mod secret_store;
pub mod state_repo;

pub use clock::{Clock, SystemClock};
pub use local_storage::{LocalEntry, LocalError, LocalKind, LocalStorage};
pub use remote_storage::{EventCursor, RemoteError, RemoteEvent, RemoteStorage};
pub use secret_store::{Secret, SecretError, SecretKey, SecretStore};
pub use state_repo::{BaseEntry, ConflictId, StateError, StateRepo, TrashId};
