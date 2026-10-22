//! Scenario tests of the remote protection (`protect_remote`): local deletions and
//! replacements never reach Proton, everything else syncs as usual.

// Helper functions in test files may abort like tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{Fixture, T0, T1};
use protonsync_application::ports::{RemoteStorage, StateRepo};
use protonsync_application::sync::SyncOutcome;
use protonsync_application::use_cases::{ConflictService, UseCaseError};
use protonsync_domain::conflict::{ConflictKind, Resolution, SyncAction};
use protonsync_domain::sync_pair::SyncDirection;
use protonsync_domain::time::Timestamp;

const T2: Timestamp = Timestamp::from_unix_seconds(1_760_000_200);

async fn protected_fixture() -> Fixture {
    let mut f = Fixture::new(SyncDirection::TwoWay).await;
    f.pair.protect_remote = true;
    f
}

#[tokio::test]
async fn local_deletion_keeps_the_remote_file() {
    let f = protected_fixture().await;
    let root = f.remote_root().await;
    f.local.write("keep.txt", b"precious", T0);
    f.local.write("folder/inner.txt", b"inner", T0);
    f.sync().await;

    f.local.remove("keep.txt");
    f.local.remove("folder");
    let report = f.sync().await;

    assert_eq!(report.trashed_remote, 0, "{report:?}");
    assert_eq!(report.ignored, 3, "{report:?}");
    assert_eq!(f.remote_content(&root, "keep.txt").await, b"precious");
    let folder = f.remote_child(&root, "folder").await.unwrap();
    assert_eq!(f.remote_content(&folder, "inner.txt").await, b"inner");
    // The deletion is not undone locally either.
    assert!(f.local.content("keep.txt").is_none());
}

#[tokio::test]
async fn recreating_a_deleted_file_is_a_conflict() {
    let f = protected_fixture().await;
    let root = f.remote_root().await;
    f.local.write("report.txt", b"original", T0);
    f.sync().await;
    f.local.remove("report.txt");
    f.sync().await;

    f.local.write("report.txt", b"something else", T1);
    let report = f.sync().await;

    assert_eq!(report.conflicts, 1, "{report:?}");
    assert_eq!(f.remote_content(&root, "report.txt").await, b"original");
    let copy = format!("report (Konflikt {}).txt", T1.unix_seconds());
    assert_eq!(f.local.content("report.txt").unwrap(), b"original");
    assert_eq!(f.local.content(&copy).unwrap(), b"something else");
    let conflicts = f.state.conflicts(&f.pair.id).unwrap();
    assert_eq!(
        conflicts[0].1.kind,
        ConflictKind::LocalModifiedRemoteProtected
    );

    // The conflict copy is a new file and is uploaded; then everything is quiet.
    let report = f.sync().await;
    assert_eq!(report.uploaded, 1, "{report:?}");
    assert_eq!(f.remote_content(&root, &copy).await, b"something else");
    f.assert_stable().await;
}

#[tokio::test]
async fn local_edit_does_not_replace_the_remote_file() {
    let f = protected_fixture().await;
    let root = f.remote_root().await;
    f.local.write("notes.txt", b"v1", T0);
    f.sync().await;

    f.local.write("notes.txt", b"v2 local", T1);
    let report = f.sync().await;

    assert_eq!((report.uploaded, report.conflicts), (0, 1), "{report:?}");
    assert_eq!(f.remote_content(&root, "notes.txt").await, b"v1");
}

#[tokio::test]
async fn restoring_identical_content_is_no_conflict() {
    let f = protected_fixture().await;
    f.local.write("same.txt", b"same", T0);
    f.sync().await;
    f.local.remove("same.txt");
    f.sync().await;

    f.local.write("same.txt", b"same", T2);
    let report = f.sync().await;

    assert_eq!(report.conflicts, 0, "{report:?}");
    f.assert_stable().await;
}

#[tokio::test]
async fn new_files_and_remote_changes_still_sync() {
    let f = protected_fixture().await;
    let root = f.remote_root().await;
    let remote = f.remote_upload(&root, "remote.txt", b"1").await;
    f.local.write("gone-remote.txt", b"x", T0);
    f.sync().await;

    f.local.write("new.txt", b"new", T1);
    f.remote
        .upload_revision(&remote.id, &b"2"[..], 1, T1)
        .await
        .unwrap();
    let gone = f.remote_child(&root, "gone-remote.txt").await.unwrap();
    f.remote.trash(&gone.id).await.unwrap();
    let report = f.sync().await;

    assert_eq!(
        (report.uploaded, report.downloaded, report.trashed_local),
        (1, 1, 1),
        "{report:?}"
    );
    assert_eq!(f.remote_content(&root, "new.txt").await, b"new");
    assert_eq!(f.local.content("remote.txt").unwrap(), b"2");
    f.assert_stable().await;
}

#[tokio::test]
async fn dry_run_shows_the_protected_deletion() {
    let f = protected_fixture().await;
    f.local.write("a.txt", b"a", T0);
    f.sync().await;
    f.local.remove("a.txt");

    let SyncOutcome::Planned(plan) = f.context().run_once(true).await.unwrap() else {
        panic!("dry run expected");
    };
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].action, SyncAction::IgnoreLocalChange);
}

#[tokio::test]
async fn protected_conflict_cannot_be_won_by_the_local_version() {
    let f = protected_fixture().await;
    f.local.write("doc.txt", b"v1", T0);
    f.sync().await;
    f.local.write("doc.txt", b"v2", T1);
    f.sync().await;

    let service = ConflictService {
        local: &f.local,
        state: &f.state,
        clock: &f.clock,
        pair: &f.pair.id,
    };
    let (id, _) = service.list().unwrap().remove(0);
    assert!(matches!(
        service.resolve(id, Resolution::KeepLocal),
        Err(UseCaseError::RemoteProtected(_))
    ));
    service.resolve(id, Resolution::KeepRemote).unwrap();
    let copy = format!("doc (Konflikt {}).txt", T1.unix_seconds());
    assert!(f.local.content(&copy).is_none());
    assert_eq!(f.local.content("doc.txt").unwrap(), b"v1");
}
