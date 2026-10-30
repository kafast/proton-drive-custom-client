//! Scenario tests of the version archive (option `archive_versions`): nothing is
//! deleted or overwritten on either side; deleted and replaced versions move into
//! the folder `_archiv` next to them, locally and in Proton. The archives are
//! not synced.

// Helper functions in test files may abort like tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{Fixture, T0, T1};
use protonsync_application::ports::{RemoteStorage, StateRepo};
use protonsync_application::sync::{RemoteCache, SyncOutcome, SyncReport};
use protonsync_application::use_cases::ConflictService;
use protonsync_domain::conflict::{ConflictKind, Resolution, SyncAction};
use protonsync_domain::node::{Node, NodeName};
use protonsync_domain::sync_pair::SyncDirection;
use protonsync_domain::time::Timestamp;

/// Archive prefix for the fixture clock (T1 = 2025-10-09 08:55:00 UTC).
const PREFIX: &str = "2025-10-09_08-55-00_";
/// A later modification time for local edits.
const T2: Timestamp = Timestamp::from_unix_seconds(1_760_000_200);

async fn archive_fixture(direction: SyncDirection) -> Fixture {
    let mut f = Fixture::new(direction).await;
    f.pair.archive_versions = true;
    f
}

async fn two_way() -> Fixture {
    archive_fixture(SyncDirection::TwoWay).await
}

fn archived(name: &str) -> String {
    format!("{PREFIX}{name}")
}

/// Simulates a new version uploaded in Proton by another device.
async fn remote_new_revision(f: &Fixture, node: &Node, content: &[u8]) {
    f.remote
        .upload_revision(&node.id, content, content.len() as u64, T0)
        .await
        .unwrap();
}

async fn folder(f: &Fixture, parent: &Node, name: &str) -> Node {
    f.remote_child(parent, name)
        .await
        .unwrap_or_else(|| panic!("remote folder {name} missing"))
}

async fn planned_actions(f: &Fixture) -> Vec<(String, SyncAction)> {
    let SyncOutcome::Planned(plan) = f.context().run_once(true).await.unwrap() else {
        panic!("dry run expected");
    };
    plan.steps
        .into_iter()
        .map(|step| (step.path.to_string(), step.action))
        .collect()
}

// --- Deletions --------------------------------------------------------------

#[tokio::test]
async fn a_local_deletion_moves_the_file_into_the_archive_in_proton() {
    let f = two_way().await;
    f.local.write("a.txt", b"content", T0);
    f.sync().await;
    let root = f.remote_root().await;
    let node = f.remote_child(&root, "a.txt").await.unwrap();

    f.local.remove("a.txt");
    let report = f.sync().await;

    assert_eq!(report.archived_remote, 1, "{report:?}");
    assert_eq!(report.trashed_remote, 0, "{report:?}");
    assert_eq!(f.remote_names(&root).await, ["_archiv"]);
    let archive = folder(&f, &root, "_archiv").await;
    assert_eq!(f.remote_names(&archive).await, [archived("a.txt")]);
    assert_eq!(
        f.remote_content(&archive, &archived("a.txt")).await,
        b"content"
    );
    // The same node was moved, not trashed: its history in Proton stays.
    let moved = f.remote.node(&node.id).await.unwrap();
    assert_eq!(moved.parent, Some(archive.id));
    // The archive in Proton is not downloaded.
    assert_eq!(f.local.paths(), Vec::<String>::new());
    assert_eq!(f.state.base_entries(&f.pair.id).unwrap(), []);
    f.assert_stable().await;
}

#[tokio::test]
async fn a_deletion_in_proton_moves_the_local_file_into_the_archive() {
    let f = two_way().await;
    let root = f.remote_root().await;
    let node = f.remote_upload(&root, "a.txt", b"content").await;
    f.sync().await;

    f.remote.trash(&node.id).await.unwrap();
    let report = f.sync().await;

    assert_eq!(report.archived_local, 1, "{report:?}");
    assert_eq!(report.trashed_local, 0, "{report:?}");
    assert_eq!(f.local.content("a.txt"), None);
    assert_eq!(
        f.local.content(&format!("_archiv/{}", archived("a.txt"))),
        Some(b"content".to_vec())
    );
    assert_eq!(f.local.trash_len(), 0);
    // The local archive is not uploaded.
    assert_eq!(f.remote_names(&root).await, Vec::<String>::new());
    f.assert_stable().await;
}

