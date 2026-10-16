use std::collections::HashMap;
use std::sync::Mutex;

use protonsync_application::ports::{Secret, SecretError, SecretKey, SecretStore};

/// In-memory secret store, for tests only.
#[derive(Debug, Default)]
pub struct InMemorySecretStore {
    entries: Mutex<HashMap<SecretKey, Secret>>,
}

impl InMemorySecretStore {
    fn entries(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, HashMap<SecretKey, Secret>>, SecretError> {
        // A "poisoned" mutex only happens when another thread panicked
        // while holding the lock.
        self.entries
            .lock()
            .map_err(|_| SecretError::Unavailable("mutex poisoned".into()))
    }
}

impl SecretStore for InMemorySecretStore {
    fn load(&self, key: &SecretKey) -> Result<Option<Secret>, SecretError> {
        Ok(self.entries()?.get(key).cloned())
    }

    fn store(&self, key: &SecretKey, secret: &Secret) -> Result<(), SecretError> {
        self.entries()?.insert(key.clone(), secret.clone());
        Ok(())
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.entries()?.remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_load_delete_roundtrip() {
        let store = InMemorySecretStore::default();
        let key = SecretKey::new("session");
        assert_eq!(store.load(&key).unwrap(), None);
        store.store(&key, &Secret::new(vec![1, 2])).unwrap();
        assert_eq!(store.load(&key).unwrap(), Some(Secret::new(vec![1, 2])));
        store.delete(&key).unwrap();
        assert_eq!(store.load(&key).unwrap(), None);
    }
}
