//! Port für das Cloud-Konto: anmelden (mit 2FA), Sitzung fortsetzen, abmelden
//! und den entfernten Speicher eines Sync-Paares bzw. die Fotobibliothek öffnen.

use std::future::Future;

use protonsync_domain::path::RelativePath;
use thiserror::Error;

use super::{PhotoLibrary, RemoteError, RemoteStorage};

/// Fehler rund um das Konto.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AccountError {
    /// Nicht angemeldet oder Sitzung abgelaufen.
    #[error("Nicht angemeldet")]
    NotLoggedIn,
    /// Zugangsdaten oder Code abgelehnt.
    #[error("{0}")]
    Rejected(String),
    /// Schritt in falscher Reihenfolge.
    #[error("{0}")]
    WrongStep(String),
    /// Dienst oder Geheimnisspeicher nicht erreichbar.
    #[error("{0}")]
    Unavailable(String),
}

impl From<RemoteError> for AccountError {
    fn from(error: RemoteError) -> Self {
        match error {
            RemoteError::Unauthorized => Self::NotLoggedIn,
            other => Self::Unavailable(other.to_string()),
        }
    }
}

/// Was die Anmeldung als Nächstes braucht.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginProgress {
    /// Ein 2FA-Code (TOTP).
    pub needs_second_factor: bool,
    /// Ein eigenes Datenpasswort (Zwei-Passwort-Modus).
    pub needs_data_password: bool,
}

/// Das Konto. Alle Methoden dürfen parallel aufgerufen werden.
pub trait Account: Send + Sync {
    /// Entfernter Speicher eines Sync-Paares.
    type Remote: RemoteStorage;
    /// Die Fotobibliothek des Kontos.
    type Photos: PhotoLibrary;

    /// Setzt eine gespeicherte Sitzung fort; liefert den Benutzer oder `None`.
    fn resume(&self) -> impl Future<Output = Result<Option<String>, AccountError>> + Send;

    /// Beginnt eine Anmeldung.
    fn login_start(
        &self,
        username: &str,
        password: &str,
    ) -> impl Future<Output = Result<LoginProgress, AccountError>> + Send;

    /// Übermittelt den 2FA-Code.
    fn login_second_factor(
        &self,
        code: &str,
    ) -> impl Future<Output = Result<LoginProgress, AccountError>> + Send;

    /// Schließt die Anmeldung ab und speichert die Sitzung; liefert den Benutzer.
    /// Ohne `data_password` gilt das Login-Passwort.
    fn login_finish(
        &self,
        data_password: Option<&str>,
    ) -> impl Future<Output = Result<String, AccountError>> + Send;

    /// Meldet ab und löscht die gespeicherte Sitzung.
    fn logout(&self) -> impl Future<Output = Result<(), AccountError>> + Send;

    /// Angemeldeter Benutzer.
    fn username(&self) -> impl Future<Output = Option<String>> + Send;

    /// Ob eine Anmeldung begonnen, aber nicht abgeschlossen ist.
    fn login_pending(&self) -> impl Future<Output = bool> + Send;

    /// Öffnet den Ordner `remote_path` unterhalb der Ablage des Kontos und legt
    /// ihn bei Bedarf an.
    fn open_remote(
        &self,
        remote_path: &RelativePath,
    ) -> impl Future<Output = Result<Self::Remote, AccountError>> + Send;

    /// Öffnet die Fotobibliothek (Proton Photos).
    fn open_photos(&self) -> impl Future<Output = Result<Self::Photos, AccountError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_errors_map_to_account_errors() {
        assert_eq!(
            AccountError::from(RemoteError::Unauthorized),
            AccountError::NotLoggedIn
        );
        assert!(matches!(
            AccountError::from(RemoteError::Unavailable("Netz".into())),
            AccountError::Unavailable(_)
        ));
    }
}
