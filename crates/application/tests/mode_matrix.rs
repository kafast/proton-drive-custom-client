//! What each sync mode does with each kind of change, as one table.
//!
//! The matrix runs every single change (and the typical conflicts) in every
//! direction, with and without the remote protection (`protect_remote`), and
//! checks exactly which changing calls reach Proton and what happens locally.
//! A property test then proves for the protected modes that nothing that
//! exists in Proton is ever deleted, renamed or replaced.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::BTreeMap;

use common::{Fixture, T0, T1};
use protonsync_application::ports::RemoteStorage;
use protonsync_application::sync::RemoteCache;
use protonsync_domain::node::{Node, NodeName};
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncDirection;
use protonsync_domain::time::Timestamp;
use protonsync_testkit::RemoteWrite;

use SyncDirection::{BackupOnly, DownloadOnly, TwoWay, UploadOnly};

const T2: Timestamp = Timestamp::from_unix_seconds(1_760_000_200);

/// The changes of the matrix, applied after a first sync of `a.txt` (content `a1`).
#[derive(Debug, Clone, Copy)]
enum Scenario {
    LocalNew,
    LocalEdit,
    LocalDelete,
    RemoteNew,
    RemoteEdit,
    RemoteDelete,
    BothEdit,
    LocalEditRemoteDelete,
    LocalDeleteRemoteEdit,
}

/// Kind of a changing call, without the identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Write {
    CreateFile,
    NewRevision,
    Trash,
}

/// One row: mode, change, expected calls in Proton, expected local content of `a.txt`.
struct Row {
    direction: SyncDirection,
    protect_remote: bool,
    scenario: Scenario,
    writes: &'static [Write],
    local_a: Option<&'static [u8]>,
}

const fn row(
    direction: SyncDirection,
    protect_remote: bool,
    scenario: Scenario,
    writes: &'static [Write],
    local_a: Option<&'static [u8]>,
) -> Row {
    Row {
        direction,
        protect_remote,
        scenario,
        writes,
        local_a,
    }
}

const NONE: &[Write] = &[];
const CREATE: &[Write] = &[Write::CreateFile];
const REVISION: &[Write] = &[Write::NewRevision];
const TRASH: &[Write] = &[Write::Trash];
const A1: Option<&[u8]> = Some(b"a1");
const A2: Option<&[u8]> = Some(b"a2");
const LOCAL: Option<&[u8]> = Some(b"local");
const GONE: Option<&[u8]> = None;

/// The whole table. Reading it row by row documents the modes.
fn matrix() -> Vec<Row> {
    use Scenario::{
        BothEdit, LocalDelete, LocalDeleteRemoteEdit, LocalEdit, LocalEditRemoteDelete, LocalNew,
        RemoteDelete, RemoteEdit, RemoteNew,
    };
    vec![
        // two_way: everything flows, conflicts keep both versions.
        row(TwoWay, false, LocalNew, CREATE, A1),
        row(TwoWay, false, LocalEdit, REVISION, LOCAL),
        row(TwoWay, false, LocalDelete, TRASH, GONE),
        row(TwoWay, false, RemoteNew, NONE, A1),
        row(TwoWay, false, RemoteEdit, NONE, A2),
        row(TwoWay, false, RemoteDelete, NONE, GONE),
        row(TwoWay, false, BothEdit, NONE, A2),
        row(TwoWay, false, LocalEditRemoteDelete, CREATE, LOCAL),
        row(TwoWay, false, LocalDeleteRemoteEdit, NONE, A2),
        // two_way + protect_remote: new files go up, nothing is replaced or deleted.
        row(TwoWay, true, LocalNew, CREATE, A1),
        row(TwoWay, true, LocalEdit, NONE, A1),
        row(TwoWay, true, LocalDelete, NONE, GONE),
        row(TwoWay, true, RemoteNew, NONE, A1),
        row(TwoWay, true, RemoteEdit, NONE, A2),
        row(TwoWay, true, RemoteDelete, NONE, GONE),
        row(TwoWay, true, BothEdit, NONE, A2),
        row(TwoWay, true, LocalEditRemoteDelete, CREATE, LOCAL),
        row(TwoWay, true, LocalDeleteRemoteEdit, NONE, A2),
        // upload_only: remote changes are ignored, unless the local side changed too.
        row(UploadOnly, false, LocalNew, CREATE, A1),
        row(UploadOnly, false, LocalEdit, REVISION, LOCAL),
        row(UploadOnly, false, LocalDelete, TRASH, GONE),
        row(UploadOnly, false, RemoteNew, NONE, A1),
        row(UploadOnly, false, RemoteEdit, NONE, A1),
        row(UploadOnly, false, RemoteDelete, NONE, A1),
        row(UploadOnly, false, BothEdit, NONE, A2),
        row(UploadOnly, false, LocalEditRemoteDelete, CREATE, LOCAL),
        row(UploadOnly, false, LocalDeleteRemoteEdit, NONE, A2),
        // upload_only + protect_remote.
        row(UploadOnly, true, LocalNew, CREATE, A1),
        row(UploadOnly, true, LocalEdit, NONE, A1),
        row(UploadOnly, true, LocalDelete, NONE, GONE),
        row(UploadOnly, true, RemoteNew, NONE, A1),
        row(UploadOnly, true, RemoteEdit, NONE, A1),
        row(UploadOnly, true, RemoteDelete, NONE, A1),
        row(UploadOnly, true, BothEdit, NONE, A2),
        row(UploadOnly, true, LocalEditRemoteDelete, CREATE, LOCAL),
        row(UploadOnly, true, LocalDeleteRemoteEdit, NONE, A2),
        // download_only: Proton is never changed.
        row(DownloadOnly, false, LocalNew, NONE, A1),
        row(DownloadOnly, false, LocalEdit, NONE, LOCAL),
        row(DownloadOnly, false, LocalDelete, NONE, GONE),
        row(DownloadOnly, false, RemoteNew, NONE, A1),
        row(DownloadOnly, false, RemoteEdit, NONE, A2),
        row(DownloadOnly, false, RemoteDelete, NONE, GONE),
        row(DownloadOnly, false, BothEdit, NONE, A2),
        row(DownloadOnly, false, LocalEditRemoteDelete, NONE, LOCAL),
        row(DownloadOnly, false, LocalDeleteRemoteEdit, NONE, A2),
        // backup_only: Proton is never changed, nothing is deleted locally.
        row(BackupOnly, false, LocalNew, NONE, A1),
        row(BackupOnly, false, LocalEdit, NONE, LOCAL),
        row(BackupOnly, false, LocalDelete, NONE, A1),
        row(BackupOnly, false, RemoteNew, NONE, A1),
        row(BackupOnly, false, RemoteEdit, NONE, A2),
        row(BackupOnly, false, RemoteDelete, NONE, A1),
        row(BackupOnly, false, BothEdit, NONE, A2),
        row(BackupOnly, false, LocalEditRemoteDelete, NONE, LOCAL),
        row(BackupOnly, false, LocalDeleteRemoteEdit, NONE, A2),
    ]
}

