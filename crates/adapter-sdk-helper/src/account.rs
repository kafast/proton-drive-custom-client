//! Port `Account` on the SDK helper: browser login, session, folders.

use std::path::PathBuf;
use std::sync::Arc;

use protonsync_application::ports::{
    Account, AccountError, EventCursor, LoginProgress, NewPhoto, PhotoLibrary, RemoteError,
    RemoteEvent, RemoteStorage,
};
use protonsync_application::use_cases::{ensure_folder, find_folder};
use protonsync_domain::node::NodeId;
use protonsync_domain::path::RelativePath;
use protonsync_domain::photo::Photo;
use serde::Deserialize;
use serde_json::json;
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::connection::Connection;
use crate::protocol::WireNode;
use crate::remote::HelperRemote;

/// Where the helper keeps the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStore {
    /// Secret Service (GNOME Keyring, `KWallet`); for the desktop.
    Keychain,
    /// A file with mode 0600 in the helper's data folder; for the server.
    File,
}

impl SessionStore {
    /// Value of `PROTON_DRIVE_CREDENTIALS_STORE` for the helper.
    fn env_value(self) -> &'static str {
        match self {
            Self::Keychain => "keychain",
            Self::File => "unsafe_file",
        }
    }
}

/// How to start the helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperConfig {
    /// The helper program (`protonsync-sdk-helper`).
    pub program: PathBuf,
    /// Data folder of the helper (session file, caches).
    pub data_dir: PathBuf,
    /// Where the session is kept.
    pub session_store: SessionStore,
}

#[derive(Default)]
struct State {
    connection: Option<Arc<Connection>>,
    username: Option<String>,
    login_pending: bool,
}

/// The account, served by the SDK helper. The helper is started on first use.
pub struct HelperAccount {
    config: Option<HelperConfig>,
    state: Mutex<State>,
}

#[derive(Deserialize)]
struct SessionAnswer {
    logged_in: bool,
    user: Option<String>,
}

#[derive(Deserialize)]
struct LoginStart {
    url: String,
}

#[derive(Deserialize)]
struct LoginWait {
    done: bool,
    user: Option<String>,
}

const PASSWORD_LOGIN_UNSUPPORTED: &str =
    "the official connection signs in through the browser: protonsync login --browser";

impl HelperAccount {
    /// Account that starts the helper as configured.
    #[must_use]
    pub fn new(config: HelperConfig) -> Self {
        Self {
            config: Some(config),
            state: Mutex::default(),
        }
    }

    /// Account on an existing connection (tests, tools).
    #[must_use]
    pub fn connected(connection: Arc<Connection>) -> Self {
        Self {
            config: None,
            state: Mutex::new(State {
                connection: Some(connection),
                ..State::default()
            }),
        }
    }

    /// The running helper; starts it if needed. A helper that stopped is
    /// started again.
    async fn connection(&self) -> Result<Arc<Connection>, AccountError> {
        let mut state = self.state.lock().await;
        if let Some(connection) = &state.connection
            && connection.is_alive()
        {
            return Ok(Arc::clone(connection));
        }
        let Some(config) = &self.config else {
            return Err(AccountError::Unavailable(
                "SDK helper is not running".into(),
            ));
        };
        let mut command = Command::new(&config.program);
        command.env("PROTON_DRIVE_CACHE_DIR", &config.data_dir).env(
            "PROTON_DRIVE_CREDENTIALS_STORE",
            config.session_store.env_value(),
        );
        let connection = Arc::new(Connection::spawn(&mut command)?);
        connection.hello().await?;
        state.connection = Some(Arc::clone(&connection));
        Ok(connection)
    }
}

// Some methods only refuse; they stay `async` to read like the others.
#[allow(clippy::unused_async_trait_impl)]
impl Account for HelperAccount {
    type Remote = HelperRemote;
    type Photos = NoPhotos;

    async fn resume(&self) -> Result<Option<String>, AccountError> {
        let connection = self.connection().await?;
        let answer: SessionAnswer = connection.call("session", &json!({})).await?;
        let user = answer.logged_in.then_some(answer.user).flatten();
        self.state.lock().await.username.clone_from(&user);
        Ok(user)
    }

    async fn login_start(&self, _: &str, _: &str) -> Result<LoginProgress, AccountError> {
        Err(AccountError::WrongStep(PASSWORD_LOGIN_UNSUPPORTED.into()))
    }

    async fn login_second_factor(&self, _: &str) -> Result<LoginProgress, AccountError> {
        Err(AccountError::WrongStep(PASSWORD_LOGIN_UNSUPPORTED.into()))
    }

    async fn login_finish(&self, _: Option<&str>) -> Result<String, AccountError> {
        Err(AccountError::WrongStep(PASSWORD_LOGIN_UNSUPPORTED.into()))
    }

    async fn browser_login_start(&self) -> Result<String, AccountError> {
        let connection = self.connection().await?;
        let answer: LoginStart = connection.call("login_web_start", &json!({})).await?;
        self.state.lock().await.login_pending = true;
        Ok(answer.url)
    }

