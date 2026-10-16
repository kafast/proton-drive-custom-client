//! Port-Traits (Schnittstellen nach außen).

pub mod account;
pub mod clock;
pub mod job_log;
pub mod local_storage;
pub mod remote_storage;
pub mod secret_store;
pub mod state_repo;

pub use account::{Account, AccountError, LoginProgress};
pub use clock::{Clock, SystemClock};
pub use job_log::{JOB_HISTORY_LIMIT, JobLog};
pub use local_storage::{LocalEntry, LocalError, LocalKind, LocalStorage};
pub use remote_storage::{EventCursor, RemoteError, RemoteEvent, RemoteStorage};
pub use secret_store::{Secret, SecretError, SecretKey, SecretStore};
pub use state_repo::{BaseEntry, ConflictId, StateError, StateRepo, TrashId};
