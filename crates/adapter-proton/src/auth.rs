//! Anmeldung bei Proton und Fortsetzen einer gespeicherten Sitzung.
//!
//! Ablauf beim ersten Login (CLI `protonsync login`):
//! 1. [`PendingLogin::start`] mit Benutzername und Passwort (SRP, das Passwort
//!    verlässt den Rechner nicht im Klartext).
//! 2. Falls verlangt: [`PendingLogin::submit_totp`] mit dem 2FA-Code.
//! 3. [`PendingLogin::finish`] mit dem Datenpasswort (im Ein-Passwort-Modus ist
//!    das das Login-Passwort). Ergebnis: ein Drive-Client und eine
//!    [`StoredSession`] für den Geheimnisspeicher.
//!
//! Gespeichert werden **nie** Passwörter, sondern Tokens und die daraus
//! abgeleiteten Schlüssel-Passphrasen. Die sind genauso schutzwürdig wie das
//! Passwort und gehören deshalb nur in den `SecretStore`.

use std::fmt;
use std::sync::{Arc, Mutex};

use proton_drive_rs::ProtonDriveClient;
use proton_drive_rs::proton_sdk::account::KeyPassphrases;
use proton_drive_rs::proton_sdk::config::ProtonClientConfiguration;
use proton_drive_rs::proton_sdk::error::ProtonError;
use proton_drive_rs::proton_sdk::ids::{SessionId, UserId};
use proton_drive_rs::proton_sdk::session::{PasswordMode, ProtonApiSession, ResumeParameters};
use protonsync_application::ports::{Secret, SecretError, SecretKey, SecretStore};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{APP_VERSION, USER_AGENT};

/// Name, unter dem die Sitzung im Geheimnisspeicher liegt.
pub const SESSION_SECRET_KEY: &str = "proton-session";

/// Version des gespeicherten Formats; bei Änderungen erhöhen.
const STORED_SESSION_FORMAT: u32 = 1;

