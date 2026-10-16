//! Geheimnisspeicher, die blockieren dürfen (Secret Service über D-Bus).
//!
//! Der Port [`SecretStore`] ist synchron. Ruft eine `async`-Aufgabe ihn auf
//! einem Tokio-Arbeitsthread auf, warten alle anderen Aufgaben dieses Threads
//! mit, bis D-Bus antwortet (beim ersten Zugriff fragt der Schlüsselbund
//! womöglich nach seinem Passwort). [`BlockingSecrets`] meldet solche
//! Aufrufe deshalb mit `block_in_place` an: Tokio verlagert die übrigen
//! Aufgaben dann auf andere Threads.

use protonsync_application::ports::{Secret, SecretError, SecretKey, SecretStore};
use tokio::runtime::{Handle, RuntimeFlavor};

/// Hülle um einen Speicher, dessen Aufrufe länger blockieren können.
pub struct BlockingSecrets<S> {
    inner: S,
}

impl<S> BlockingSecrets<S> {
    /// Umhüllt `inner`.
    pub fn new(inner: S) -> Self {
        Self { inner }
    }
}

/// Führt `call` aus; auf einem Arbeitsthread der Mehrthread-Laufzeit über
/// `block_in_place`. Außerhalb von Tokio (oder in der Einthread-Laufzeit der
/// Tests, wo `block_in_place` nicht erlaubt ist) direkt.
fn blocking<T>(call: impl FnOnce() -> T) -> T {
    match Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(call)
        }
        _ => call(),
    }
}

impl<S: SecretStore> SecretStore for BlockingSecrets<S> {
    fn load(&self, key: &SecretKey) -> Result<Option<Secret>, SecretError> {
        blocking(|| self.inner.load(key))
    }

    fn store(&self, key: &SecretKey, secret: &Secret) -> Result<(), SecretError> {
        blocking(|| self.inner.store(key, secret))
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        blocking(|| self.inner.delete(key))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use protonsync_testkit::InMemorySecretStore;

    fn round_trip(store: &BlockingSecrets<InMemorySecretStore>) {
        let key = SecretKey::new("session");
        store.store(&key, &Secret::new(b"geheim".to_vec())).unwrap();
        assert_eq!(store.load(&key).unwrap().unwrap().expose(), b"geheim");
        store.delete(&key).unwrap();
        assert!(store.load(&key).unwrap().is_none());
    }

    #[test]
    fn works_outside_tokio() {
        round_trip(&BlockingSecrets::new(InMemorySecretStore::default()));
    }

    #[tokio::test]
    async fn works_in_the_single_thread_runtime() {
        round_trip(&BlockingSecrets::new(InMemorySecretStore::default()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn works_on_a_worker_thread() {
        round_trip(&BlockingSecrets::new(InMemorySecretStore::default()));
    }
}
