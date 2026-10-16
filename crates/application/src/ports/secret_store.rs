//! Port für Geheimnisse (Session-Tokens, abgeleitete Schlüssel).
//!
//! Desktop: Secret Service (GNOME Keyring/KWallet). Server: verschlüsselte Datei.

use thiserror::Error;

/// Name, unter dem ein Geheimnis abgelegt wird, z. B. `session`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SecretKey(String);

impl SecretKey {
    /// Erzeugt einen Schlüsselnamen.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Der Name als Text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Ein Geheimnis. `Debug` gibt den Inhalt absichtlich **nicht** aus,
/// damit er nie versehentlich in Logs landet.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(Vec<u8>);

impl Secret {
    /// Übernimmt die Bytes eines Geheimnisses.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Zugriff auf die Bytes. Nur dort aufrufen, wo der Wert wirklich gebraucht wird.
    #[must_use]
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<verborgen>)")
    }
}

/// Fehler beim Zugriff auf den Geheimnisspeicher.
#[derive(Debug, Error)]
pub enum SecretError {
    /// Der Speicher ist nicht erreichbar oder gesperrt.
    #[error("Geheimnisspeicher nicht verfügbar: {0}")]
    Unavailable(String),
}

/// Speichert und liest Geheimnisse.
pub trait SecretStore: Send + Sync {
    /// Liest ein Geheimnis; `Ok(None)`, wenn es keins gibt.
    ///
    /// # Errors
    /// [`SecretError`], wenn der Speicher nicht verfügbar ist.
    fn load(&self, key: &SecretKey) -> Result<Option<Secret>, SecretError>;

    /// Legt ein Geheimnis ab und ersetzt ein vorhandenes.
    ///
    /// # Errors
    /// [`SecretError`], wenn der Speicher nicht verfügbar ist.
    fn store(&self, key: &SecretKey, secret: &Secret) -> Result<(), SecretError>;

    /// Entfernt ein Geheimnis; kein Fehler, wenn es keins gab.
    ///
    /// # Errors
    /// [`SecretError`], wenn der Speicher nicht verfügbar ist.
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_hides_the_secret() {
        let secret = Secret::new(b"hunter2".to_vec());
        assert_eq!(format!("{secret:?}"), "Secret(<verborgen>)");
    }
}