/// Fehler bei Anmeldung oder Fortsetzen.
#[derive(Debug, Error)]
pub enum AuthError {
    /// Benutzername, Passwort oder 2FA-Code wurden abgelehnt.
    #[error("Anmeldung abgelehnt: {0}")]
    Rejected(String),
    /// Das Konto verlangt einen Sicherheitsschlüssel (FIDO2); das unterstützt protonsync nicht.
    #[error(
        "Das Konto verlangt einen Sicherheitsschlüssel (FIDO2); bitte zusätzlich TOTP aktivieren"
    )]
    SecurityKeyOnly,
    /// Ein Schritt wurde in falscher Reihenfolge aufgerufen.
    #[error("Falscher Anmeldeschritt: {0}")]
    WrongStep(&'static str),
    /// Proton ist nicht erreichbar oder hat einen technischen Fehler gemeldet.
    #[error("Proton nicht erreichbar: {0}")]
    Unavailable(String),
    /// Es gibt keine gespeicherte Sitzung; `protonsync login` ist nötig.
    #[error("Keine gespeicherte Sitzung, bitte zuerst anmelden")]
    NotLoggedIn,
    /// Die gespeicherte Sitzung ist unlesbar oder hat ein unbekanntes Format.
    #[error("Gespeicherte Sitzung beschädigt: {0}")]
    CorruptSession(String),
    /// Der Geheimnisspeicher ist nicht verfügbar.
    #[error(transparent)]
    Secrets(#[from] SecretError),
}

impl From<ProtonError> for AuthError {
    fn from(error: ProtonError) -> Self {
        // Fehlermeldungen von Proton enthalten keine Geheimnisse, nur Codes und Texte.
        match &error {
            ProtonError::Api(api) if (400..500).contains(&api.http_status) => {
                Self::Rejected(error.to_string())
            }
            _ => Self::Unavailable(error.to_string()),
        }
    }
}

/// Konfiguration aller Anfragen: ehrliche Kennung als Drittanbieter-App.
fn client_configuration() -> ProtonClientConfiguration {
    ProtonClientConfiguration::new(APP_VERSION).with_user_agent(USER_AGENT)
}

/// Eine begonnene Anmeldung (nach Passwort, ggf. vor dem 2FA-Code).
pub struct PendingLogin {
    session: ProtonApiSession,
}

impl PendingLogin {
    /// Startet die Anmeldung mit Benutzername und Passwort.
    ///
    /// # Errors
    /// [`AuthError::Rejected`] bei falschen Zugangsdaten, sonst
    /// [`AuthError::Unavailable`].
    pub async fn start(username: &str, password: &[u8]) -> Result<Self, AuthError> {
        let session = ProtonApiSession::begin(client_configuration(), username, password).await?;
        let methods = session.second_factor_methods();
        if session.is_waiting_for_second_factor() && methods.fido2 && !methods.totp {
            return Err(AuthError::SecurityKeyOnly);
        }
        Ok(Self { session })
    }

    /// Ob als Nächstes ein TOTP-Code nötig ist.
    #[must_use]
    pub fn needs_second_factor(&self) -> bool {
        self.session.is_waiting_for_second_factor()
    }

    /// Ob das Konto ein eigenes Datenpasswort (Zwei-Passwort-Modus) hat.
    #[must_use]
    pub fn needs_data_password(&self) -> bool {
        self.session.password_mode() == PasswordMode::Dual
    }

    /// Übermittelt den TOTP-Code der Authenticator-App.
    ///
    /// # Errors
    /// [`AuthError::WrongStep`], wenn kein Code verlangt war;
    /// [`AuthError::Rejected`] bei falschem Code.
    pub async fn submit_totp(&mut self, code: &str) -> Result<(), AuthError> {
        if !self.needs_second_factor() {
            return Err(AuthError::WrongStep("kein 2FA-Code verlangt"));
        }
        self.session.apply_second_factor_code(code.trim()).await?;
        Ok(())
    }

    /// Schließt die Anmeldung ab und entsperrt die Schlüssel.
    ///
    /// `data_password` ist im Zwei-Passwort-Modus das Mailbox-Passwort,
    /// sonst noch einmal das Login-Passwort.
    ///
    /// # Errors
    /// [`AuthError::WrongStep`], solange der 2FA-Code fehlt;
    /// [`AuthError::Rejected`], wenn das Passwort die Schlüssel nicht entsperrt.
    pub async fn finish(
        self,
        data_password: &[u8],
    ) -> Result<(ProtonDriveClient, StoredSession), AuthError> {
        if self.needs_second_factor() {
            return Err(AuthError::WrongStep("2FA-Code fehlt"));
        }
        let unlocking_client = ProtonDriveClient::new(&self.session, data_password);
        let passphrases = unlocking_client.account().key_passphrases().await?;
        if passphrases.is_empty() {
            return Err(AuthError::Rejected("keine Schlüssel entsperrt".into()));
        }
        let stored = StoredSession::capture(&self.session, &passphrases).await;
        // Ab hier arbeitet der Client ohne Passwort, genau wie nach dem Fortsetzen.
        let client = ProtonDriveClient::with_key_passphrases(&self.session, passphrases);
        Ok((client, stored))
    }
}

/// Alles, was zum Fortsetzen einer Sitzung ohne Passwort nötig ist.
///
/// `Debug` zeigt absichtlich nur den Benutzernamen.
#[derive(Clone, Serialize, Deserialize)]
pub struct StoredSession {
    format: u32,
    session_id: String,
    username: String,
    user_id: String,
    access_token: String,
    refresh_token: String,
    scopes: Vec<String>,
    dual_password: bool,
    /// Schlüssel-ID → Passphrase (hexadezimal).
    key_passphrases: Vec<(String, String)>,
}

impl fmt::Debug for StoredSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredSession")
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

impl StoredSession {
    async fn capture(session: &ProtonApiSession, passphrases: &KeyPassphrases) -> Self {
        let tokens = session.current_tokens().await;
        Self {
            format: STORED_SESSION_FORMAT,
            session_id: session.session_id().as_str().to_owned(),
            username: session.username().to_owned(),
            user_id: session.user_id().as_str().to_owned(),
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            scopes: session.scopes().to_vec(),
            dual_password: session.password_mode() == PasswordMode::Dual,
            key_passphrases: passphrases
                .iter()
                .map(|(id, passphrase)| (id.to_owned(), hex::encode(passphrase)))
                .collect(),
        }
    }

    /// Der angemeldete Benutzer (für Statusanzeigen).
    #[must_use]
    pub fn username(&self) -> &str {
        &self.username
    }

    /// Serialisiert die Sitzung als Geheimnis.
    ///
    /// # Errors
    /// [`AuthError::CorruptSession`], falls die Serialisierung scheitert.
    pub fn to_secret(&self) -> Result<Secret, AuthError> {
        serde_json::to_vec(self)
            .map(Secret::new)
            .map_err(|e| AuthError::CorruptSession(e.to_string()))
    }

    /// Liest eine Sitzung aus einem Geheimnis.
    ///
    /// # Errors
    /// [`AuthError::CorruptSession`] bei unlesbarem Inhalt oder unbekanntem Format.
    pub fn from_secret(secret: &Secret) -> Result<Self, AuthError> {
        let stored: Self = serde_json::from_slice(secret.expose())
            // Die Meldung von serde_json enthält nur Positionen, keine Inhalte.
            .map_err(|e| AuthError::CorruptSession(e.to_string()))?;
        if stored.format != STORED_SESSION_FORMAT {
            return Err(AuthError::CorruptSession(format!(
                "Format {}",
                stored.format
            )));
        }
        Ok(stored)
    }

    fn passphrases(&self) -> Result<KeyPassphrases, AuthError> {
        let mut passphrases = KeyPassphrases::new();
        for (id, encoded) in &self.key_passphrases {
            let bytes = hex::decode(encoded)
                .ok_or_else(|| AuthError::CorruptSession("Passphrase".into()))?;
            passphrases.insert(id.clone(), bytes);
        }
        Ok(passphrases)
    }

    fn resume_parameters(&self) -> ResumeParameters {
        ResumeParameters {
            session_id: SessionId::new(self.session_id.clone()),
            username: self.username.clone(),
            user_id: UserId::new(self.user_id.clone()),
            access_token: self.access_token.clone(),
            refresh_token: self.refresh_token.clone(),
            scopes: self.scopes.clone(),
            is_waiting_for_second_factor_code: false,
            password_mode: if self.dual_password {
                PasswordMode::Dual
            } else {
                PasswordMode::Single
            },
        }
    }
}

/// Legt die Sitzung im Geheimnisspeicher ab.
///
/// # Errors
/// [`AuthError::Secrets`], wenn der Speicher nicht verfügbar ist.
pub fn save_session<S: SecretStore + ?Sized>(
    store: &S,
    session: &StoredSession,
) -> Result<(), AuthError> {
    store.store(&SecretKey::new(SESSION_SECRET_KEY), &session.to_secret()?)?;
    Ok(())
}

/// Liest die gespeicherte Sitzung.
///
/// # Errors
/// [`AuthError::NotLoggedIn`], wenn keine da ist; sonst Speicher- oder Formatfehler.
pub fn load_session<S: SecretStore + ?Sized>(store: &S) -> Result<StoredSession, AuthError> {
    let secret = store
        .load(&SecretKey::new(SESSION_SECRET_KEY))?
        .ok_or(AuthError::NotLoggedIn)?;
    StoredSession::from_secret(&secret)
}

/// Setzt eine gespeicherte Sitzung fort, ohne Passwort.
///
/// Erneuert Proton die Tokens, wird die Sitzung automatisch im `store`
/// aktualisiert, damit der nächste Start mit gültigen Tokens beginnt.
///
/// # Errors
/// [`AuthError::NotLoggedIn`] ohne gespeicherte Sitzung, sonst Format- oder
/// Verbindungsfehler.
pub fn resume<S: SecretStore + 'static>(store: Arc<S>) -> Result<ProtonDriveClient, AuthError> {
    let stored = load_session(store.as_ref())?;
    let passphrases = stored.passphrases()?;
    let session = ProtonApiSession::resume(client_configuration(), stored.resume_parameters())?;
    persist_refreshed_tokens(&session, stored, store);
    Ok(ProtonDriveClient::with_key_passphrases(
        &session,
        passphrases,
    ))
}