    async fn browser_login_wait(&self) -> Result<Option<String>, AccountError> {
        let connection = self.connection().await?;
        let result: Result<LoginWait, RemoteError> =
            connection.call("login_web_wait", &json!({})).await;
        let mut state = self.state.lock().await;
        match result {
            Ok(answer) if answer.done => {
                state.login_pending = false;
                state.username.clone_from(&answer.user);
                Ok(Some(answer.user.unwrap_or_default()))
            }
            Ok(_) => Ok(None),
            Err(RemoteError::Rejected(message)) => {
                state.login_pending = false;
                Err(AccountError::Rejected(message))
            }
            Err(other) => Err(other.into()),
        }
    }

    fn uses_browser_login(&self) -> bool {
        true
    }

    async fn logout(&self) -> Result<(), AccountError> {
        let connection = self.connection().await?;
        let _: serde_json::Value = connection.call("logout", &json!({})).await?;
        let mut state = self.state.lock().await;
        state.username = None;
        state.login_pending = false;
        Ok(())
    }

    async fn username(&self) -> Option<String> {
        self.state.lock().await.username.clone()
    }

    async fn login_pending(&self) -> bool {
        self.state.lock().await.login_pending
    }

    async fn open_remote(&self, remote_path: &RelativePath) -> Result<HelperRemote, AccountError> {
        let connection = self.connection().await?;
        let root: WireNode = connection.call("my_files_root", &json!({})).await?;
        let my_files = HelperRemote::new(Arc::clone(&connection), NodeId::new(root.id));
        let start = my_files.root_folder().await?;
        let folder = Box::pin(ensure_folder(&my_files, &start, remote_path)).await?;
        Ok(HelperRemote::new(connection, folder.id))
    }

    async fn open_shared_remote(
        &self,
        remote_path: &RelativePath,
    ) -> Result<HelperRemote, AccountError> {
        let connection = self.connection().await?;
        let mut segments = remote_path.segments();
        let shared_name = segments.next().ok_or_else(|| {
            AccountError::Unavailable("a shared pair needs the name of a shared folder".into())
        })?;
        let shared: Vec<WireNode> = connection.call("shared_with_me", &json!({})).await?;
        let root = shared
            .into_iter()
            .find(|node| node.name == shared_name)
            .ok_or_else(|| {
                AccountError::Unavailable(format!(
                    "\"{shared_name}\" not found in \"Shared with me\""
                ))
            })?;
        let shared_remote = HelperRemote::new(Arc::clone(&connection), NodeId::new(root.id));
        let start = shared_remote.root_folder().await?;
        let inner_path = RelativePath::parse(&segments.collect::<Vec<_>>().join("/"))
            .map_err(|e| AccountError::Unavailable(e.to_string()))?;
        let folder = Box::pin(find_folder(&shared_remote, &start, &inner_path))
            .await?
            .ok_or_else(|| {
                AccountError::Unavailable(format!(
                    "folder {remote_path} not found in \"Shared with me\""
                ))
            })?;
        Ok(HelperRemote::new(connection, folder.id))
    }

    async fn open_photos(&self) -> Result<NoPhotos, AccountError> {
        Err(AccountError::Unavailable(PHOTOS_MISSING.into()))
    }
}

const PHOTOS_MISSING: &str = "photos are not yet available with the official connection";

/// Placeholder photo library: the helper does not serve photos yet
/// (ADR 0025, step 4). [`HelperAccount::open_photos`] never returns one.
pub struct NoPhotos;

fn photos_missing() -> RemoteError {
    RemoteError::Unavailable(PHOTOS_MISSING.into())
}

#[allow(clippy::unused_async_trait_impl)]
impl PhotoLibrary for NoPhotos {
    async fn photos(&self) -> Result<Vec<Photo>, RemoteError> {
        Err(photos_missing())
    }

    async fn photo(&self, _: &NodeId) -> Result<Photo, RemoteError> {
        Err(photos_missing())
    }

    async fn events_since(
        &self,
        _: Option<&EventCursor>,
    ) -> Result<(Vec<RemoteEvent>, EventCursor), RemoteError> {
        Err(photos_missing())
    }

    async fn download_to<W: std::io::Write + Send>(
        &self,
        _: &NodeId,
        _: &mut W,
    ) -> Result<u64, RemoteError> {
        Err(photos_missing())
    }

    async fn upload<R: std::io::Read + Send>(
        &self,
        _: &NewPhoto,
        _: R,
    ) -> Result<Photo, RemoteError> {
        Err(photos_missing())
    }

    async fn trash(&self, _: &NodeId) -> Result<(), RemoteError> {
        Err(photos_missing())
    }

    async fn restore(&self, _: &NodeId) -> Result<(), RemoteError> {
        Err(photos_missing())
    }

    async fn delete_permanently(&self, _: &NodeId) -> Result<(), RemoteError> {
        Err(photos_missing())
    }
}
