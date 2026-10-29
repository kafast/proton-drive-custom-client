//! Safety tests of the modes that must never change anything in Proton:
//! `download_only` and `backup_only` (ADR 0019).
//!
//! Every run goes through a [`RecordingRemote`], which records each changing
//! call (create, upload, rename, move, trash, restore, delete). Each test
//! checks two things after every run:
//! - the engine made **no** changing call, and
//! - the remote tree is exactly as before the run (independent of the recorder).
//!
//! The scenarios cover local edits, deletions, renames, conflicts, failures
//! in the middle of a run, restarts, event-based runs and the conflict and
//! trash use cases. A property test at the end mixes random changes on both sides.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::BTreeMap;

use common::{Fixture, T0, T1};
use protonsync_application::ports::{RemoteError, RemoteStorage, StateRepo};
use protonsync_application::sync::{RemoteCache, SyncOutcome, SyncReport};
use protonsync_application::use_cases::{ConflictService, TrashService};
use protonsync_domain::conflict::{ConflictKind, Resolution};
use protonsync_domain::node::{Node, NodeName};
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncDirection;
use protonsync_domain::time::Timestamp;
use protonsync_domain::trash::RetentionPeriod;
use protonsync_testkit::RecordingRemote;

/// The modes under test.
const READ_ONLY_MODES: [SyncDirection; 2] =
    [SyncDirection::DownloadOnly, SyncDirection::BackupOnly];

/// A local edit after the first sync (different time than the downloads).
const T2: Timestamp = Timestamp::from_unix_seconds(1_760_000_200);

/// Runs `scenario` once per read-only mode, each time on a fresh fixture.
async fn in_each_read_only_mode(scenario: impl AsyncFn(Fixture)) {
    for direction in READ_ONLY_MODES {
        scenario(Fixture::new(direction).await).await;
    }
}

/// What a full read of the pair's remote folder yields right now.
async fn remote_tree(f: &Fixture) -> BTreeMap<RelativePath, Node> {
    let mut fresh = RemoteCache::new();
    fresh.refresh(&f.remote, &f.pair.remote_root).await.unwrap();
    fresh.tree().unwrap().clone()
}

/// A run that must leave Proton untouched.
async fn sync_read_only(f: &Fixture) -> SyncReport {
    let before = remote_tree(f).await;
    let (report, writes) = f.sync_recorded().await;
    assert_untouched(f, &before, &writes, &report).await;
    report
}

/// Like [`sync_read_only`], with a cached remote tree (event-based run).
async fn sync_cached_read_only(f: &Fixture, cache: &mut RemoteCache) -> SyncReport {
    let before = remote_tree(f).await;
    let (report, writes) = f.sync_cached_recorded(cache).await;
    assert_untouched(f, &before, &writes, &report).await;
    report
}

async fn assert_untouched(
    f: &Fixture,
    before: &BTreeMap<RelativePath, Node>,
    writes: &[protonsync_testkit::RemoteWrite],
    report: &SyncReport,
) {
    let mode = f.pair.direction;
    assert!(
        writes.is_empty(),
        "{mode:?} changed Proton: {writes:?}\n{report:?}"
    );
    assert_eq!(
        &remote_tree(f).await,
        before,
        "{mode:?}: remote tree changed\n{report:?}"
    );
    assert_eq!(report.uploaded, 0, "{mode:?}: {report:?}");
    assert_eq!(report.trashed_remote, 0, "{mode:?}: {report:?}");
}

fn name(value: &str) -> NodeName {
    NodeName::new(value).unwrap()
}

/// Proton content after the first sync:
/// `a.txt`, `docs/b.txt`, `docs/sub/c.txt`.
struct Synced {
    root: Node,
    a: Node,
    docs: Node,
    b: Node,
    sub: Node,
    c: Node,
}