#[tokio::test]
async fn deleted_folders_are_archived_as_a_whole_on_both_sides() {
    let f = two_way().await;
    f.local.write("mine/a.txt", b"a", T0);
    f.local.write("mine/sub/b.txt", b"b", T0);
    let root = f.remote_root().await;
    let theirs = f
        .remote
        .create_folder(&root.id, &NodeName::new("theirs").unwrap())
        .await
        .unwrap();
    f.remote_upload(&theirs, "c.txt", b"c").await;
    f.sync().await;

    f.local.remove("mine");
    f.remote.trash(&theirs.id).await.unwrap();
    let report = f.sync().await;

    assert_eq!(report.archived_remote, 1, "{report:?}");
    assert_eq!(report.archived_local, 1, "{report:?}");
    // Proton: the folder with all its content sits in the archive.
    let archive = folder(&f, &root, "_archiv").await;
    let mine = folder(&f, &archive, &archived("mine")).await;
    assert_eq!(f.remote_content(&mine, "a.txt").await, b"a");
    let sub = folder(&f, &mine, "sub").await;
    assert_eq!(f.remote_content(&sub, "b.txt").await, b"b");
    // Locally: the same for the folder deleted in Proton.
    let local_archive = format!("_archiv/{}", archived("theirs"));
    assert_eq!(
        f.local.content(&format!("{local_archive}/c.txt")),
        Some(b"c".to_vec())
    );
    f.assert_stable().await;
}

#[tokio::test]
async fn archives_live_next_to_the_file_in_subfolders() {
    let f = two_way().await;
    f.local.write("docs/2026/a.txt", b"a", T0);
    f.sync().await;

    f.local.remove("docs/2026/a.txt");
    f.sync().await;

    let root = f.remote_root().await;
    let docs = folder(&f, &root, "docs").await;
    let year = folder(&f, &docs, "2026").await;
    assert_eq!(f.remote_names(&year).await, ["_archiv"]);
    let archive = folder(&f, &year, "_archiv").await;
    assert_eq!(f.remote_content(&archive, &archived("a.txt")).await, b"a");
    assert_eq!(f.remote_names(&root).await, ["docs"]);
    f.assert_stable().await;
}

// --- Replaced versions ------------------------------------------------------

#[tokio::test]
async fn a_local_edit_archives_the_old_version_in_proton_first() {
    let f = two_way().await;
    f.local.write("report.txt", b"v1", T0);
    f.sync().await;

    f.local.write("report.txt", b"v2 local", T2);
    let report = f.sync().await;

    assert_eq!(report.uploaded, 1, "{report:?}");
    let root = f.remote_root().await;
    assert_eq!(f.remote_content(&root, "report.txt").await, b"v2 local");
    let archive = folder(&f, &root, "_archiv").await;
    assert_eq!(
        f.remote_content(&archive, &archived("report.txt")).await,
        b"v1"
    );
    // Nothing archived locally: the local file is the newest version.
    assert!(!f.local.paths().iter().any(|p| p.starts_with("_archiv")));
    f.assert_stable().await;
}

#[tokio::test]
async fn a_remote_edit_archives_the_old_local_version_first() {
    let f = two_way().await;
    let root = f.remote_root().await;
    let node = f.remote_upload(&root, "report.txt", b"v1").await;
    f.sync().await;

    remote_new_revision(&f, &node, b"v2 remote").await;
    let report = f.sync().await;

    assert_eq!(report.downloaded, 1, "{report:?}");
    assert_eq!(f.local.content("report.txt").unwrap(), b"v2 remote");
    assert_eq!(
        f.local
            .content(&format!("_archiv/{}", archived("report.txt")))
            .unwrap(),
        b"v1"
    );
    assert_eq!(f.remote_names(&root).await, ["report.txt"]);
    f.assert_stable().await;
}

#[tokio::test]
async fn many_versions_pile_up_in_the_archive_in_proton() {
    let f = two_way().await;
    f.local.write("notes.md", b"v1", T0);
    f.sync().await;
    f.local.write("notes.md", b"v2", T1);
    f.sync().await;
    f.local.write("notes.md", b"v3", T2);
    f.sync().await;

    let root = f.remote_root().await;
    assert_eq!(f.remote_content(&root, "notes.md").await, b"v3");
    let archive = folder(&f, &root, "_archiv").await;
    // Same second on the fixed clock: the second archived version gets a counter.
    assert_eq!(
        f.remote_names(&archive).await,
        [archived("notes (2).md"), archived("notes.md")]
    );
    assert_eq!(
        f.remote_content(&archive, &archived("notes.md")).await,
        b"v1"
    );
    assert_eq!(
        f.remote_content(&archive, &archived("notes (2).md")).await,
        b"v2"
    );
    f.assert_stable().await;
}