/// Meldet die Sitzung bei Proton ab und löscht sie aus dem Speicher.
///
/// Auch wenn Proton nicht erreichbar ist, wird lokal gelöscht.
///
/// # Errors
/// [`AuthError::Secrets`], wenn das lokale Löschen scheitert.
pub async fn logout<S: SecretStore + ?Sized>(store: &S) -> Result<(), AuthError> {
    if let Ok(stored) = load_session(store)
        && let Ok(session) =
            ProtonApiSession::resume(client_configuration(), stored.resume_parameters())
    {
        // Ein Fehler hier (z. B. offline) verhindert das lokale Abmelden nicht.
        let _ = session.end().await;
    }
    store.delete(&SecretKey::new(SESSION_SECRET_KEY))?;
    Ok(())
}

/// Hängt einen Rückruf an, der erneuerte Tokens sofort speichert.
fn persist_refreshed_tokens<S: SecretStore + 'static>(
    session: &ProtonApiSession,
    stored: StoredSession,
    store: Arc<S>,
) {
    // Der Rückruf läuft später auf einem anderen Thread; deshalb gehört ihm
    // eine eigene Kopie der Sitzung hinter einem Mutex (`move`).
    let stored = Mutex::new(stored);
    session.http().set_on_tokens_refreshed(move |tokens| {
        let Ok(mut current) = stored.lock() else {
            return;
        };
        current.access_token = tokens.access_token;
        current.refresh_token = tokens.refresh_token;
        // Ein Speicherfehler darf die laufende Anfrage nicht abbrechen; der
        // nächste Refresh versucht es erneut.
        let _ = save_session(store.as_ref(), &current);
    });
}

