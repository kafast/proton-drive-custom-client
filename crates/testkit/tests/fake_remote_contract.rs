//! Der `FakeRemote` muss dieselben Contract-Tests bestehen wie der echte Adapter.

protonsync_testkit::remote_storage_contract_tests!(protonsync_testkit::FakeRemote::new);

use protonsync_application::ports::{RemoteError, RemoteStorage};

#[tokio::test]
async fn injected_failure_hits_only_the_next_call() {
    let remote = protonsync_testkit::FakeRemote::new();
    remote.fail_next_call(RemoteError::Unavailable("Netz weg".into()));
    assert!(matches!(
        remote.root_folder().await,
        Err(RemoteError::Unavailable(_))
    ));
    assert!(remote.root_folder().await.is_ok());
}

/// Der geteilte Speicher (`Arc`) muss sich genauso verhalten.
mod shared {
    protonsync_testkit::remote_storage_contract_tests!(|| std::sync::Arc::new(
        protonsync_testkit::FakeRemote::new()
    ));
}