#[tokio::test]
async fn the_same_content_is_not_archived_again() {
    let f = two_way().await;
    f.local.write("a.txt", b"same", T0);
    let root = f.remote_root().await;
    let other = f.remote_upload(&root, "b.txt", b"same b").await;
    f.sync().await;

    // Only the modification time changes locally; Proton gets a new revision
    // with the same content.
    f.local.write("a.txt", b"same", T2);
    remote_new_revision(&f, &other, b"same b").await;
    let report = f.sync().await;

    assert_eq!(report.uploaded + report.downloaded, 0, "{report:?}");
    assert_eq!(f.remote_names(&root).await, ["a.txt", "b.txt"]);
    assert!(!f.local.paths().iter().any(|p| p.starts_with("_archiv")));
    f.assert_stable().await;
}

#[tokio::test]
async fn new_files_are_transferred_without_archiving() {
    let f = two_way().await;
    f.local.write("local.txt", b"l", T0);
    let root = f.remote_root().await;
    f.remote_upload(&root, "remote.txt", b"r").await;

    let report = f.sync().await;

    assert_eq!((report.uploaded, report.downloaded), (1, 1), "{report:?}");
    assert_eq!(f.remote_names(&root).await, ["local.txt", "remote.txt"]);
    assert_eq!(f.local.paths(), ["local.txt", "remote.txt"]);
    f.assert_stable().await;
}

// --- The archives themselves ------------------------------------------------

#[tokio::test]
async fn the_archives_are_never_synced() {
    let f = two_way().await;
    // An archive folder that already exists on each side, e.g. from earlier runs.
    f.local.write("_archiv/old-local.txt", b"l", T0);
    f.local.write("docs/_archiv/x.txt", b"x", T0);
    let root = f.remote_root().await;
    let archive = f
        .remote
        .create_folder(&root.id, &NodeName::new("_archiv").unwrap())
        .await
        .unwrap();
    f.remote_upload(&archive, "old-remote.txt", b"r").await;

    let report = f.sync().await;

    // Only the folder `docs` itself is new.
    assert_eq!((report.uploaded, report.downloaded), (1, 0), "{report:?}");
    assert_eq!(f.remote_names(&archive).await, ["old-remote.txt"]);
    let docs = folder(&f, &root, "docs").await;
    assert_eq!(f.remote_names(&docs).await, Vec::<String>::new());
    assert_eq!(f.local.content("_archiv/old-remote.txt"), None);
    f.assert_stable().await;

    // Cleaning up an archive by hand is not passed on either.
    f.local.remove("_archiv");
    f.remote.trash(&archive.id).await.unwrap();
    let report = f.sync().await;
    assert_eq!(report.archived_local + report.archived_remote, 0);
    f.assert_stable().await;
}

#[tokio::test]
async fn an_existing_archive_folder_in_proton_is_reused() {
    let f = two_way().await;
    f.local.write("a.txt", b"a", T0);
    f.local.write("b.txt", b"b", T0);
    f.sync().await;
    let root = f.remote_root().await;
    f.local.remove("a.txt");
    f.sync().await;

    f.local.remove("b.txt");
    f.sync().await;

    assert_eq!(f.remote_names(&root).await, ["_archiv"]);
    let archive = folder(&f, &root, "_archiv").await;
    assert_eq!(
        f.remote_names(&archive).await,
        [archived("a.txt"), archived("b.txt")]
    );
}

#[tokio::test]
async fn a_name_taken_in_the_archive_in_proton_is_avoided() {
    let f = two_way().await;
    f.local.write("a.txt", b"current", T0);
    f.sync().await;
    // Left over in the archive under the original name (e.g. copied there by hand).
    let root = f.remote_root().await;
    let archive = f
        .remote
        .create_folder(&root.id, &NodeName::new("_archiv").unwrap())
        .await
        .unwrap();
    f.remote_upload(&archive, "a.txt", b"left over").await;

    f.local.remove("a.txt");
    let report = f.sync().await;

    assert!(report.failures.is_empty(), "{report:?}");
    assert_eq!(
        f.remote_names(&archive).await,
        [archived("a.txt"), "a.txt".to_owned()]
    );
    assert_eq!(
        f.remote_content(&archive, &archived("a.txt")).await,
        b"current"
    );
    assert_eq!(f.remote_content(&archive, "a.txt").await, b"left over");
    f.assert_stable().await;
}