async fn synced_tree(f: &Fixture) -> Synced {
    let root = f.remote_root().await;
    let a = f.remote_upload(&root, "a.txt", b"a1").await;
    let docs = f
        .remote
        .create_folder(&root.id, &name("docs"))
        .await
        .unwrap();
    let b = f.remote_upload(&docs, "b.txt", b"b1").await;
    let sub = f
        .remote
        .create_folder(&docs.id, &name("sub"))
        .await
        .unwrap();
    let c = f.remote_upload(&sub, "c.txt", b"c1").await;
    let report = sync_read_only(f).await;
    assert_eq!(report.downloaded, 5, "{report:?}");
    assert!(report.failures.is_empty(), "{report:?}");
    Synced {
        root,
        a,
        docs,
        b,
        sub,
        c,
    }
}

/// Simulates a new version uploaded in Proton by another device.
async fn remote_new_revision(f: &Fixture, node: &Node, content: &[u8]) {
    f.remote
        .upload_revision(&node.id, content, content.len() as u64, T0)
        .await
        .unwrap();
}

fn is_backup(f: &Fixture) -> bool {
    f.pair.direction == SyncDirection::BackupOnly
}

/// Content of a local file in the version archive (backup mode) whose name ends with `original`.
fn archived_content(f: &Fixture, folder: &str, original: &str) -> Option<Vec<u8>> {
    let prefix = if folder.is_empty() {
        "_archiv/".to_owned()
    } else {
        format!("{folder}/_archiv/")
    };
    f.local
        .paths()
        .into_iter()
        .find(|path| path.starts_with(&prefix) && path.ends_with(original))
        .and_then(|path| f.local.content(&path))
}

// ---------------------------------------------------------------------------
// Local changes never reach Proton.

#[tokio::test]
async fn new_local_files_and_folders_stay_local() {
    in_each_read_only_mode(async |f| {
        let s = synced_tree(&f).await;
        f.local.write("new.txt", b"new", T2);
        f.local.write("docs/new.txt", b"new", T2);
        f.local.write("fresh/deep/file.txt", b"deep", T2);
        f.local.mkdir("empty");

        for _ in 0..3 {
            sync_read_only(&f).await;
        }

        assert_eq!(f.remote_names(&s.root).await, ["a.txt", "docs"]);
        assert_eq!(f.local.content("fresh/deep/file.txt").unwrap(), b"deep");
        assert_eq!(f.local.content("docs/new.txt").unwrap(), b"new");
    })
    .await;
}

#[tokio::test]
async fn local_edits_are_never_uploaded() {
    in_each_read_only_mode(async |f| {
        let s = synced_tree(&f).await;
        f.local.write("a.txt", b"edited locally", T2);
        f.local.write("docs/sub/c.txt", b"edited too", T2);

        for _ in 0..3 {
            sync_read_only(&f).await;
        }

        assert_eq!(f.remote_content(&s.root, "a.txt").await, b"a1");
        assert_eq!(f.remote_content(&s.sub, "c.txt").await, b"c1");
        // The local edit is not overwritten either, as long as Proton does not change.
        assert_eq!(f.local.content("a.txt").unwrap(), b"edited locally");
    })
    .await;
}

#[tokio::test]
async fn local_deletions_never_delete_in_proton() {
    in_each_read_only_mode(async |f| {
        let s = synced_tree(&f).await;
        f.local.remove("a.txt");
        f.local.remove("docs");

        let report = sync_read_only(&f).await;
        sync_read_only(&f).await;

        assert_eq!(f.remote_names(&s.root).await, ["a.txt", "docs"]);
        assert_eq!(f.remote_names(&s.sub).await, ["c.txt"]);
        if is_backup(&f) {
            // The backup restores what was deleted locally.
            assert_eq!(report.downloaded, 5, "{report:?}");
            assert_eq!(f.local.content("docs/sub/c.txt").unwrap(), b"c1");
        } else {
            assert_eq!(f.local.content("a.txt"), None);
        }
    })
    .await;
}

#[tokio::test]
async fn local_renames_never_rename_or_upload_in_proton() {
    in_each_read_only_mode(async |f| {
        let s = synced_tree(&f).await;
        // A rename looks like "deleted here, new there" to the engine.
        f.local.remove("a.txt");
        f.local.write("renamed.txt", b"a1", T2);
        f.local.remove("docs/sub");
        f.local.write("docs/moved/c.txt", b"c1", T2);

        sync_read_only(&f).await;
        sync_read_only(&f).await;

        assert_eq!(f.remote_names(&s.root).await, ["a.txt", "docs"]);
        assert_eq!(f.remote_names(&s.docs).await, ["b.txt", "sub"]);
        assert_eq!(f.local.content("renamed.txt").unwrap(), b"a1");
    })
    .await;
}

