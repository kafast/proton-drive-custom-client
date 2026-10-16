//! Secrets in the desktop's Secret Service (GNOME Keyring, `KWallet`).
//!
//! The calls block briefly (D-Bus); so in the daemon, use them from
//! within `tokio::task::spawn_blocking`.

use keyring::{Entry, Error};
use protonsync_application::ports::{Secret, SecretError, SecretKey, SecretStore};

use crate::validate_key;

/// Service name under which the entries appear in the keyring.
const SERVICE: &str = "protonsync";

/// Secret store backed by the Secret Service.
#[derive(Debug, Default)]
pub struct KeyringStore;

impl KeyringStore {
    /// Creates the store. Whether the Secret Service is reachable only shows
    /// on the first access.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

fn entry(key: &SecretKey) -> Result<Entry, SecretError> {
    Entry::new(SERVICE, validate_key(key)?).map_err(|e| unavailable(&e))
}

fn unavailable(error: &Error) -> SecretError {
    SecretError::Unavailable(format!("Secret Service: {error}"))
}

impl SecretStore for KeyringStore {
    fn load(&self, key: &SecretKey) -> Result<Option<Secret>, SecretError> {
        match entry(key)?.get_secret() {
            Ok(bytes) => Ok(Some(Secret::new(bytes))),
            Err(Error::NoEntry) => Ok(None),
            Err(error) => Err(unavailable(&error)),
        }
    }

    fn store(&self, key: &SecretKey, secret: &Secret) -> Result<(), SecretError> {
        entry(key)?
            .set_secret(secret.expose())
            .map_err(|e| unavailable(&e))
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        match entry(key)?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(error) => Err(unavailable(&error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs a running Secret Service (desktop session).
    #[test]
    #[ignore = "live test: needs GNOME Keyring or KWallet"]
    fn round_trips_through_secret_service() {
        let store = KeyringStore::new();
        let key = SecretKey::new("protonsync-live-test");
        store.store(&key, &Secret::new(b"x".to_vec())).unwrap();
        assert_eq!(store.load(&key).unwrap(), Some(Secret::new(b"x".to_vec())));
        store.delete(&key).unwrap();
        assert_eq!(store.load(&key).unwrap(), None);
    }

    #[test]
    fn invalid_names_fail_before_dbus() {
        let store = KeyringStore::new();
        assert!(store.load(&SecretKey::new("../x")).is_err());
    }
}
