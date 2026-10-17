//! Port `Account` for Proton: login in steps, resuming the session,
//! opening folders in "My files".

use std::sync::Arc;

use proton_drive_rs::ProtonDriveClient;
use protonsync_application::ports::{
    Account, AccountError, LoginProgress, RemoteStorage, SecretStore,
};
use protonsync_application::use_cases::ensure_folder;
use protonsync_domain::path::RelativePath;
use tokio::sync::Mutex;
use zeroize::Zeroizing;

use crate::auth::{self, AuthError, PendingLogin};
use crate::{ProtonPhotos, ProtonRemote};

impl From<AuthError> for AccountError {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::NotLoggedIn => Self::NotLoggedIn,
            AuthError::Rejected(_) | AuthError::SecurityKeyOnly => {
                Self::Rejected(error.to_string())
            }
            AuthError::WrongStep(_) => Self::WrongStep(error.to_string()),
            AuthError::Unavailable(_) | AuthError::CorruptSession(_) | AuthError::Secrets(_) => {
                Self::Unavailable(error.to_string())
            }
        }
    }
}

/// The Proton account with its session in the secret store `S`.
pub struct ProtonAccount<S: ?Sized> {
    secrets: Arc<S>,
    // An async mutex, because the lock is held across `.await`.
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    session: Option<Session>,
    pending: Option<Pending>,
}

struct Session {
    client: ProtonDriveClient,
    username: String,
}

/// A started login. The login password is kept only until completion
/// (for single-password mode) and overwritten when dropped.
struct Pending {
    login: PendingLogin,
    password: Zeroizing<String>,
}

impl<S: SecretStore + ?Sized + 'static> ProtonAccount<S> {
    /// Creates the account; not logged in yet.
    pub fn new(secrets: Arc<S>) -> Self {
        Self {
            secrets,
            inner: Mutex::new(Inner::default()),
        }
    }
}

fn progress_of(login: &PendingLogin) -> LoginProgress {
    LoginProgress {
        needs_second_factor: login.needs_second_factor(),
        needs_data_password: login.needs_data_password(),
    }
}

fn no_login_started() -> AccountError {
    AccountError::WrongStep("no login started".into())
}

impl<S: SecretStore + ?Sized + 'static> Account for ProtonAccount<S> {
    type Remote = ProtonRemote;
    type Photos = ProtonPhotos;

    async fn resume(&self) -> Result<Option<String>, AccountError> {
        let username = match auth::load_session(self.secrets.as_ref()) {
            Ok(stored) => stored.username().to_owned(),
            Err(AuthError::NotLoggedIn) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let client = auth::resume(Arc::clone(&self.secrets))?;
        self.inner.lock().await.session = Some(Session {
            client,
            username: username.clone(),
        });
        Ok(Some(username))
    }

    async fn login_start(
        &self,
        username: &str,
        password: &str,
    ) -> Result<LoginProgress, AccountError> {
        let login = PendingLogin::start(username, password.as_bytes()).await?;
        let progress = progress_of(&login);
        self.inner.lock().await.pending = Some(Pending {
            login,
            password: Zeroizing::new(password.to_owned()),
        });
        Ok(progress)
    }

    async fn login_second_factor(&self, code: &str) -> Result<LoginProgress, AccountError> {
        let mut inner = self.inner.lock().await;
        let pending = inner.pending.as_mut().ok_or_else(no_login_started)?;
        pending.login.submit_totp(code).await?;
        Ok(progress_of(&pending.login))
    }

    async fn login_finish(&self, data_password: Option<&str>) -> Result<String, AccountError> {
        let mut inner = self.inner.lock().await;
        let pending = inner.pending.take().ok_or_else(no_login_started)?;
        let password = Zeroizing::new(
            data_password
                .unwrap_or(pending.password.as_str())
                .as_bytes()
                .to_vec(),
        );
        let (_, stored) = pending.login.finish(&password).await?;
        auth::save_session(self.secrets.as_ref(), &stored)?;
        // Resume instead of keeping the login client: this way refreshed
        // tokens are saved from the start.
        let client = auth::resume(Arc::clone(&self.secrets))?;
        let username = stored.username().to_owned();
        inner.session = Some(Session {
            client,
            username: username.clone(),
        });
        Ok(username)
    }

    async fn logout(&self) -> Result<(), AccountError> {
        let mut inner = self.inner.lock().await;
        inner.session = None;
        inner.pending = None;
        auth::logout(self.secrets.as_ref()).await?;
        Ok(())
    }

    async fn username(&self) -> Option<String> {
        let inner = self.inner.lock().await;
        inner
            .session
            .as_ref()
            .map(|session| session.username.clone())
    }

    async fn login_pending(&self) -> bool {
        self.inner.lock().await.pending.is_some()
    }

    async fn open_remote(&self, remote_path: &RelativePath) -> Result<ProtonRemote, AccountError> {
        let client = {
            let inner = self.inner.lock().await;
            let session = inner.session.as_ref().ok_or(AccountError::NotLoggedIn)?;
            session.client.clone()
        };
        let my_files = ProtonRemote::my_files(client.clone()).await?;
        let start = my_files.root_folder().await?;
        let folder = Box::pin(ensure_folder(&my_files, &start, remote_path)).await?;
        Ok(ProtonRemote::with_root(client, &folder.id)?)
    }

    async fn open_photos(&self) -> Result<ProtonPhotos, AccountError> {
        let inner = self.inner.lock().await;
        let session = inner.session.as_ref().ok_or(AccountError::NotLoggedIn)?;
        Ok(ProtonPhotos::new(session.client.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protonsync_testkit::InMemorySecretStore;

    fn account() -> ProtonAccount<InMemorySecretStore> {
        ProtonAccount::new(Arc::new(InMemorySecretStore::default()))
    }

    #[tokio::test]
    async fn starts_logged_out_without_stored_session() {
        let account = account();
        assert_eq!(account.resume().await, Ok(None));
        assert_eq!(account.username().await, None);
        assert!(!account.login_pending().await);
        let root = RelativePath::root();
        assert!(matches!(
            account.open_remote(&root).await,
            Err(AccountError::NotLoggedIn)
        ));
    }

    #[tokio::test]
    async fn login_steps_need_a_started_login() {
        let account = account();
        assert!(matches!(
            account.login_second_factor("123456").await,
            Err(AccountError::WrongStep(_))
        ));
        assert!(matches!(
            account.login_finish(None).await,
            Err(AccountError::WrongStep(_))
        ));
    }

    #[test]
    fn auth_errors_map_to_account_errors() {
        assert_eq!(
            AccountError::from(AuthError::NotLoggedIn),
            AccountError::NotLoggedIn
        );
        assert!(matches!(
            AccountError::from(AuthError::SecurityKeyOnly),
            AccountError::Rejected(_)
        ));
        assert!(matches!(
            AccountError::from(AuthError::CorruptSession("x".into())),
            AccountError::Unavailable(_)
        ));
    }
}
