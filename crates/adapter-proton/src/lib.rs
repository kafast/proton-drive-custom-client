//! Proton Drive connection (adapter for the ports `RemoteStorage` and `PhotoLibrary`).
//!
//! **The only crate that knows `proton-sdk` and `proton-drive-rs`** (ADR 0002).
//! It translates between the types of the community crates and the domain types.
//!
//! - [`auth`]: login with password and TOTP, saving and resuming the session.
//! - [`ProtonAccount`]: port `Account` (login, session, folder per sync pair).
//! - [`ProtonRemote`]: `RemoteStorage` on a folder in "My files".
//! - [`ProtonPhotos`]: `PhotoLibrary` on Proton Photos.

mod account;
pub mod auth;
mod convert;
mod photos;
mod remote;

pub use account::ProtonAccount;
pub use photos::ProtonPhotos;
pub use remote::ProtonRemote;

/// Honest identification as a third-party app in the `x-pm-appversion` header
/// (scheme `external-drive-{name}@{semver}-{channel}`, see CLAUDE.md).
pub const APP_VERSION: &str = concat!(
    "external-drive-protonsync@",
    env!("CARGO_PKG_VERSION"),
    "-alpha"
);

/// User agent of the HTTP requests.
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