#[tokio::test]
async fn a_file_blocking_the_archive_folder_stops_the_deletion() {
    let f = two_way().await;
    f.local.write("a.txt", b"keep", T0);
    f.sync().await;
    let root = f.remote_root().await;
    // A file (not a folder) named `_archiv` in Proton.
    f.remote_upload(&root, "_archiv", b"not a folder").await;

    f.local.remove("a.txt");
    let report = f.sync().await;

    assert_eq!(report.failures.len(), 1, "{report:?}");
    assert_eq!(report.failures[0].action, SyncAction::ArchiveRemote);
    // Nothing is lost: the file stays in Proton and is retried next time.
    assert_eq!(f.remote_content(&root, "a.txt").await, b"keep");
    assert_eq!(
        planned_actions(&f).await,
        [("a.txt".to_owned(), SyncAction::ArchiveRemote)]
    );

    // Once the obstacle is gone, the next run archives the file.
    let blocker = f.remote_child(&root, "_archiv").await.unwrap();
    f.remote.trash(&blocker.id).await.unwrap();
    let report = f.sync().await;
    assert!(report.failures.is_empty(), "{report:?}");
    let archive = folder(&f, &root, "_archiv").await;
    assert_eq!(
        f.remote_content(&archive, &archived("a.txt")).await,
        b"keep"
    );
    f.assert_stable().await;
}

#[tokio::test]
async fn runs_from_events_keep_the_archive_out_of_the_tree() {
    let f = two_way().await;
    let mut cache = RemoteCache::new();
    f.local.write("a.txt", b"a", T0);
    f.local.write("b.txt", b"b1", T0);
    f.sync_cached(&mut cache).await;

    f.local.remove("a.txt");
    f.local.write("b.txt", b"b2", T2);
    let report = f.sync_cached(&mut cache).await;
    assert_eq!(report.archived_remote, 1, "{report:?}");
    assert_eq!(report.uploaded, 1, "{report:?}");

    // The following event-based runs have nothing left to do.
    let again = f.sync_cached(&mut cache).await;
    assert_eq!(again, SyncReport::default(), "{again:?}");
    assert_eq!(f.local.paths(), ["b.txt"]);
    f.assert_stable().await;
}

// --- Interplay with other rules ---------------------------------------------

#[tokio::test]
async fn conflicts_still_keep_both_versions_without_archiving() {
    let f = two_way().await;
    let root = f.remote_root().await;
    let node = f.remote_upload(&root, "a.txt", b"v1").await;
    f.sync().await;

    f.local.write("a.txt", b"local", T2);
    remote_new_revision(&f, &node, b"remote").await;
    let report = f.sync().await;

    assert_eq!(report.conflicts, 1, "{report:?}");
    assert_eq!(report.archived_local + report.archived_remote, 0);
    let conflicts = f.state.conflicts(&f.pair.id).unwrap();
    assert_eq!(conflicts[0].1.kind, ConflictKind::BothModified);
    assert_eq!(f.local.content("a.txt").unwrap(), b"remote");
}

#[tokio::test]
async fn resolving_a_conflict_archives_the_losing_version_in_proton() {
    let f = two_way().await;
    let root = f.remote_root().await;
    let node = f.remote_upload(&root, "a.txt", b"v1").await;
    f.sync().await;
    f.local.write("a.txt", b"local", T2);
    remote_new_revision(&f, &node, b"remote").await;
    f.sync().await;
    // Uploads the conflict copy.
    f.sync().await;
    let (id, _) = f.state.conflicts(&f.pair.id).unwrap().remove(0);
    let service = ConflictService {
        local: &f.local,
        state: &f.state,
        clock: &f.clock,
        pair: &f.pair.id,
    };

    service.resolve(id, Resolution::KeepLocal).unwrap();
    f.sync().await;

    // The local version won; the remote one is in the archive in Proton.
    assert_eq!(f.remote_content(&root, "a.txt").await, b"local");
    let archive = folder(&f, &root, "_archiv").await;
    let names = f.remote_names(&archive).await;
    assert!(names.contains(&archived("a.txt")), "{names:?}");
    assert_eq!(
        f.remote_content(&archive, &archived("a.txt")).await,
        b"remote"
    );
    f.assert_stable().await;
}