#[tokio::test]
async fn a_file_replaced_by_a_folder_locally_is_skipped() {
    in_each_read_only_mode(async |f| {
        let s = synced_tree(&f).await;
        f.local.remove("a.txt");
        f.local.write("a.txt/inner.txt", b"x", T2);

        let report = sync_read_only(&f).await;

        assert_eq!(report.skipped.len(), 1, "{report:?}");
        assert_eq!(f.remote_names(&s.root).await, ["a.txt", "docs"]);
        assert_eq!(f.local.content("a.txt/inner.txt").unwrap(), b"x");
    })
    .await;
}

#[tokio::test]
async fn same_name_created_on_both_sides_never_uploads() {
    in_each_read_only_mode(async |f| {
        let root = f.remote_root().await;
        f.local.write("same.txt", b"local", T2);
        f.remote_upload(&root, "same.txt", b"remote").await;

        sync_read_only(&f).await;
        sync_read_only(&f).await;

        assert_eq!(f.remote_content(&root, "same.txt").await, b"remote");
        assert_eq!(f.remote_names(&root).await, ["same.txt"]);
        // The local version still exists somewhere locally.
        let kept = f
            .local
            .paths()
            .into_iter()
            .any(|path| f.local.content(&path).as_deref() == Some(b"local".as_slice()));
        assert!(kept, "local version lost: {:?}", f.local.paths());
    })
    .await;
}

// ---------------------------------------------------------------------------
// Changes on both sides.

#[tokio::test]
async fn both_sides_edited_keeps_both_versions_and_uploads_nothing() {
    in_each_read_only_mode(async |f| {
        let s = synced_tree(&f).await;
        f.local.write("docs/b.txt", b"local edit", T2);
        remote_new_revision(&f, &s.b, b"remote edit").await;

        sync_read_only(&f).await;
        sync_read_only(&f).await;

        assert_eq!(f.remote_content(&s.docs, "b.txt").await, b"remote edit");
        assert_eq!(f.local.content("docs/b.txt").unwrap(), b"remote edit");
        let local_version = if is_backup(&f) {
            archived_content(&f, "docs", "b.txt")
        } else {
            let copy = f
                .state
                .conflicts(&f.pair.id)
                .unwrap()
                .pop()
                .and_then(|(_, conflict)| conflict.conflict_copy)
                .expect("conflict copy");
            f.local.content(copy.as_str())
        };
        assert_eq!(local_version.unwrap(), b"local edit");
    })
    .await;
}

#[tokio::test]
async fn both_sides_edited_identically_converges_without_upload() {
    in_each_read_only_mode(async |f| {
        let s = synced_tree(&f).await;
        f.local.write("a.txt", b"same", T2);
        remote_new_revision(&f, &s.a, b"same").await;

        sync_read_only(&f).await;

        assert_eq!(f.state.conflicts(&f.pair.id).unwrap(), []);
        f.assert_stable().await;
    })
    .await;
}

#[tokio::test]
async fn local_edit_of_a_file_deleted_in_proton_is_kept_and_not_uploaded() {
    in_each_read_only_mode(async |f| {
        let s = synced_tree(&f).await;
        f.local.write("a.txt", b"local edit", T2);
        f.remote.trash(&s.a.id).await.unwrap();

        sync_read_only(&f).await;
        sync_read_only(&f).await;

        assert_eq!(f.remote_names(&s.root).await, ["docs"]);
        assert_eq!(f.local.content("a.txt").unwrap(), b"local edit");
    })
    .await;
}

#[tokio::test]
async fn local_deletion_of_a_file_changed_in_proton_downloads_it_again() {
    in_each_read_only_mode(async |f| {
        let s = synced_tree(&f).await;
        f.local.remove("a.txt");
        remote_new_revision(&f, &s.a, b"a2").await;

        sync_read_only(&f).await;

        assert_eq!(f.local.content("a.txt").unwrap(), b"a2");
        assert_eq!(f.remote_content(&s.root, "a.txt").await, b"a2");
    })
    .await;
}

