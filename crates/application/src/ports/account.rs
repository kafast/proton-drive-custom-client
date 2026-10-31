//! Port for the cloud account: log in (with 2FA), resume a session, log out
//! and open the remote storage of a sync pair or the photo library.

use std::future::Future;

use protonsync_domain::path::RelativePath;
use thiserror::Error;

use super::{PhotoLibrary, RemoteError, RemoteStorage};

/// Errors around the account.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AccountError {
    /// Not logged in or session expired.
    #[error("not logged in")]
    NotLoggedIn,
    /// Credentials or code rejected.
    #[error("{0}")]
    Rejected(String),
    /// Step in the wrong order.
    #[error("{0}")]
    WrongStep(String),
    /// Service or secret store unreachable.
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

/// What the login needs next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginProgress {
    /// A 2FA code (TOTP).
    pub needs_second_factor: bool,
    /// A separate data password (two-password mode).
    pub needs_data_password: bool,
}

/// The account. All methods may be called in parallel.
pub trait Account: Send + Sync {
    /// Remote storage of a sync pair.
    type Remote: RemoteStorage;
    /// The photo library of the account.
    type Photos: PhotoLibrary;

    /// Resumes a stored session; returns the user or `None`.
    fn resume(&self) -> impl Future<Output = Result<Option<String>, AccountError>> + Send;

    /// Starts a login.
    fn login_start(
        &self,
        username: &str,
        password: &str,
    ) -> impl Future<Output = Result<LoginProgress, AccountError>> + Send;

    /// Submits the 2FA code.
    fn login_second_factor(
        &self,
        code: &str,
    ) -> impl Future<Output = Result<LoginProgress, AccountError>> + Send;

    /// Completes the login and stores the session; returns the user.
    /// Without `data_password` the login password applies.
    fn login_finish(
        &self,
        data_password: Option<&str>,
    ) -> impl Future<Output = Result<String, AccountError>> + Send;

    /// Logs out and deletes the stored session.
    fn logout(&self) -> impl Future<Output = Result<(), AccountError>> + Send;

    /// Logged-in user.
    fn username(&self) -> impl Future<Output = Option<String>> + Send;

    /// Whether a login was started but not completed.
    fn login_pending(&self) -> impl Future<Output = bool> + Send;

    /// Opens the folder `remote_path` below the account's storage and creates
    /// it if needed.
    fn open_remote(
        &self,
        remote_path: &RelativePath,
    ) -> impl Future<Output = Result<Self::Remote, AccountError>> + Send;

    /// Opens a folder that another user shares with this account
    /// ("Shared with me"). The first segment of `remote_path` is the name of
    /// the shared folder, the rest a subfolder inside it. Nothing is created,
    /// because the share may be read-only.
    fn open_shared_remote(
        &self,
        remote_path: &RelativePath,
    ) -> impl Future<Output = Result<Self::Remote, AccountError>> + Send;

    /// Opens the photo library (Proton Photos).
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
            AccountError::from(RemoteError::Unavailable("network".into())),
            AccountError::Unavailable(_)
        ));
    }
}
