//! Test helpers for protonsync.
//!
//! - [`FakeAccount`]: account with fixed credentials (including 2FA).
//! - [`FakeRemote`]: deterministic in-memory stand-in for Proton Drive.
//! - [`RecordingRemote`]: wrapper that records every changing call to a
//!   [`RemoteStorage`](protonsync_application::ports::RemoteStorage);
//!   [`RecordingPhotoLibrary`] does the same for the photo library.
//! - [`FakeLocal`]: in-memory stand-in for the local file system.
//! - [`FakePhotoLibrary`]: deterministic in-memory stand-in for Proton Photos.
//! - [`InMemoryStateRepo`]: sync state without a database.
//! - [`InMemoryContentCache`]: cache of the virtual drive without files.
//! - [`FixedClock`]: clock that only advances on request.
//! - [`InMemorySecretStore`]: secret store without a keyring.
//! - [`contract`]: behavior that **every** implementation of a port must
//!   show; include it via [`remote_storage_contract_tests!`],
//!   [`local_storage_contract_tests!`], [`state_repo_contract_tests!`],
//!   [`drive_repo_contract_tests!`], [`content_cache_contract_tests!`] and
//!   [`photo_library_contract_tests!`].
//!
//! This crate is only used as a `dev-dependency`, never in production code.

pub mod contract;
mod fake_account;
mod fake_cache;
mod fake_clock;
mod fake_disk;
mod fake_local;
mod fake_notifier;
mod fake_photos;
mod fake_remote;
mod fake_secrets;
mod fake_state;
mod hash;
mod recording_remote;

pub use fake_account::{FAKE_PASSWORD, FAKE_TOTP, FAKE_USER, FakeAccount};
pub use fake_cache::InMemoryContentCache;
pub use fake_clock::FixedClock;
pub use fake_disk::FakeDiskSpace;
pub use fake_local::{FakeLocal, FakeStaging};
pub use fake_notifier::FakeNotifier;
pub use fake_photos::FakePhotoLibrary;
pub use fake_remote::{FakeRemote, UploadHold};
pub use fake_secrets::InMemorySecretStore;
pub use fake_state::InMemoryStateRepo;
pub use recording_remote::{PhotoWrite, RecordingPhotoLibrary, RecordingRemote, RemoteWrite};
