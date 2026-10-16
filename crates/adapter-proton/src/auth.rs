//! Logging in to Proton and resuming a saved session.
//!
//! Flow of the first login (CLI `protonsync login`):
//! 1. [`PendingLogin::start`] with username and password (SRP, the password
//!    never leaves the machine in plain text).
//! 2. If requested: [`PendingLogin::submit_totp`] with the 2FA code.
//! 3. [`PendingLogin::finish`] with the data password (in single-password mode
//!    this is the login password). Result: a Drive client and a
//!    [`StoredSession`] for the secret store.
//!
//! Passwords are **never** saved, only tokens and the key passphrases derived
//! from them. Those need the same protection as the password and therefore
//! belong only in the `SecretStore`.

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

/// Name under which the session is kept in the secret store.
pub const SESSION_SECRET_KEY: &str = "proton-session";

/// Version of the stored format; increase it on changes.
const STORED_SESSION_FORMAT: u32 = 1;

/// Errors during login or resume.
#[derive(Debug, Error)]
pub enum AuthError {
    /// Username, password or 2FA code were rejected.
    #[error("login rejected: {0}")]
    Rejected(String),
    /// The account requires a security key (FIDO2); protonsync does not support that.
    #[error("the account requires a security key (FIDO2); please enable TOTP as well")]
    SecurityKeyOnly,
    /// A step was called in the wrong order.
    #[error("wrong login step: {0}")]
    WrongStep(&'static str),
    /// Proton is unreachable or reported a technical error.
    #[error("Proton unreachable: {0}")]
    Unavailable(String),
    /// There is no saved session; `protonsync login` is required.
    #[error("no saved session, please log in first")]
    NotLoggedIn,
    /// The saved session is unreadable or has an unknown format.
    #[error("saved session corrupt: {0}")]
    CorruptSession(String),
    /// The secret store is not available.
    #[error(transparent)]
    Secrets(#[from] SecretError),
}

impl From<ProtonError> for AuthError {
    fn from(error: ProtonError) -> Self {
        // Error messages from Proton contain no secrets, only codes and texts.
        match &error {
            ProtonError::Api(api) if (400..500).contains(&api.http_status) => {
                Self::Rejected(error.to_string())
            }
            _ => Self::Unavailable(error.to_string()),
        }
    }
}

/// Configuration of all requests: honest identification as a third-party app.
fn client_configuration() -> ProtonClientConfiguration {
    ProtonClientConfiguration::new(APP_VERSION).with_user_agent(USER_AGENT)
}

/// A started login (after the password, possibly before the 2FA code).
pub struct PendingLogin {
    session: ProtonApiSession,
}

impl PendingLogin {
    /// Starts the login with username and password.
    ///
    /// # Errors
    /// [`AuthError::Rejected`] for wrong credentials, otherwise
    /// [`AuthError::Unavailable`].
    pub async fn start(username: &str, password: &[u8]) -> Result<Self, AuthError> {
        let session = ProtonApiSession::begin(client_configuration(), username, password).await?;
        let methods = session.second_factor_methods();
        if session.is_waiting_for_second_factor() && methods.fido2 && !methods.totp {
            return Err(AuthError::SecurityKeyOnly);
        }
        Ok(Self { session })
    }

    /// Whether a TOTP code is needed next.
    #[must_use]
    pub fn needs_second_factor(&self) -> bool {
        self.session.is_waiting_for_second_factor()
    }

    /// Whether the account has a separate data password (two-password mode).
    #[must_use]
    pub fn needs_data_password(&self) -> bool {
        self.session.password_mode() == PasswordMode::Dual
    }

    /// Submits the TOTP code from the authenticator app.
    ///
    /// # Errors
    /// [`AuthError::WrongStep`] if no code was requested;
    /// [`AuthError::Rejected`] for a wrong code.
    pub async fn submit_totp(&mut self, code: &str) -> Result<(), AuthError> {
        if !self.needs_second_factor() {
            return Err(AuthError::WrongStep("no 2FA code requested"));
        }
        self.session.apply_second_factor_code(code.trim()).await?;
        Ok(())
    }

    /// Completes the login and unlocks the keys.
    ///
    /// `data_password` is the mailbox password in two-password mode,
    /// otherwise the login password once more.
    ///
    /// # Errors
    /// [`AuthError::WrongStep`] while the 2FA code is missing;
    /// [`AuthError::Rejected`] if the password does not unlock the keys.
    pub async fn finish(
        self,
        data_password: &[u8],
    ) -> Result<(ProtonDriveClient, StoredSession), AuthError> {
        if self.needs_second_factor() {
            return Err(AuthError::WrongStep("2FA code missing"));
        }
        let unlocking_client = ProtonDriveClient::new(&self.session, data_password);
        let passphrases = unlocking_client.account().key_passphrases().await?;
        if passphrases.is_empty() {
            return Err(AuthError::Rejected("no keys unlocked".into()));
        }
        let stored = StoredSession::capture(&self.session, &passphrases).await;
        // From here on the client works without a password, just like after a resume.
        let client = ProtonDriveClient::with_key_passphrases(&self.session, passphrases);
        Ok((client, stored))
    }
}

/// Everything needed to resume a session without a password.
///
/// `Debug` deliberately shows only the username.
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
    /// Key ID → passphrase (hexadecimal).
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

    /// The logged-in user (for status displays).
    #[must_use]
    pub fn username(&self) -> &str {
        &self.username
    }

    /// Serializes the session as a secret.
    ///
    /// # Errors
    /// [`AuthError::CorruptSession`] if serialization fails.
    pub fn to_secret(&self) -> Result<Secret, AuthError> {
        serde_json::to_vec(self)
            .map(Secret::new)
            .map_err(|e| AuthError::CorruptSession(e.to_string()))
    }

    /// Reads a session from a secret.
    ///
    /// # Errors
    /// [`AuthError::CorruptSession`] for unreadable content or an unknown format.
    pub fn from_secret(secret: &Secret) -> Result<Self, AuthError> {
        let stored: Self = serde_json::from_slice(secret.expose())
            // The serde_json message contains only positions, no contents.
            .map_err(|e| AuthError::CorruptSession(e.to_string()))?;
        if stored.format != STORED_SESSION_FORMAT {
            return Err(AuthError::CorruptSession(format!(
                "format {}",
                stored.format
            )));
        }
        Ok(stored)
    }

    fn passphrases(&self) -> Result<KeyPassphrases, AuthError> {
        let mut passphrases = KeyPassphrases::new();
        for (id, encoded) in &self.key_passphrases {
            let bytes = hex::decode(encoded)
                .ok_or_else(|| AuthError::CorruptSession("passphrase".into()))?;
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

/// Saves the session in the secret store.
///
/// # Errors
/// [`AuthError::Secrets`] if the store is not available.
pub fn save_session<S: SecretStore + ?Sized>(
    store: &S,
    session: &StoredSession,
) -> Result<(), AuthError> {
    store.store(&SecretKey::new(SESSION_SECRET_KEY), &session.to_secret()?)?;
    Ok(())
}

/// Reads the saved session.
///
/// # Errors
/// [`AuthError::NotLoggedIn`] if there is none; otherwise store or format errors.
pub fn load_session<S: SecretStore + ?Sized>(store: &S) -> Result<StoredSession, AuthError> {
    let secret = store
        .load(&SecretKey::new(SESSION_SECRET_KEY))?
        .ok_or(AuthError::NotLoggedIn)?;
    StoredSession::from_secret(&secret)
}

/// Resumes a saved session, without a password.
///
/// When Proton refreshes the tokens, the session is updated in `store`
/// automatically, so that the next start begins with valid tokens.
///
/// # Errors
/// [`AuthError::NotLoggedIn`] without a saved session, otherwise format or
/// connection errors.
pub fn resume<S: SecretStore + ?Sized + 'static>(
    store: Arc<S>,
) -> Result<ProtonDriveClient, AuthError> {
    let stored = load_session(store.as_ref())?;
    let (session, client) = open_session(&stored)?;
    persist_refreshed_tokens(&session, stored, store);
    Ok(client)
}

/// Resumes a session without saving refreshed tokens (for tests).
///
/// # Errors
/// Format or connection errors.
pub fn resume_without_store(stored: &StoredSession) -> Result<ProtonDriveClient, AuthError> {
    open_session(stored).map(|(_, client)| client)
}

fn open_session(
    stored: &StoredSession,
) -> Result<(ProtonApiSession, ProtonDriveClient), AuthError> {
    let passphrases = stored.passphrases()?;
    let session = ProtonApiSession::resume(client_configuration(), stored.resume_parameters())?;
    let client = ProtonDriveClient::with_key_passphrases(&session, passphrases);
    Ok((session, client))
}

/// Ends the session at Proton and deletes it from the store.
///
/// The local copy is deleted even if Proton is unreachable.
///
/// # Errors
/// [`AuthError::Secrets`] if the local deletion fails.
pub async fn logout<S: SecretStore + ?Sized>(store: &S) -> Result<(), AuthError> {
    if let Ok(stored) = load_session(store)
        && let Ok(session) =
            ProtonApiSession::resume(client_configuration(), stored.resume_parameters())
    {
        // An error here (e.g. offline) does not prevent the local logout.
        let _ = session.end().await;
    }
    store.delete(&SecretKey::new(SESSION_SECRET_KEY))?;
    Ok(())
}

/// Attaches a callback that saves refreshed tokens immediately.
fn persist_refreshed_tokens<S: SecretStore + ?Sized + 'static>(
    session: &ProtonApiSession,
    stored: StoredSession,
    store: Arc<S>,
) {
    // The callback runs later on another thread; that is why it owns its
    // own copy of the session behind a mutex (`move`).
    let stored = Mutex::new(stored);
    session.http().set_on_tokens_refreshed(move |tokens| {
        let Ok(mut current) = stored.lock() else {
            return;
        };
        current.access_token = tokens.access_token;
        current.refresh_token = tokens.refresh_token;
        // A store error must not abort the running request; the next
        // refresh tries again.
        let _ = save_session(store.as_ref(), &current);
    });
}

/// Minimal hex encoding, so that no further dependency is needed for it.
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
            access_token: "access-secret".into(),
            refresh_token: "refresh-secret".into(),
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
        assert!(!text.contains("secret"));
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
        let garbage = Secret::new(b"not json".to_vec());
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
