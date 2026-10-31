//! `HelperAccount` against the fake helper: browser login, session, folders.

mod support;

use std::sync::Arc;

use protonsync_adapter_sdk_helper::HelperAccount;
use protonsync_application::ports::{Account, AccountError, RemoteStorage};
use protonsync_domain::node::NodeName;
use protonsync_domain::path::RelativePath;
use protonsync_testkit::FakeRemote;

fn account_on(storage: Arc<FakeRemote>) -> HelperAccount {
    let (_, connection) = support::helper_remote_on(storage);
    HelperAccount::connected(connection)
}

#[tokio::test]
async fn browser_login_then_resume_and_logout() {
    let account = account_on(Arc::new(FakeRemote::new()));
    assert!(account.uses_browser_login());
    assert_eq!(account.resume().await.unwrap(), None);

    let url = account.browser_login_start().await.unwrap();
    assert!(url.starts_with("https://"));
    assert!(account.login_pending().await);
    assert_eq!(account.browser_login_wait().await.unwrap(), None);
    let user = account.browser_login_wait().await.unwrap();
    assert_eq!(user.as_deref(), Some("tester@example.invalid"));
    assert!(!account.login_pending().await);
    assert_eq!(
        account.username().await.as_deref(),
        Some("tester@example.invalid")
    );

    assert!(account.resume().await.unwrap().is_some());
    account.logout().await.unwrap();
    assert_eq!(account.username().await, None);
    assert_eq!(account.resume().await.unwrap(), None);
}

#[tokio::test]
async fn waiting_without_a_started_login_is_rejected() {
    let account = account_on(Arc::new(FakeRemote::new()));
    assert!(matches!(
        account.browser_login_wait().await,
        Err(AccountError::Rejected(_))
    ));
}

#[tokio::test]
async fn password_login_is_refused() {
    let account = account_on(Arc::new(FakeRemote::new()));
    assert!(matches!(
        account.login_start("user", "secret").await,
        Err(AccountError::WrongStep(_))
    ));
    assert!(matches!(
        account.login_second_factor("123456").await,
        Err(AccountError::WrongStep(_))
    ));
    assert!(matches!(
        account.login_finish(None).await,
        Err(AccountError::WrongStep(_))
    ));
}

#[tokio::test]
async fn open_remote_creates_the_folder_path() {
    let storage = Arc::new(FakeRemote::new());
    let account = account_on(Arc::clone(&storage));
    let path = RelativePath::parse("Backup/Laptop").unwrap();
    let remote = account.open_remote(&path).await.unwrap();
    assert_eq!(remote.root_folder().await.unwrap().name.as_str(), "Laptop");
    // A second open finds the same folder instead of creating another one.
    let again = account.open_remote(&path).await.unwrap();
    assert_eq!(
        again.root_folder().await.unwrap().id,
        remote.root_folder().await.unwrap().id
    );
}

#[tokio::test]
async fn open_shared_remote_finds_but_never_creates() {
    let storage = Arc::new(FakeRemote::new());
    let root = storage.root_folder().await.unwrap();
    let shared = storage
        .create_folder(&root.id, &NodeName::new("shared-Fotos").unwrap())
        .await
        .unwrap();
    storage
        .create_folder(&shared.id, &NodeName::new("2026").unwrap())
        .await
        .unwrap();
    let account = account_on(Arc::clone(&storage));

    let inner = account
        .open_shared_remote(&RelativePath::parse("shared-Fotos/2026").unwrap())
        .await
        .unwrap();
    assert_eq!(inner.root_folder().await.unwrap().name.as_str(), "2026");
    assert!(matches!(
        account
            .open_shared_remote(&RelativePath::parse("shared-Fotos/fehlt").unwrap())
            .await,
        Err(AccountError::Unavailable(_))
    ));
    assert!(matches!(
        account
            .open_shared_remote(&RelativePath::parse("unbekannt").unwrap())
            .await,
        Err(AccountError::Unavailable(_))
    ));
}

#[tokio::test]
async fn photos_are_not_served_yet() {
    let account = account_on(Arc::new(FakeRemote::new()));
    assert!(matches!(
        account.open_photos().await,
        Err(AccountError::Unavailable(_))
    ));
}

#[tokio::test]
async fn a_configured_but_missing_helper_is_unavailable() {
    let account = HelperAccount::new(protonsync_adapter_sdk_helper::HelperConfig {
        program: "/nonexistent/protonsync-sdk-helper".into(),
        data_dir: std::env::temp_dir(),
        session_store: protonsync_adapter_sdk_helper::SessionStore::File,
    });
    assert!(matches!(
        account.resume().await,
        Err(AccountError::Unavailable(_))
    ));
}