#[tokio::test]
async fn deleted_on_both_sides_is_only_forgotten() {
    in_each_read_only_mode(async |f| {
        let s = synced_tree(&f).await;
        f.local.remove("a.txt");
        f.remote.trash(&s.a.id).await.unwrap();

        sync_read_only(&f).await;

        f.assert_stable().await;
    })
    .await;
}

#[tokio::test]
async fn local_folder_deletion_with_remote_changes_below_never_deletes_in_proton() {
    in_each_read_only_mode(async |f| {
        let s = synced_tree(&f).await;
        f.local.remove("docs");
        remote_new_revision(&f, &s.c, b"c2").await;

        sync_read_only(&f).await;
        sync_read_only(&f).await;

        assert_eq!(f.local.content("docs/sub/c.txt").unwrap(), b"c2");
        assert_eq!(f.remote_names(&s.docs).await, ["b.txt", "sub"]);
    })
    .await;
}

#[tokio::test]
async fn remote_folder_deletion_with_local_changes_below_never_recreates_it_in_proton() {
    in_each_read_only_mode(async |f| {
        let s = synced_tree(&f).await;
        f.local.write("docs/sub/c.txt", b"local edit", T2);
        f.local.write("docs/new.txt", b"new", T2);
        f.remote.trash(&s.docs.id).await.unwrap();

        sync_read_only(&f).await;
        sync_read_only(&f).await;

        assert_eq!(f.remote_names(&s.root).await, ["a.txt"]);
        assert_eq!(f.local.content("docs/sub/c.txt").unwrap(), b"local edit");
        assert_eq!(f.local.content("docs/new.txt").unwrap(), b"new");
    })
    .await;
}

#[tokio::test]
async fn remote_renames_and_moves_are_followed_locally_only() {
    in_each_read_only_mode(async |f| {
        let s = synced_tree(&f).await;
        f.remote
            .rename(&s.a.id, &name("a-renamed.txt"))
            .await
            .unwrap();
        f.remote.move_node(&s.c.id, &s.root.id).await.unwrap();

        sync_read_only(&f).await;

        assert_eq!(f.local.content("a-renamed.txt").unwrap(), b"a1");
        assert_eq!(f.local.content("c.txt").unwrap(), b"c1");
        if !is_backup(&f) {
            assert_eq!(f.local.content("a.txt"), None);
        }
        assert_eq!(f.remote_names(&s.sub).await, Vec::<String>::new());
    })
    .await;
}

// ---------------------------------------------------------------------------
// Failures, restarts and event-based runs.

#[tokio::test]
async fn a_network_loss_during_downloads_never_leads_to_writes() {
    in_each_read_only_mode(async |f| {
        let root = f.remote_root().await;
        for index in 0..5 {
            f.remote_upload(&root, &format!("f{index}.txt"), b"x").await;
        }
        let recorder = RecordingRemote::new(&f.remote);
        recorder.fail_next_downloads(RemoteError::Unavailable("network down".into()), 3);

        let report = f.sync_through(&recorder).await;

        assert_eq!(report.failures.len(), 3, "{report:?}");
        assert_eq!(recorder.writes(), []);
        // Restart: the next run catches up without writing.
        let report = sync_read_only(&f).await;
        assert_eq!(report.downloaded, 3, "{report:?}");
        f.assert_stable().await;
    })
    .await;
}

#[tokio::test]
async fn an_expired_login_aborts_the_run_without_writes() {
    in_each_read_only_mode(async |f| {
        let s = synced_tree(&f).await;
        remote_new_revision(&f, &s.a, b"a2").await;
        f.local.write("docs/b.txt", b"local edit", T2);
        let recorder = RecordingRemote::new(&f.remote);
        recorder.fail_next_downloads(RemoteError::Unauthorized, 1);

        let result = f.context_with(&recorder).run_once(false).await;

        assert!(result.is_err(), "{result:?}");
        assert_eq!(recorder.writes(), []);
        // After the new login everything is caught up, still without writes.
        sync_read_only(&f).await;
        assert_eq!(f.local.content("a.txt").unwrap(), b"a2");
        assert_eq!(f.remote_content(&s.docs, "b.txt").await, b"b1");
    })
    .await;
}

