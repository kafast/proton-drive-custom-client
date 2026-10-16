//! Geheimnisspeicher (Adapter für den Port `SecretStore`).
//!
//! - [`EncryptedFileStore`]: für den Server. Jedes Geheimnis liegt als eigene,
//!   mit AES-256-GCM verschlüsselte Datei (Rechte 0600) im Datenverzeichnis.
//!   Der Schlüssel kommt aus systemd-Credentials, einer Datei oder einer
//!   Umgebungsvariable ([`MasterKey::from_environment`]).
//! - `KeyringStore` (Feature `keyring`): für den Desktop, im Secret Service
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

/// Erlaubte Zeichen in Namen von Geheimnissen. Die Namen werden zu Dateinamen,
/// deshalb keine Pfadtrenner oder Punkte.
fn validate_key(key: &SecretKey) -> Result<&str, SecretError> {
    let name = key.as_str();
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if valid {
        Ok(name)
    } else {
        Err(SecretError::Unavailable(format!(
            "ungültiger Name: {name:?}"
        )))
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
