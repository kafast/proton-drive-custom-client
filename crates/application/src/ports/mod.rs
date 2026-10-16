//! Port-Traits (Schnittstellen nach außen).

pub mod clock;
pub mod remote_storage;
pub mod secret_store;

pub use clock::{Clock, SystemClock};
pub use remote_storage::{EventCursor, RemoteError, RemoteEvent, RemoteStorage};
pub use secret_store::{Secret, SecretError, SecretKey, SecretStore};