#[tokio::test]
async fn protect_remote_is_stricter_than_the_archive() {
    let mut f = two_way().await;
    f.pair.protect_remote = true;
    f.local.write("a.txt", b"a", T0);
    f.sync().await;

    f.local.remove("a.txt");
    let report = f.sync().await;

    assert_eq!(report.archived_remote, 0, "{report:?}");
    let root = f.remote_root().await;
    assert_eq!(f.remote_names(&root).await, ["a.txt"]);
}

#[tokio::test]
async fn download_only_archives_locally_instead_of_trashing() {
    let f = archive_fixture(SyncDirection::DownloadOnly).await;
    let root = f.remote_root().await;
    let a = f.remote_upload(&root, "a.txt", b"a1").await;
    let b = f.remote_upload(&root, "b.txt", b"b").await;
    f.sync().await;

    remote_new_revision(&f, &a, b"a2").await;
    f.remote.trash(&b.id).await.unwrap();
    let report = f.sync().await;

    assert_eq!(report.archived_local, 1, "{report:?}");
    assert_eq!(f.local.content("a.txt").unwrap(), b"a2");
    assert_eq!(
        f.local.content(&format!("_archiv/{}", archived("a.txt"))),
        Some(b"a1".to_vec())
    );
    assert_eq!(
        f.local.content(&format!("_archiv/{}", archived("b.txt"))),
        Some(b"b".to_vec())
    );
    assert_eq!(f.remote_names(&root).await, ["a.txt"]);
    f.assert_stable().await;
}

#[tokio::test]
async fn upload_only_archives_in_proton_instead_of_trashing() {
    let f = archive_fixture(SyncDirection::UploadOnly).await;
    f.local.write("a.txt", b"a1", T0);
    f.local.write("b.txt", b"b", T0);
    f.sync().await;

    f.local.write("a.txt", b"a2", T2);
    f.local.remove("b.txt");
    let report = f.sync().await;

    assert_eq!(report.archived_remote, 1, "{report:?}");
    let root = f.remote_root().await;
    assert_eq!(f.remote_content(&root, "a.txt").await, b"a2");
    let archive = folder(&f, &root, "_archiv").await;
    assert_eq!(
        f.remote_names(&archive).await,
        [archived("a.txt"), archived("b.txt")]
    );
    assert_eq!(f.local.paths(), ["a.txt"]);
    f.assert_stable().await;
}

#[tokio::test]
async fn without_the_option_deletions_still_go_to_the_trash() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    f.local.write("a.txt", b"a", T0);
    f.sync().await;

    f.local.remove("a.txt");
    let report = f.sync().await;

    assert_eq!(report.trashed_remote, 1, "{report:?}");
    let root = f.remote_root().await;
    assert_eq!(f.remote_names(&root).await, Vec::<String>::new());
}

#[tokio::test]
async fn the_dry_run_shows_the_archiving_and_changes_nothing() {
    let f = two_way().await;
    f.local.write("gone-locally.txt", b"1", T0);
    f.local.write("edited.txt", b"1", T0);
    let root = f.remote_root().await;
    let gone = f.remote_upload(&root, "gone-remotely.txt", b"1").await;
    let changed = f.remote_upload(&root, "changed-remotely.txt", b"1").await;
    f.sync().await;

    f.local.remove("gone-locally.txt");
    f.local.write("edited.txt", b"2", T2);
    f.remote.trash(&gone.id).await.unwrap();
    remote_new_revision(&f, &changed, b"2").await;

    assert_eq!(
        planned_actions(&f).await,
        [
            (
                "changed-remotely.txt".to_owned(),
                SyncAction::ArchiveAndDownload
            ),
            ("edited.txt".to_owned(), SyncAction::ArchiveRemoteAndUpload),
            ("gone-locally.txt".to_owned(), SyncAction::ArchiveRemote),
            ("gone-remotely.txt".to_owned(), SyncAction::ArchiveLocal),
        ]
    );
    assert_eq!(
        f.remote_names(&root).await,
        ["changed-remotely.txt", "edited.txt", "gone-locally.txt"]
    );
    assert!(!f.local.paths().iter().any(|p| p.starts_with("_archiv")));
}
