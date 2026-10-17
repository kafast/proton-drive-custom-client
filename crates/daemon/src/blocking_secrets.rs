//! Secret stores that may block (Secret Service over D-Bus).
//!
//! The port [`SecretStore`] is synchronous. If an `async` task calls it on
//! a Tokio worker thread, all other tasks on that thread wait as well until
//! D-Bus answers (on first access the keyring may ask for its password).
//! [`BlockingSecrets`] therefore announces such calls with
//! `block_in_place`: Tokio then moves the remaining tasks to other
//! threads.

use protonsync_application::ports::{Secret, SecretError, SecretKey, SecretStore};
use tokio::runtime::{Handle, RuntimeFlavor};

/// Wrapper around a store whose calls may block for a while.
pub struct BlockingSecrets<S> {
    inner: S,
}

impl<S> BlockingSecrets<S> {
    /// Wraps `inner`.
    pub fn new(inner: S) -> Self {
        Self { inner }
    }
}

/// Runs `call`; on a worker thread of the multi-thread runtime via
/// `block_in_place`. Outside Tokio (or in the single-thread runtime of the
/// tests, where `block_in_place` is not allowed) directly.
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
        store.store(&key, &Secret::new(b"secret".to_vec())).unwrap();
        assert_eq!(store.load(&key).unwrap().unwrap().expose(), b"secret");
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