async fn fixture(direction: SyncDirection, protect_remote: bool) -> (Fixture, Node) {
    let mut f = Fixture::new(direction).await;
    f.pair.protect_remote = protect_remote;
    let root = f.remote_root().await;
    // Start: the same `a.txt` on both sides, recorded as synced.
    let a = f.remote_upload(&root, "a.txt", b"a1").await;
    f.local.write("a.txt", b"a1", T0);
    f.sync().await;
    (f, a)
}

async fn remote_edit(f: &Fixture, node: &Node) {
    f.remote
        .upload_revision(&node.id, &b"a2"[..], 2, T1)
        .await
        .unwrap();
}

async fn apply(f: &Fixture, a: &Node, scenario: Scenario) {
    let root = f.remote_root().await;
    match scenario {
        Scenario::LocalNew => f.local.write("new.txt", b"new", T2),
        Scenario::LocalEdit => f.local.write("a.txt", b"local", T2),
        Scenario::LocalDelete => f.local.remove("a.txt"),
        Scenario::RemoteNew => {
            f.remote_upload(&root, "remote.txt", b"remote").await;
        }
        Scenario::RemoteEdit => remote_edit(f, a).await,
        Scenario::RemoteDelete => f.remote.trash(&a.id).await.unwrap(),
        Scenario::BothEdit => {
            f.local.write("a.txt", b"local", T2);
            remote_edit(f, a).await;
        }
        Scenario::LocalEditRemoteDelete => {
            f.local.write("a.txt", b"local", T2);
            f.remote.trash(&a.id).await.unwrap();
        }
        Scenario::LocalDeleteRemoteEdit => {
            f.local.remove("a.txt");
            remote_edit(f, a).await;
        }
    }
}

fn kind(write: &RemoteWrite) -> Write {
    match write {
        RemoteWrite::UploadFile { .. } => Write::CreateFile,
        RemoteWrite::UploadRevision(_) => Write::NewRevision,
        RemoteWrite::Trash(_) => Write::Trash,
        other => panic!("unexpected call in the matrix: {other:?}"),
    }
}

