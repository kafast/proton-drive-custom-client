//! Port-Traits (Schnittstellen nach außen).

pub mod account;
pub mod clock;
pub mod drive_cache;
pub mod job_log;
pub mod local_storage;
pub mod photo_library;
pub mod remote_storage;
pub mod secret_store;
pub mod state_repo;

pub use account::{Account, AccountError, LoginProgress};
pub use clock::{Calendar, Clock, SystemClock, UtcCalendar};
pub use drive_cache::{CacheKey, CacheRecord, ContentCache, DriveRepo};
pub use job_log::{JOB_HISTORY_LIMIT, JobLog};
pub use local_storage::{LocalEntry, LocalError, LocalKind, LocalStorage};
pub use photo_library::{NewPhoto, PhotoBin, PhotoLibrary};
pub use remote_storage::{EventCursor, RemoteBin, RemoteError, RemoteEvent, RemoteStorage};
pub use secret_store::{Secret, SecretError, SecretKey, SecretStore};
pub use state_repo::{BaseEntry, ConflictId, StateError, StateRepo, TrashId};
