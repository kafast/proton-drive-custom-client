//! Port for secrets (session tokens, derived keys).
//!
//! Desktop: Secret Service (GNOME Keyring/KWallet). Server: encrypted file.

use thiserror::Error;

/// Name under which a secret is stored, e.g. `session`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SecretKey(String);

impl SecretKey {
    /// Creates a key name.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The name as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A secret. `Debug` deliberately does **not** print the content,
/// so that it never ends up in logs by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(Vec<u8>);

impl Secret {
    /// Takes over the bytes of a secret.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Access to the bytes. Only call it where the value is really needed.
    #[must_use]
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<hidden>)")
    }
}

/// Error when accessing the secret store.
#[derive(Debug, Error)]
pub enum SecretError {
    /// The store is unreachable or locked.
    #[error("secret store unavailable: {0}")]
    Unavailable(String),
}

/// Stores and reads secrets.
pub trait SecretStore: Send + Sync {
    /// Reads a secret; `Ok(None)` if there is none.
    ///
    /// # Errors
    /// [`SecretError`] if the store is unavailable.
    fn load(&self, key: &SecretKey) -> Result<Option<Secret>, SecretError>;

    /// Stores a secret, replacing an existing one.
    ///
    /// # Errors
    /// [`SecretError`] if the store is unavailable.
    fn store(&self, key: &SecretKey, secret: &Secret) -> Result<(), SecretError>;

    /// Removes a secret; not an error if there was none.
    ///
    /// # Errors
    /// [`SecretError`] if the store is unavailable.
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_hides_the_secret() {
        let secret = Secret::new(b"hunter2".to_vec());
        assert_eq!(format!("{secret:?}"), "Secret(<hidden>)");
    }
}
