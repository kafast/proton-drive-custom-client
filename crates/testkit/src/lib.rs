//! Test-Hilfen für protonsync.
//!
//! - [`FakeRemote`]: deterministischer In-Memory-Ersatz für Proton Drive.
//! - [`FakeLocal`]: In-Memory-Ersatz für das lokale Dateisystem.
//! - [`InMemoryStateRepo`]: Sync-Zustand ohne Datenbank.
//! - [`FixedClock`]: Uhr, die nur auf Befehl weiterläuft.
//! - [`InMemorySecretStore`]: Geheimnisspeicher ohne Keyring.
//! - [`contract`]: Verhalten, das **jede** Implementierung eines Ports erfüllen
//!   muss; einbinden über [`remote_storage_contract_tests!`],
//!   [`local_storage_contract_tests!`] und [`state_repo_contract_tests!`].
//!
//! Dieses Crate wird nur als `dev-dependency` benutzt, nie im Produktivcode.

pub mod contract;
mod fake_clock;
mod fake_local;
mod fake_remote;
mod fake_secrets;
mod fake_state;
mod hash;

pub use fake_clock::FixedClock;
pub use fake_local::{FakeLocal, FakeStaging};
pub use fake_remote::FakeRemote;
pub use fake_secrets::InMemorySecretStore;
pub use fake_state::InMemoryStateRepo;
