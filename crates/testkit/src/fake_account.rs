//! Account stand-in for tests of the daemon workflow.

use std::sync::{Arc, Mutex};

use protonsync_application::ports::{Account, AccountError, LoginProgress};
use protonsync_domain::path::RelativePath;

use crate::{FakePhotoLibrary, FakeRemote};

/// Username that [`FakeAccount`] accepts.
pub const FAKE_USER: &str = "krischan";
/// Password that [`FakeAccount`] accepts.
pub const FAKE_PASSWORD: &str = "correct";
/// 2FA code that [`FakeAccount`] accepts.
pub const FAKE_TOTP: &str = "123456";

/// An account with fixed credentials. Every opened folder is the whole
/// [`FakeRemote`] (the path is not evaluated).
#[derive(Debug)]
pub struct FakeAccount {
    remote: Arc<FakeRemote>,
    photos: Arc<FakePhotoLibrary>,
    needs_second_factor: bool,
    state: Mutex<AccountState>,
    /// Paths opened as shared folders ("Shared with me"), in order. Shared,
    /// so tests can still read it after the account moved into the daemon.
    shared_opened: Arc<Mutex<Vec<String>>>,
}

#[derive(Debug, Default)]
struct AccountState {
    stored_session: bool,
    logged_in: bool,
    pending: Option<LoginProgress>,
    /// Error for the next `open_remote` calls (e.g. Proton refuses to create
    /// the folder) and how many calls still fail.
    open_remote_error: Option<(AccountError, u32)>,
}

impl FakeAccount {
    /// Account without 2FA and without a stored session.
    #[must_use]
    pub fn new(remote: Arc<FakeRemote>) -> Self {
        Self {
            remote,
            photos: Arc::new(FakePhotoLibrary::new()),
            needs_second_factor: false,
            state: Mutex::new(AccountState::default()),
            shared_opened: Arc::default(),
        }
    }

    /// The account's photo library (for checks and changes "from the phone").
    #[must_use]
    pub fn photos(&self) -> Arc<FakePhotoLibrary> {
        Arc::clone(&self.photos)
    }

    /// The account requires a 2FA code.
    #[must_use]
    pub fn with_second_factor(mut self) -> Self {
        self.needs_second_factor = true;
        self
    }

    /// There already is a stored session (for `resume`).
    #[must_use]
    pub fn with_stored_session(self) -> Self {
        self.with_state(|state| state.stored_session = true);
        self
    }

    /// Whether a session is stored.
    #[must_use]
    pub fn has_stored_session(&self) -> bool {
        self.state.lock().is_ok_and(|state| state.stored_session)
    }

    /// The next `times` attempts to open a remote folder fail with `error`.
    #[must_use]
    pub fn failing_open_remote(self, error: AccountError, times: u32) -> Self {
        self.with_state(|state| state.open_remote_error = Some((error, times)));
        self
    }

    /// Log of the paths opened with `open_shared_remote`.
    #[must_use]
    pub fn shared_opened(&self) -> Arc<Mutex<Vec<String>>> {
        Arc::clone(&self.shared_opened)
    }

    fn take_open_remote_error(&self) -> Option<AccountError> {
        self.with_state(|state| {
            let (error, times) = state.open_remote_error.as_mut()?;
            if *times == 0 {
                return None;
            }
            *times -= 1;
            Some(error.clone())
        })
        .flatten()
    }

    fn with_state<T>(&self, change: impl FnOnce(&mut AccountState) -> T) -> Option<T> {
        self.state.lock().ok().map(|mut state| change(&mut state))
    }
}

fn rejected(what: &str) -> AccountError {
    AccountError::Rejected(format!("{what} rejected"))
}

fn wrong_step() -> AccountError {
    AccountError::WrongStep("no login started".into())
}

// The interface is asynchronous, but the fake needs no `.await`.
#[allow(clippy::unused_async_trait_impl)]
impl Account for FakeAccount {
    type Remote = Arc<FakeRemote>;
    type Photos = Arc<FakePhotoLibrary>;

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
            return Err(rejected("password"));
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
            return Err(rejected("code"));
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
            return Err(AccountError::WrongStep("2FA code missing".into()));
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
        if let Some(error) = self.take_open_remote_error() {
            return Err(error);
        }
        Ok(Arc::clone(&self.remote))
    }

    async fn open_shared_remote(
        &self,
        remote_path: &RelativePath,
    ) -> Result<Arc<FakeRemote>, AccountError> {
        let remote = self.open_remote(remote_path).await?;
        if let Ok(mut opened) = self.shared_opened.lock() {
            opened.push(remote_path.as_str().to_owned());
        }
        Ok(remote)
    }

    async fn open_photos(&self) -> Result<Arc<FakePhotoLibrary>, AccountError> {
        if self.username().await.is_none() {
            return Err(AccountError::NotLoggedIn);
        }
        Ok(Arc::clone(&self.photos))
    }
}
