//! Secret store (adapter for the port `SecretStore`).
//!
//! - [`EncryptedFileStore`]: for the server. Each secret is stored as its own
//!   AES-256-GCM encrypted file (permissions 0600) in the data directory.
//!   The key comes from systemd credentials, a file or an
//!   environment variable ([`MasterKey::from_environment`]).
//! - `KeyringStore` (feature `keyring`): for the desktop, in the Secret Service
//!   (GNOME Keyring, `KWallet`).

mod file_store;
#[cfg(feature = "keyring")]
mod keyring_store;
mod master_key;

pub use file_store::EncryptedFileStore;
#[cfg(feature = "keyring")]
pub use keyring_store::KeyringStore;
pub use master_key::{KEY_CREDENTIAL_NAME, KEY_ENV, KEY_FILE_ENV, KeyError, MasterKey};

use protonsync_application::ports::{SecretError, SecretKey};

/// Allowed characters in secret names. The names become file names,
/// so no path separators or dots.
fn validate_key(key: &SecretKey) -> Result<&str, SecretError> {
    let name = key.as_str();
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if valid {
        Ok(name)
    } else {
        Err(SecretError::Unavailable(format!("invalid name: {name:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_simple_names_are_accepted() {
        assert!(validate_key(&SecretKey::new("proton-session")).is_ok());
        for bad in ["", "../x", "a.b", "A", "a/b"] {
            assert!(validate_key(&SecretKey::new(bad)).is_err(), "{bad:?}");
        }
    }
}
