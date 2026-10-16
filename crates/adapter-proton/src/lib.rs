//! Proton-Drive-Anbindung (Adapter für den Port `RemoteStorage`).
//!
//! **Einziges Crate, das `proton-sdk` und `proton-drive-rs` kennt** (ADR 0002).
//! Es übersetzt zwischen den Typen der Community-Crates und den Domänentypen.
//!
//! - [`auth`]: Anmeldung mit Passwort und TOTP, Speichern und Fortsetzen der Sitzung.
//! - [`ProtonAccount`]: Port `Account` (Anmeldung, Sitzung, Ordner je Sync-Paar).
//! - [`ProtonRemote`]: `RemoteStorage` auf einem Ordner in „Meine Dateien“.

mod account;
pub mod auth;
mod convert;
mod remote;

pub use account::ProtonAccount;
pub use remote::ProtonRemote;

/// Ehrliche Kennung als Drittanbieter-App im Header `x-pm-appversion`
/// (Schema `external-drive-{name}@{semver}-{channel}`, siehe CLAUDE.md).
pub const APP_VERSION: &str = concat!(
    "external-drive-protonsync@",
    env!("CARGO_PKG_VERSION"),
    "-alpha"
);

/// User-Agent der HTTP-Anfragen.
pub const USER_AGENT: &str = concat!("protonsync/", env!("CARGO_PKG_VERSION"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_version_follows_protons_scheme() {
        let (name, version) = APP_VERSION.split_once('@').unwrap();
        assert_eq!(name, "external-drive-protonsync");
        let (semver, channel) = version.split_once('-').unwrap();
        assert_eq!(semver.split('.').count(), 3);
        assert!(["stable", "beta", "alpha"].contains(&channel));
    }
}
