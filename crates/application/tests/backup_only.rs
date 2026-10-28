//! Scenario tests of the backup mode (`backup_only`): everything from Proton is
//! stored locally, nothing is uploaded or deleted, and replaced local versions
//! go to the version archive `_archiv` next to the file.

// Helper functions in test files may abort like tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{Fixture, T0, T1};
use protonsync_application::ports::{RemoteStorage, StateRepo};
use protonsync_application::sync::SyncOutcome;
use protonsync_domain::conflict::SyncAction;
use protonsync_domain::node::{Node, NodeName};
use protonsync_domain::sync_pair::SyncDirection;

/// Archive prefix for the fixture clock (T1 = 2025-10-09 08:55:00 UTC).
const PREFIX: &str = "2025-10-09_08-55-00_";

async fn backup_fixture() -> Fixture {
    Fixture::new(SyncDirection::BackupOnly).await
}

/// Simulates a new version uploaded in Proton by another device.
async fn remote_new_revision(f: &Fixture, node: &Node, content: &[u8]) {
    f.remote
        .upload_revision(&node.id, content, content.len() as u64, T0)
        .await
        .unwrap();
}

#[tokio::test]
async fn everything_in_proton_is_stored_locally() {
    let f = backup_fixture().await;
    let root = f.remote_root().await;
    f.remote_upload(&root, "a.txt", b"a").await;
    let folder = f
        .remote
        .create_folder(&root.id, &NodeName::new("docs").unwrap())
        .await
        .unwrap();
    f.remote_upload(&folder, "b.txt", b"b").await;

    let report = f.sync().await;

    assert_eq!(report.downloaded, 3, "{report:?}");
    assert_eq!(f.local.content("a.txt").unwrap(), b"a");
    assert_eq!(f.local.content("docs/b.txt").unwrap(), b"b");
    f.assert_stable().await;
}

#[tokio::test]
async fn a_changed_file_archives_the_old_version_first() {
    let f = backup_fixture().await;
    let root = f.remote_root().await;
    let node = f.remote_upload(&root, "report.txt", b"v1").await;
    f.sync().await;

    remote_new_revision(&f, &node, b"v2").await;
    let report = f.sync().await;

    assert_eq!(report.downloaded, 1, "{report:?}");
    assert_eq!(f.local.content("report.txt").unwrap(), b"v2");
    let archived = format!("_archiv/{PREFIX}report.txt");
    assert_eq!(f.local.content(&archived).unwrap(), b"v1");
    f.assert_stable().await;
}

#[tokio::test]
async fn many_versions_pile_up_in_the_archive() {
    let f = backup_fixture().await;
    let root = f.remote_root().await;
    let folder = f
        .remote
        .create_folder(&root.id, &NodeName::new("docs").unwrap())
        .await
        .unwrap();
    let node = f.remote_upload(&folder, "notes.md", b"v1").await;
    f.sync().await;

    remote_new_revision(&f, &node, b"v2").await;
    f.sync().await;
    remote_new_revision(&f, &node, b"v3").await;
    f.sync().await;

    assert_eq!(f.local.content("docs/notes.md").unwrap(), b"v3");
    // Same second on the fixed clock: the second archived version gets a counter.
    assert_eq!(
        f.local
            .content(&format!("docs/_archiv/{PREFIX}notes.md"))
            .unwrap(),
        b"v1"
    );
    assert_eq!(
        f.local
            .content(&format!("docs/_archiv/{PREFIX}notes (2).md"))
            .unwrap(),
        b"v2"
    );
    // The archive is never uploaded.
    assert_eq!(f.remote_names(&folder).await, ["notes.md"]);
    f.assert_stable().await;
}