#[tokio::test]
async fn proton_unreachable_at_start_changes_nothing_anywhere() {
    in_each_read_only_mode(async |f| {
        synced_tree(&f).await;
        let local_before = f.local.paths();
        f.remote
            .fail_next_call(RemoteError::Unavailable("network down".into()));
        let recorder = RecordingRemote::new(&f.remote);

        let result = f.context_with(&recorder).run_once(false).await;

        assert!(result.is_err(), "{result:?}");
        assert_eq!(recorder.writes(), []);
        assert_eq!(f.local.paths(), local_before);
    })
    .await;
}

#[tokio::test]
async fn event_based_runs_never_write_even_with_lost_events_or_gaps() {
    in_each_read_only_mode(async |f| {
        let s = synced_tree(&f).await;
        let mut cache = RemoteCache::new();
        sync_cached_read_only(&f, &mut cache).await;

        // Normal events.
        remote_new_revision(&f, &s.a, b"a2").await;
        f.local.write("docs/b.txt", b"local edit", T2);
        f.local.remove("docs/sub");
        sync_cached_read_only(&f, &mut cache).await;

        // Lost events: the cache believes c.txt is still there.
        f.remote.silence_events(true);
        f.remote.trash(&s.sub.id).await.unwrap();
        f.remote.silence_events(false);
        f.local.write("local-only.txt", b"x", T2);
        sync_cached_read_only(&f, &mut cache).await;

        // Gap in the event stream: full re-read.
        f.remote.push_gap();
        sync_cached_read_only(&f, &mut cache).await;
        sync_cached_read_only(&f, &mut cache).await;

        assert_eq!(f.local.content("a.txt").unwrap(), b"a2");
        assert_eq!(f.remote_content(&s.docs, "b.txt").await, b"b1");
    })
    .await;
}

#[tokio::test]
async fn a_restart_after_many_runs_never_writes() {
    in_each_read_only_mode(async |f| {
        let s = synced_tree(&f).await;
        let mut cache = RemoteCache::new();
        for round in 0..5_u8 {
            f.local.write(&format!("local{round}.txt"), &[round], T2);
            remote_new_revision(&f, &s.b, &[b'b', round]).await;
            sync_cached_read_only(&f, &mut cache).await;
        }
        // Restart: cache gone, only the stored state is left.
        drop(cache);
        sync_read_only(&f).await;
        assert_eq!(f.local.content("docs/b.txt").unwrap(), [b'b', 4]);
        assert_eq!(
            f.remote_names(&s.root).await,
            ["a.txt", "docs"],
            "local files must stay local"
        );
    })
    .await;
}

#[tokio::test]
async fn a_dry_run_never_writes_anywhere() {
    in_each_read_only_mode(async |f| {
        let s = synced_tree(&f).await;
        f.local.write("new.txt", b"new", T2);
        f.local.remove("a.txt");
        f.local.write("docs/b.txt", b"edit", T2);
        remote_new_revision(&f, &s.b, b"b2").await;
        let local_before = f.local.paths();
        let recorder = RecordingRemote::new(&f.remote);

        let outcome = f.context_with(&recorder).run_once(true).await.unwrap();

        assert!(matches!(outcome, SyncOutcome::Planned(_)));
        assert_eq!(recorder.writes(), []);
        assert_eq!(f.local.paths(), local_before);
        assert_eq!(f.local.content("docs/b.txt").unwrap(), b"edit");
    })
    .await;
}

// ---------------------------------------------------------------------------
// Use cases around the sync: conflicts and trash.