/// Minimale Hex-Kodierung, damit dafür keine weitere Abhängigkeit nötig ist.
mod hex {
    use std::fmt::Write;

    pub fn encode(bytes: &[u8]) -> String {
        bytes
            .iter()
            .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
                let _ = write!(out, "{b:02x}");
                out
            })
    }

    pub fn decode(text: &str) -> Option<Vec<u8>> {
        if !text.len().is_multiple_of(2) {
            return None;
        }
        (0..text.len())
            .step_by(2)
            .map(|i| {
                text.get(i..i + 2)
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protonsync_testkit::InMemorySecretStore;

    fn sample() -> StoredSession {
        StoredSession {
            format: STORED_SESSION_FORMAT,
            session_id: "sid".into(),
            username: "krischan".into(),
            user_id: "uid".into(),
            access_token: "access-geheim".into(),
            refresh_token: "refresh-geheim".into(),
            scopes: vec!["full".into()],
            dual_password: false,
            key_passphrases: vec![("key1".into(), hex::encode(b"passphrase"))],
        }
    }

    #[test]
    fn hex_round_trips() {
        let bytes = [0u8, 1, 0xab, 0xff];
        assert_eq!(hex::encode(&bytes), "0001abff");
        assert_eq!(hex::decode("0001abff"), Some(bytes.to_vec()));
        assert_eq!(hex::decode("0g"), None);
        assert_eq!(hex::decode("abc"), None);
    }

    #[test]
    fn debug_hides_tokens() {
        let text = format!("{:?}", sample());
        assert!(text.contains("krischan"));
        assert!(!text.contains("geheim"));
        assert!(!text.contains("706173"));
    }

    #[test]
    fn session_round_trips_through_store() {
        let store = InMemorySecretStore::default();
        save_session(&store, &sample()).unwrap();
        let loaded = load_session(&store).unwrap();
        assert_eq!(loaded.username(), "krischan");
        assert_eq!(
            loaded.passphrases().unwrap().get("key1"),
            Some(&b"passphrase"[..])
        );
        assert!(matches!(
            loaded.resume_parameters().password_mode,
            PasswordMode::Single
        ));
    }

    #[test]
    fn missing_session_means_not_logged_in() {
        let store = InMemorySecretStore::default();
        assert!(matches!(load_session(&store), Err(AuthError::NotLoggedIn)));
    }

    #[test]
    fn unknown_format_is_rejected() {
        let mut stored = sample();
        stored.format = 99;
        let secret = stored.to_secret().unwrap();
        assert!(matches!(
            StoredSession::from_secret(&secret),
            Err(AuthError::CorruptSession(_))
        ));
        let garbage = Secret::new(b"kein json".to_vec());
        assert!(matches!(
            StoredSession::from_secret(&garbage),
            Err(AuthError::CorruptSession(_))
        ));
    }

    #[test]
    fn resume_without_session_fails_cleanly() {
        let store = Arc::new(InMemorySecretStore::default());
        assert!(matches!(resume(store), Err(AuthError::NotLoggedIn)));
    }
}
