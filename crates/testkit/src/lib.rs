//! Test-Hilfen für protonsync.
//!
//! - [`FakeRemote`]: deterministischer In-Memory-Ersatz für Proton Drive.
//! - [`FixedClock`]: Uhr, die nur auf Befehl weiterläuft.
//! - [`InMemorySecretStore`]: Geheimnisspeicher ohne Keyring.
//! - [`contract`]: Verhalten, das **jede** `RemoteStorage`-Implementierung
//!   erfüllen muss; einbinden über [`remote_storage_contract_tests!`].
//!
//! Dieses Crate wird nur als `dev-dependency` benutzt, nie im Produktivcode.

pub mod contract;
mod fake_clock;
mod fake_remote;
mod fake_secrets;

pub use fake_clock::FixedClock;
pub use fake_remote::FakeRemote;
pub use fake_secrets::InMemorySecretStore;
