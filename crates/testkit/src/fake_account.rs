//! Konto-Ersatz für Tests des Daemon-Ablaufs.

use std::sync::{Arc, Mutex};

use protonsync_application::ports::{Account, AccountError, LoginProgress};
use protonsync_domain::path::RelativePath;

use crate::FakeRemote;

/// Benutzername, den [`FakeAccount`] akzeptiert.
pub const FAKE_USER: &str = "krischan";
/// Passwort, das [`FakeAccount`] akzeptiert.
pub const FAKE_PASSWORD: &str = "richtig";
/// 2FA-Code, den [`FakeAccount`] akzeptiert.
pub const FAKE_TOTP: &str = "123456";

/// Ein Konto mit festen Zugangsdaten. Jeder geöffnete Ordner ist der ganze
/// [`FakeRemote`] (der Pfad wird nicht ausgewertet).
#[derive(Debug)]
pub struct FakeAccount {
    remote: Arc<FakeRemote>,
    needs_second_factor: bool,
    state: Mutex<AccountState>,
}

#[derive(Debug, Default)]
struct AccountState {
    stored_session: bool,
    logged_in: bool,
    pending: Option<LoginProgress>,
}

impl FakeAccount {
    /// Konto ohne 2FA und ohne gespeicherte Sitzung.
    #[must_use]
    pub fn new(remote: Arc<FakeRemote>) -> Self {
        Self {
            remote,
            needs_second_factor: false,
            state: Mutex::new(AccountState::default()),
        }
    }

    /// Das Konto verlangt einen 2FA-Code.
    #[must_use]
    pub fn with_second_factor(mut self) -> Self {
        self.needs_second_factor = true;
        self
    }

    /// Es gibt bereits eine gespeicherte Sitzung (für `resume`).
    #[must_use]
    pub fn with_stored_session(self) -> Self {
        self.with_state(|state| state.stored_session = true);
        self
    }

    /// Ob eine Sitzung gespeichert ist.
    #[must_use]
    pub fn has_stored_session(&self) -> bool {
        self.state.lock().is_ok_and(|state| state.stored_session)
    }

    fn with_state<T>(&self, change: impl FnOnce(&mut AccountState) -> T) -> Option<T> {
        self.state.lock().ok().map(|mut state| change(&mut state))
    }
}

fn rejected(what: &str) -> AccountError {
    AccountError::Rejected(format!("{what} abgelehnt"))
}

fn wrong_step() -> AccountError {
    AccountError::WrongStep("Keine Anmeldung begonnen".into())
}

// Die Schnittstelle ist asynchron, der Fake braucht aber kein `.await`.
#[allow(clippy::unused_async_trait_impl)]
impl Account for FakeAccount {
    type Remote = Arc<FakeRemote>;

    async fn resume(&self) -> Result<Option<String>, AccountError> {
        Ok(self
            .with_state(|state| {
                state.logged_in = state.stored_session;
                state.stored_session
            })
            .unwrap_or(false)
            .then(|| FAKE_USER.to_owned()))
    }

    async fn login_start(
        &self,
        username: &str,
        password: &str,
    ) -> Result<LoginProgress, AccountError> {
        if username != FAKE_USER || password != FAKE_PASSWORD {
            return Err(rejected("Passwort"));
        }
        let progress = LoginProgress {
            needs_second_factor: self.needs_second_factor,
            needs_data_password: false,
        };
        self.with_state(|state| state.pending = Some(progress));
        Ok(progress)
    }

    async fn login_second_factor(&self, code: &str) -> Result<LoginProgress, AccountError> {
        let pending = self
            .with_state(|state| state.pending)
            .flatten()
            .ok_or_else(wrong_step)?;
        if code != FAKE_TOTP {
            return Err(rejected("Code"));
        }
        let progress = LoginProgress {
            needs_second_factor: false,
            ..pending
        };
        self.with_state(|state| state.pending = Some(progress));
        Ok(progress)
    }

    async fn login_finish(&self, _data_password: Option<&str>) -> Result<String, AccountError> {
        let pending = self
            .with_state(|state| state.pending.take())
            .flatten()
            .ok_or_else(wrong_step)?;
        if pending.needs_second_factor {
            return Err(AccountError::WrongStep("2FA-Code fehlt".into()));
        }
        self.with_state(|state| {
            state.logged_in = true;
            state.stored_session = true;
        });
        Ok(FAKE_USER.to_owned())
    }

    async fn logout(&self) -> Result<(), AccountError> {
        self.with_state(|state| *state = AccountState::default());
        Ok(())
    }

    async fn username(&self) -> Option<String> {
        self.with_state(|state| state.logged_in)
            .unwrap_or(false)
            .then(|| FAKE_USER.to_owned())
    }

    async fn login_pending(&self) -> bool {
        self.with_state(|state| state.pending.is_some())
            .unwrap_or(false)
    }

    async fn open_remote(
        &self,
        _remote_path: &RelativePath,
    ) -> Result<Arc<FakeRemote>, AccountError> {
        if self.username().await.is_none() {
            return Err(AccountError::NotLoggedIn);
        }
        Ok(Arc::clone(&self.remote))
    }
}