#[tokio::test]
async fn every_mode_handles_every_change_as_documented() {
    let mut mismatches = Vec::new();
    for row in matrix() {
        let (f, a) = fixture(row.direction, row.protect_remote).await;
        apply(&f, &a, row.scenario).await;

        let (report, writes) = f.sync_recorded().await;
        // A second run must not change Proton any further.
        let (_, writes_again) = f.sync_recorded().await;

        let kinds: Vec<Write> = writes.iter().map(kind).collect();
        let local_a = f.local.content("a.txt");
        let label = format!(
            "{:?} protect={} {:?}",
            row.direction, row.protect_remote, row.scenario
        );
        if kinds != row.writes {
            mismatches.push(format!(
                "{label}: calls {kinds:?}, expected {:?}",
                row.writes
            ));
        }
        if local_a.as_deref() != row.local_a {
            mismatches.push(format!(
                "{label}: local a.txt {local_a:?}, expected {:?}",
                row.local_a
            ));
        }
        // The second run may only upload the conflict copy, which is a new local file.
        let unexpected_again: Vec<&RemoteWrite> = writes_again
            .iter()
            .filter(|write| {
                !matches!(write, RemoteWrite::UploadFile { name, .. } if name.contains("(Konflikt "))
            })
            .collect();
        if !unexpected_again.is_empty() {
            mismatches.push(format!("{label}: second run wrote {unexpected_again:?}"));
        }
        if !report.failures.is_empty() {
            mismatches.push(format!("{label}: failures {:?}", report.failures));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn the_matrix_covers_every_mode_and_change() {
    let rows = matrix();
    for (direction, protect) in [
        (TwoWay, false),
        (TwoWay, true),
        (UploadOnly, false),
        (UploadOnly, true),
        (DownloadOnly, false),
        (BackupOnly, false),
    ] {
        let count = rows
            .iter()
            .filter(|row| row.direction == direction && row.protect_remote == protect)
            .count();
        assert_eq!(count, 9, "{direction:?} protect={protect}");
    }
}

// ---------------------------------------------------------------------------
// Property test: `protect_remote` never deletes, renames or replaces anything in Proton.

mod protected {
    use proptest::prelude::*;
    use protonsync_application::use_cases::ensure_folder;

    use super::*;

    const PATHS: [&str; 6] = ["a.txt", "b.txt", "d", "d/a.txt", "d/e/c.txt", "f/x.txt"];

    #[derive(Debug, Clone)]
    enum Op {
        LocalWrite(usize, u8),
        LocalRemove(usize),
        LocalRename(usize, usize),
        RemoteWrite(usize, u8),
        RemoteTrash(usize),
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
            path.prop_map(Op::RemoteTrash),
            Just(Op::Sync),
            Just(Op::SyncCached),
        ]
    }

    fn parse(path: &str) -> RelativePath {
        RelativePath::parse(path).unwrap()
    }

    async fn remote_tree(f: &Fixture) -> BTreeMap<RelativePath, Node> {
        let mut fresh = RemoteCache::new();
        fresh.refresh(&f.remote, &f.pair.remote_root).await.unwrap();
        fresh.tree().unwrap().clone()
    }

    /// Another device writes `path` in Proton; does nothing if blocked by a folder.
    async fn remote_write(f: &Fixture, path: &str, content: u8) {
        let path = parse(path);
        let root = f.remote.node(&f.pair.remote_root).await.unwrap();
        let Ok(parent) = ensure_folder(&f.remote, &root, &path.parent().unwrap_or_default()).await
        else {
            return;
        };
        let file_name: NodeName = path.file_name().unwrap();
        let existing = f
            .remote
            .list_children(&parent.id)
            .await
            .unwrap()
            .into_iter()
            .find(|node| node.name == file_name);
        let data = [content, 0x55];
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

    /// A run may only add to Proton: every node from before is still there, unchanged.
    async fn protected_sync(f: &Fixture, cache: Option<&mut RemoteCache>) {
        let before = remote_tree(f).await;
        let (report, writes) = match cache {
            Some(cache) => f.sync_cached_recorded(cache).await,
            None => f.sync_recorded().await,
        };
        for write in &writes {
            assert!(
                matches!(
                    write,
                    RemoteWrite::CreateFolder { .. } | RemoteWrite::UploadFile { .. }
                ),
                "{:?} with protect_remote changed Proton: {write:?}\n{report:?}",
                f.pair.direction
            );
        }
        let after = remote_tree(f).await;
        for (path, node) in &before {
            assert_eq!(
                after.get(path),
                Some(node),
                "{path:?} was changed or removed in Proton\n{report:?}"
            );
        }
    }

    fn run_scenario(direction: SyncDirection, ops: &[Op]) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut f = Fixture::new(direction).await;
            f.pair.protect_remote = true;
            let mut cache = RemoteCache::new();
            for (step, op) in (0_i64..).zip(ops) {
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
                    Op::RemoteWrite(p, content) => remote_write(&f, PATHS[*p], *content).await,
                    Op::RemoteTrash(p) => {
                        if let Some(node) = remote_tree(&f).await.get(&parse(PATHS[*p])) {
                            f.remote.trash(&node.id).await.unwrap();
                        }
                    }
                    Op::Sync => protected_sync(&f, None).await,
                    Op::SyncCached => protected_sync(&f, Some(&mut cache)).await,
                }
            }
            protected_sync(&f, None).await;
        });
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn two_way_with_protection_only_ever_adds_to_proton(
            ops in prop::collection::vec(op(), 1..30)
        ) {
            run_scenario(TwoWay, &ops);
        }

        #[test]
        fn upload_only_with_protection_only_ever_adds_to_proton(
            ops in prop::collection::vec(op(), 1..30)
        ) {
            run_scenario(UploadOnly, &ops);
        }
    }
}