#[tokio::test]
async fn resolving_conflicts_in_download_only_never_writes() {
    let resolutions = [
        Resolution::KeepLocal,
        Resolution::KeepRemote,
        Resolution::KeepBoth,
    ];
    for resolution in resolutions {
        let f = Fixture::new(SyncDirection::DownloadOnly).await;
        let s = synced_tree(&f).await;
        // Three kinds of conflict at once.
        f.local.write("a.txt", b"local edit", T2);
        remote_new_revision(&f, &s.a, b"a2").await;
        f.local.write("docs/b.txt", b"local edit", T2);
        f.remote.trash(&s.b.id).await.unwrap();
        f.local.remove("docs/sub/c.txt");
        remote_new_revision(&f, &s.c, b"c2").await;
        sync_read_only(&f).await;

        let service = ConflictService {
            local: &f.local,
            state: &f.state,
            clock: &f.clock,
            pair: &f.pair.id,
        };
        let conflicts = service.list().unwrap();
        let kinds: Vec<ConflictKind> = conflicts.iter().map(|(_, c)| c.kind).collect();
        assert!(kinds.contains(&ConflictKind::BothModified), "{kinds:?}");
        for (id, _) in conflicts {
            service.resolve(id, resolution).unwrap();
        }

        for _ in 0..2 {
            sync_read_only(&f).await;
        }
        assert_eq!(f.remote_content(&s.root, "a.txt").await, b"a2");
        assert_eq!(f.remote_names(&s.docs).await, ["sub"]);
    }
}

#[tokio::test]
async fn trash_restore_and_expiry_never_write_in_read_only_modes() {
    in_each_read_only_mode(async |f| {
        let s = synced_tree(&f).await;
        // Deleted in Proton → local trash (download_only only; backup keeps the copy).
        f.remote.trash(&s.a.id).await.unwrap();
        sync_read_only(&f).await;
        let recorder = RecordingRemote::new(&f.remote);
        let trash = TrashService {
            remote: &recorder,
            local: &f.local,
            state: &f.state,
            clock: &f.clock,
            pair: &f.pair.id,
        };
        let entries = trash.list().unwrap();
        assert!(
            entries.iter().all(|(_, entry)| matches!(
                entry.origin,
                protonsync_domain::trash::TrashOrigin::Local { .. }
            )),
            "only local trash entries expected: {entries:?}"
        );
        for (id, _) in entries {
            trash.restore(id).await.unwrap();
        }
        f.clock.advance_days(400);
        trash
            .purge_expired(RetentionPeriod::days(30).unwrap(), false)
            .await
            .unwrap();
        assert_eq!(recorder.writes(), []);
        sync_read_only(&f).await;
        assert_eq!(f.remote_names(&s.root).await, ["docs"]);
    })
    .await;
}

// ---------------------------------------------------------------------------
// Property test: random changes on both sides, many runs.

mod random {
    use proptest::prelude::*;
    use protonsync_application::use_cases::ensure_folder;

    use super::*;

    /// Paths that random operations work on. `d` is used as a file and as a
    /// folder, so that kind conflicts arise too.
    const PATHS: [&str; 7] = [
        "a.txt",
        "b.txt",
        "d",
        "d/a.txt",
        "d/b.txt",
        "d/e/c.txt",
        "f/x.txt",
    ];

    #[derive(Debug, Clone)]
    enum Op {
        LocalWrite(usize, u8),
        LocalRemove(usize),
        LocalRename(usize, usize),
        RemoteWrite(usize, u8),
        RemoteTrash(usize),
        RemoteRename(usize, usize),
        Sync,
        SyncCached,
    }

    fn op() -> impl Strategy<Value = Op> {
        let path = 0..PATHS.len();
        prop_oneof![
            (path.clone(), any::<u8>()).prop_map(|(p, c)| Op::LocalWrite(p, c)),
            path.clone().prop_map(Op::LocalRemove),
            (path.clone(), path.clone()).prop_map(|(a, b)| Op::LocalRename(a, b)),
            (path.clone(), any::<u8>()).prop_map(|(p, c)| Op::RemoteWrite(p, c)),
            path.clone().prop_map(Op::RemoteTrash),
            (path.clone(), path).prop_map(|(a, b)| Op::RemoteRename(a, b)),
            Just(Op::Sync),
            Just(Op::SyncCached),
        ]
    }

    fn parse(path: &str) -> RelativePath {
        RelativePath::parse(path).unwrap()
    }

