//! The `FakeRemote` must pass the same contract tests as the real adapter.

protonsync_testkit::remote_storage_contract_tests!(protonsync_testkit::FakeRemote::new);

use protonsync_application::ports::{RemoteError, RemoteStorage};

#[tokio::test]
async fn injected_failure_hits_only_the_next_call() {
    let remote = protonsync_testkit::FakeRemote::new();
    remote.fail_next_call(RemoteError::Unavailable("network down".into()));
    assert!(matches!(
        remote.root_folder().await,
        Err(RemoteError::Unavailable(_))
    ));
    assert!(remote.root_folder().await.is_ok());
}

/// The shared storage (`Arc`) must behave the same way.
mod shared {
    protonsync_testkit::remote_storage_contract_tests!(|| std::sync::Arc::new(
        protonsync_testkit::FakeRemote::new()
    ));
}
