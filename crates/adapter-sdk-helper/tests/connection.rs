//! Behavior of the connection itself: handshake, overlapping calls, a helper
//! that disappears, a real child process.

mod support;

use std::sync::Arc;

use protonsync_adapter_sdk_helper::Connection;
use protonsync_application::ports::{RemoteError, RemoteStorage};
use protonsync_domain::node::NodeName;
use protonsync_domain::time::Timestamp;
use protonsync_testkit::FakeRemote;
use tokio::process::Command;

#[tokio::test]
async fn hello_reports_the_sdk_version() {
    let (_, connection) = support::helper_remote_on(Arc::new(FakeRemote::new()));
    assert_eq!(connection.hello().await.unwrap(), "fake");
}

#[tokio::test]
async fn overlapping_calls_get_their_own_answers() {
    let (remote, _) = support::helper_remote_on(Arc::new(FakeRemote::new()));
    let root = remote.root_folder().await.unwrap();
    let names = ["a", "b", "c", "d", "e", "f"];
    let calls = names.map(|name| {
        let remote = remote.clone();
        let root = root.id.clone();
        tokio::spawn(async move {
            remote
                .create_folder(&root, &NodeName::new(name).unwrap())
                .await
                .unwrap()
        })
    });
    for (call, name) in calls.into_iter().zip(names) {
        assert_eq!(call.await.unwrap().name.as_str(), name);
    }
}

#[tokio::test]
async fn errors_from_the_storage_reach_the_caller() {
    let storage = Arc::new(FakeRemote::new());
    let (remote, _) = support::helper_remote_on(Arc::clone(&storage));
    storage.fail_next_call(RemoteError::Unauthorized);
    assert_eq!(remote.root_folder().await, Err(RemoteError::Unauthorized));
}

#[tokio::test]
async fn upload_of_a_growing_source_is_rejected() {
    let (remote, _) = support::helper_remote_on(Arc::new(FakeRemote::new()));
    let root = remote.root_folder().await.unwrap();
    let result = remote
        .upload_file(
            &root.id,
            &NodeName::new("grew").unwrap(),
            &b"abcdef"[..],
            3,
            Timestamp::from_unix_seconds(1),
        )
        .await;
    assert!(matches!(result, Err(RemoteError::Transfer(_))));
    assert_eq!(remote.list_children(&root.id).await.unwrap(), Vec::new());
}

#[tokio::test]
async fn a_vanished_helper_makes_calls_unavailable() {
    let (ours, theirs) = tokio::io::duplex(1024);
    drop(theirs);
    let (reader, writer) = tokio::io::split(ours);
    let connection = Connection::over(reader, writer);
    assert!(matches!(
        connection.hello().await,
        Err(RemoteError::Unavailable(_))
    ));
}

#[tokio::test]
async fn a_missing_program_is_unavailable() {
    let result = Connection::spawn(&mut Command::new("/nonexistent/protonsync-sdk-helper"));
    assert!(matches!(result, Err(RemoteError::Unavailable(_))));
}

#[cfg(unix)]
#[tokio::test]
async fn a_child_process_is_spoken_to_over_stdio() {
    // A stand-in helper: answers the first request (id 1) and exits.
    let script = r#"read line; echo '{"id":1,"ok":{"protocol":1,"sdk_version":"sh"}}'"#;
    let connection = Connection::spawn(Command::new("sh").args(["-c", script])).unwrap();
    assert_eq!(connection.hello().await.unwrap(), "sh");
    // The helper has exited by now.
    assert!(matches!(
        connection.hello().await,
        Err(RemoteError::Unavailable(_))
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn a_different_protocol_version_is_rejected() {
    let script = r#"read line; echo '{"id":1,"ok":{"protocol":99,"sdk_version":"x"}}'; sleep 1"#;
    let connection = Connection::spawn(Command::new("sh").args(["-c", script])).unwrap();
    assert!(matches!(
        connection.hello().await,
        Err(RemoteError::Rejected(_))
    ));
}