#[tokio::test]
async fn files_deleted_in_proton_stay_locally() {
    let f = backup_fixture().await;
    let root = f.remote_root().await;
    let file = f.remote_upload(&root, "old.txt", b"keep me").await;
    let folder = f
        .remote
        .create_folder(&root.id, &NodeName::new("gone").unwrap())
        .await
        .unwrap();
    f.remote_upload(&folder, "inner.txt", b"inner").await;
    f.sync().await;

    f.remote.trash(&file.id).await.unwrap();
    f.remote.trash(&folder.id).await.unwrap();
    let report = f.sync().await;

    assert_eq!(report.trashed_local, 0, "{report:?}");
    assert_eq!(f.local.content("old.txt").unwrap(), b"keep me");
    assert_eq!(f.local.content("gone/inner.txt").unwrap(), b"inner");
    assert_eq!(f.local.trash_len(), 0);
    // No longer tracked: nothing left to do, not even an ignored step.
    assert_eq!(f.state.base_entries(&f.pair.id).unwrap(), []);
    f.assert_stable().await;
}

#[tokio::test]
async fn a_file_back_in_proton_replaces_the_kept_copy_via_the_archive() {
    let f = backup_fixture().await;
    let root = f.remote_root().await;
    let file = f.remote_upload(&root, "a.txt", b"first").await;
    f.sync().await;
    f.remote.trash(&file.id).await.unwrap();
    f.sync().await;

    f.remote_upload(&root, "a.txt", b"second").await;
    f.sync().await;

    assert_eq!(f.local.content("a.txt").unwrap(), b"second");
    assert_eq!(
        f.local.content(&format!("_archiv/{PREFIX}a.txt")).unwrap(),
        b"first"
    );
    f.assert_stable().await;
}

#[tokio::test]
async fn local_files_and_edits_are_never_uploaded() {
    let f = backup_fixture().await;
    let root = f.remote_root().await;
    let node = f.remote_upload(&root, "shared.txt", b"remote").await;
    f.sync().await;

    f.local.write("local-only.txt", b"mine", T1);
    f.local.write("shared.txt", b"edited locally", T1);
    let report = f.sync().await;

    assert_eq!(report.uploaded, 0, "{report:?}");
    assert_eq!(f.remote_names(&root).await, ["shared.txt"]);
    assert_eq!(f.remote_content(&root, "shared.txt").await, b"remote");
    assert_eq!(f.local.content("shared.txt").unwrap(), b"edited locally");
    f.assert_stable().await;

    // When Proton changes, the local edit is archived, not lost.
    remote_new_revision(&f, &node, b"remote v2").await;
    f.sync().await;
    assert_eq!(f.local.content("shared.txt").unwrap(), b"remote v2");
    assert_eq!(
        f.local
            .content(&format!("_archiv/{PREFIX}shared.txt"))
            .unwrap(),
        b"edited locally"
    );
    assert_eq!(f.local.content("local-only.txt").unwrap(), b"mine");
}

#[tokio::test]
async fn a_locally_deleted_file_is_restored() {
    let f = backup_fixture().await;
    let root = f.remote_root().await;
    f.remote_upload(&root, "a.txt", b"a").await;
    f.sync().await;

    f.local.remove("a.txt");
    let report = f.sync().await;

    assert_eq!(report.downloaded, 1, "{report:?}");
    assert_eq!(report.trashed_remote, 0, "{report:?}");
    assert_eq!(f.local.content("a.txt").unwrap(), b"a");
    assert_eq!(f.remote_names(&root).await, ["a.txt"]);
    f.assert_stable().await;
}

#[tokio::test]
async fn dry_run_shows_the_archiving() {
    let f = backup_fixture().await;
    let root = f.remote_root().await;
    let node = f.remote_upload(&root, "a.txt", b"v1").await;
    f.sync().await;
    remote_new_revision(&f, &node, b"v2").await;

    let SyncOutcome::Planned(plan) = f.context().run_once(true).await.unwrap() else {
        panic!("dry run expected");
    };

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].action, SyncAction::ArchiveAndDownload);
    // Nothing changed yet.
    assert_eq!(f.local.content("a.txt").unwrap(), b"v1");
}
