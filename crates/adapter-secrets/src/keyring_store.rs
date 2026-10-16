//! Geheimnisse im Secret Service des Desktops (GNOME Keyring, `KWallet`).
//!
//! Die Aufrufe blockieren kurz (D-Bus); im Daemon deshalb aus
//! `tokio::task::spawn_blocking` heraus verwenden.

use keyring::{Entry, Error};
use protonsync_application::ports::{Secret, SecretError, SecretKey, SecretStore};

use crate::validate_key;

/// Dienstname, unter dem die Einträge im Schlüsselbund erscheinen.
const SERVICE: &str = "protonsync";

/// Geheimnisspeicher über den Secret Service.
#[derive(Debug, Default)]
pub struct KeyringStore;

impl KeyringStore {
    /// Erzeugt den Speicher. Ob der Secret Service erreichbar ist, zeigt erst
    /// der erste Zugriff.
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
        entry(key)?.set_secret(secret.expose()).map_err(|e| unavailable(&e))
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

    /// Braucht einen laufenden Secret Service (Desktop-Sitzung).
    #[test]
    #[ignore = "Live-Test: braucht GNOME Keyring oder KWallet"]
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