    /// Another device writes `path` in Proton (creating folders as needed).
    /// Silently does nothing if the path is blocked by an entry of the other kind.
    async fn remote_write(f: &Fixture, path: &str, content: u8) {
        let path = parse(path);
        let root = f.remote.node(&f.pair.remote_root).await.unwrap();
        let parent_path = path.parent().unwrap_or_default();
        let Ok(parent) = ensure_folder(&f.remote, &root, &parent_path).await else {
            return;
        };
        let file_name = path.file_name().unwrap();
        let existing = f
            .remote
            .list_children(&parent.id)
            .await
            .unwrap()
            .into_iter()
            .find(|node| node.name == file_name);
        let data = [content, 0xAA];
        match existing {
            Some(node) if node.is_folder() => {}
            Some(node) => {
                f.remote
                    .upload_revision(&node.id, &data[..], 2, T0)
                    .await
                    .unwrap();
            }
            None => {
                f.remote
                    .upload_file(&parent.id, &file_name, &data[..], 2, T0)
                    .await
                    .unwrap();
            }
        }
    }

    async fn remote_node(f: &Fixture, path: &str) -> Option<Node> {
        remote_tree(f).await.get(&parse(path)).cloned()
    }

    async fn apply(f: &Fixture, cache: &mut RemoteCache, op: &Op, step: i64) {
        // Every local write gets its own time, so that it counts as a change.
        let now = Timestamp::from_unix_seconds(T1.unix_seconds() + 1000 + step);
        match op {
            Op::LocalWrite(p, content) => f.local.write(PATHS[*p], &[*content], now),
            Op::LocalRemove(p) => f.local.remove(PATHS[*p]),
            Op::LocalRename(from, to) => {
                if let Some(content) = f.local.content(PATHS[*from]) {
                    f.local.remove(PATHS[*from]);
                    f.local.write(PATHS[*to], &content, now);
                }
            }
            Op::RemoteWrite(p, content) => remote_write(f, PATHS[*p], *content).await,
            Op::RemoteTrash(p) => {
                if let Some(node) = remote_node(f, PATHS[*p]).await {
                    f.remote.trash(&node.id).await.unwrap();
                }
            }
            Op::RemoteRename(from, to) => {
                let (from, to) = (parse(PATHS[*from]), parse(PATHS[*to]));
                if from.parent() == to.parent()
                    && let Some(node) = remote_node(f, from.as_str()).await
                {
                    // Renaming onto an existing name is refused by Proton; that is fine here.
                    let _ = f.remote.rename(&node.id, &to.file_name().unwrap()).await;
                }
            }
            Op::Sync => {
                sync_read_only(f).await;
            }
            Op::SyncCached => {
                sync_cached_read_only(f, cache).await;
            }
        }
    }

    /// All local file contents (for the backup invariant).
    fn local_contents(f: &Fixture) -> Vec<Vec<u8>> {
        f.local
            .paths()
            .iter()
            .filter_map(|path| f.local.content(path))
            .collect()
    }

    fn run_scenario(direction: SyncDirection, ops: &[Op]) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            let f = Fixture::new(direction).await;
            let mut cache = RemoteCache::new();
            for (step, op) in (0_i64..).zip(ops) {
                let before = local_contents(&f);
                apply(&f, &mut cache, op, step).await;
                if direction == SyncDirection::BackupOnly && matches!(op, Op::Sync | Op::SyncCached)
                {
                    // The backup never loses a local version: each content is still somewhere.
                    let after = local_contents(&f);
                    for content in before {
                        assert!(
                            after.contains(&content),
                            "backup lost local content {content:?} at {op:?}"
                        );
                    }
                }
            }
            // A final run must not write either.
            sync_read_only(&f).await;
        });
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn download_only_never_changes_proton(ops in prop::collection::vec(op(), 1..30)) {
            run_scenario(SyncDirection::DownloadOnly, &ops);
        }

        #[test]
        fn backup_only_never_changes_proton_nor_loses_local_versions(
            ops in prop::collection::vec(op(), 1..30)
        ) {
            run_scenario(SyncDirection::BackupOnly, &ops);
        }
    }
}
