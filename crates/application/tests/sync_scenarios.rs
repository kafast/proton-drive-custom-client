//! Scenario tests of the sync engine against the fakes (without network and file system).

// Helper functions in test files may abort like tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{FailingOnUpload, Fixture, T0, T1};
use protonsync_application::ports::{LocalStorage, RemoteError, RemoteStorage, StateRepo};
use protonsync_application::sync::{RemoteCache, SyncContext, SyncError, SyncOutcome, SyncReport};
use protonsync_domain::conflict::{ConflictKind, SyncAction};
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncDirection;
use protonsync_domain::trash::TrashOrigin;

#[tokio::test]
async fn first_run_merges_both_sides() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("local.txt", b"from here", T0);
    f.local.write("folder/deep.txt", b"deep", T0);
    f.remote_upload(&root, "remote.txt", b"from there").await;

    let report = f.sync().await;

    assert_eq!(report.uploaded, 3, "{report:?}");
    assert_eq!(report.downloaded, 1);
    assert_eq!(f.local.content("remote.txt").unwrap(), b"from there");
    assert_eq!(f.remote_content(&root, "local.txt").await, b"from here");
    let folder = f.remote_child(&root, "folder").await.unwrap();
    assert_eq!(f.remote_content(&folder, "deep.txt").await, b"deep");
    f.assert_stable().await;
}

#[tokio::test]
async fn edits_flow_in_both_directions() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("a.txt", b"1", T0);
    let remote_b = f.remote_upload(&root, "b.txt", b"1").await;
    f.sync().await;

    f.local.write("a.txt", b"local new", T1);
    f.remote
        .upload_revision(&remote_b.id, &b"remote new"[..], 10, T1)
        .await
        .unwrap();
    let report = f.sync().await;

    assert_eq!((report.uploaded, report.downloaded), (1, 1));
    assert_eq!(f.remote_content(&root, "a.txt").await, b"local new");
    assert_eq!(f.local.content("b.txt").unwrap(), b"remote new");
    f.assert_stable().await;
}

#[tokio::test]
async fn deletions_go_to_the_trash_on_the_other_side() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("local-gone.txt", b"x", T0);
    f.local.write("remote-gone.txt", b"y", T0);
    f.sync().await;

    f.local.remove("local-gone.txt");
    let gone = f.remote_child(&root, "remote-gone.txt").await.unwrap();
    f.remote.trash(&gone.id).await.unwrap();
    let report = f.sync().await;

    assert_eq!((report.trashed_remote, report.trashed_local), (1, 1));
    assert_eq!(f.remote_names(&root).await, Vec::<String>::new());
    assert_eq!(f.local.paths(), Vec::<String>::new());
    assert_eq!(f.local.trash_len(), 1);
    let trash = f.state.trash_entries(&f.pair.id).unwrap();
    assert_eq!(trash.len(), 2);
    assert!(
        trash
            .iter()
            .any(|(_, e)| matches!(e.origin, TrashOrigin::Remote { .. }))
    );
    assert!(
        trash
            .iter()
            .any(|(_, e)| matches!(e.origin, TrashOrigin::Local { .. }))
    );
    f.assert_stable().await;
}

#[tokio::test]
async fn deleting_a_folder_trashes_it_once() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("folder/a", b"a", T0);
    f.local.write("folder/b", b"b", T0);
    f.sync().await;

    f.local.remove("folder");
    let report = f.sync().await;

    assert_eq!(report.trashed_remote, 1, "{report:?}");
    assert_eq!(f.remote_names(&root).await, Vec::<String>::new());
    f.assert_stable().await;
}

#[tokio::test]
async fn both_sides_edited_keeps_both_versions_and_queues_a_conflict() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("report.txt", b"base", T0);
    f.sync().await;

    f.local.write("report.txt", b"local version", T1);
    let node = f.remote_child(&root, "report.txt").await.unwrap();
    f.remote
        .upload_revision(&node.id, &b"remote version"[..], 14, T1)
        .await
        .unwrap();
    let report = f.sync().await;

    assert_eq!(report.conflicts, 1, "{report:?}");
    let copy = format!("report (Konflikt {}).txt", T1.unix_seconds());
    assert_eq!(f.local.content("report.txt").unwrap(), b"remote version");
    assert_eq!(f.local.content(&copy).unwrap(), b"local version");
    let conflicts = f.state.conflicts(&f.pair.id).unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].1.kind, ConflictKind::BothModified);
    assert_eq!(
        conflicts[0].1.conflict_copy.as_ref().unwrap().as_str(),
        copy
    );

    // The next run uploads the conflict copy; afterwards everything is quiet.
    let report = f.sync().await;
    assert_eq!(report.uploaded, 1);
    assert_eq!(f.remote_content(&root, &copy).await, b"local version");
    f.assert_stable().await;
}

#[tokio::test]
async fn edit_beats_delete_and_is_reported() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("a.txt", b"base", T0);
    f.local.write("b.txt", b"base", T0);
    f.sync().await;

    // a: changed locally, deleted remotely. b: deleted locally, changed remotely.
    f.local.write("a.txt", b"local new", T1);
    let a = f.remote_child(&root, "a.txt").await.unwrap();
    f.remote.trash(&a.id).await.unwrap();
    f.local.remove("b.txt");
    let b = f.remote_child(&root, "b.txt").await.unwrap();
    f.remote
        .upload_revision(&b.id, &b"remote new"[..], 10, T1)
        .await
        .unwrap();
    let report = f.sync().await;

    assert_eq!(report.conflicts, 2, "{report:?}");
    assert_eq!(f.remote_content(&root, "a.txt").await, b"local new");
    assert_eq!(f.local.content("b.txt").unwrap(), b"remote new");
    let kinds: Vec<ConflictKind> = f
        .state
        .conflicts(&f.pair.id)
        .unwrap()
        .into_iter()
        .map(|(_, conflict)| conflict.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            ConflictKind::LocalModifiedRemoteDeleted,
            ConflictKind::LocalDeletedRemoteModified
        ]
    );
    f.assert_stable().await;
}

#[tokio::test]
async fn dry_run_changes_nothing() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("a.txt", b"x", T0);
    f.remote_upload(&root, "b.txt", b"y").await;

    let SyncOutcome::Planned(plan) = f.context().run_once(true).await.unwrap() else {
        unreachable!("dry run")
    };

    let actions: Vec<(String, SyncAction)> = plan
        .steps
        .iter()
        .map(|step| (step.path.to_string(), step.action))
        .collect();
    assert_eq!(
        actions,
        [
            ("a.txt".into(), SyncAction::Upload),
            ("b.txt".into(), SyncAction::Download)
        ]
    );
    assert_eq!(f.local.paths(), ["a.txt"]);
    assert_eq!(f.remote_names(&root).await, ["b.txt"]);
    assert_eq!(f.state.base_entries(&f.pair.id).unwrap(), []);
}

#[tokio::test]
async fn deleted_folder_with_new_remote_content_is_recreated() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("folder/old.txt", b"old", T0);
    f.sync().await;

    f.local.remove("folder");
    let folder = f.remote_child(&root, "folder").await.unwrap();
    f.remote_upload(&folder, "new.txt", b"new").await;
    let report = f.sync().await;

    // Nothing new is lost: the folder comes back with the new file,
    // only the old file deleted locally goes to the trash.
    assert_eq!(f.local.content("folder/new.txt").unwrap(), b"new");
    assert_eq!(f.remote_names(&folder).await, ["new.txt"]);
    assert_eq!(report.trashed_remote, 1, "{report:?}");
    f.assert_stable().await;
}

#[tokio::test]
async fn failed_step_is_reported_and_retried() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("a.txt", b"x", T0);

    // The first calls (cursor, listing) succeed; the upload fails.
    let failing = FailingOnUpload::new(&f.remote);
    failing
        .fail_uploads
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let context = SyncContext {
        remote: &failing,
        local: &f.local,
        state: &f.state,
        clock: &f.clock,
        pair: &f.pair,
    };
    let SyncOutcome::Applied(report) = context.run_once(false).await.unwrap() else {
        unreachable!()
    };
    assert_eq!(report.uploaded, 0);
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].path.as_str(), "a.txt");

    let report = f.sync().await;
    assert_eq!(report.uploaded, 1);
    assert_eq!(f.remote_content(&root, "a.txt").await, b"x");
}

#[tokio::test]
async fn missing_login_aborts_the_run() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    f.remote.fail_next_call(RemoteError::Unauthorized);
    assert_eq!(
        f.context().run_once(false).await,
        Err(SyncError::Remote(RemoteError::Unauthorized))
    );
}

#[tokio::test]
async fn download_only_never_touches_proton() {
    let f = Fixture::new(SyncDirection::DownloadOnly).await;
    let root = f.remote_root().await;
    f.remote_upload(&root, "mirror.txt", b"remote").await;
    f.sync().await;

    f.local.write("mirror.txt", b"messed up locally", T1);
    f.local.write("local-only.txt", b"x", T1);
    let report = f.sync().await;

    assert_eq!(report.ignored, 2, "{report:?}");
    assert_eq!(f.remote_names(&root).await, ["mirror.txt"]);
    assert_eq!(f.remote_content(&root, "mirror.txt").await, b"remote");
}

#[tokio::test]
async fn interrupted_upload_is_recognised_as_identical() {
    // Simulates a crash after the upload but before the base is stored:
    // both sides have the same file, the base does not know it.
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("a.txt", b"same", T0);
    f.remote_upload(&root, "a.txt", b"same").await;

    let report = f.sync().await;

    assert_eq!(report.conflicts, 0, "{report:?}");
    assert_eq!(report.uploaded + report.downloaded, 0);
    assert_eq!(f.state.base_entries(&f.pair.id).unwrap().len(), 1);
    f.assert_stable().await;
}

#[tokio::test]
async fn file_and_folder_with_same_name_are_skipped() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.mkdir("twice");
    f.remote_upload(&root, "twice", b"file").await;

    let report = f.sync().await;

    assert_eq!(report.skipped.len(), 1);
    assert_eq!(
        report.skipped[0].path,
        RelativePath::parse("twice").unwrap()
    );
    assert!(
        f.local
            .entry(&RelativePath::parse("twice").unwrap())
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn cached_runs_pick_up_remote_changes_without_listing_folders() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("folder/local.txt", b"1", T0);
    let mut cache = RemoteCache::new();
    f.sync_cached(&mut cache).await;

    let folder = f.remote_child(&root, "folder").await.unwrap();
    f.remote_upload(&folder, "new.txt", b"from there").await;
    let before = f.remote.list_calls();
    let report = f.sync_cached(&mut cache).await;

    assert_eq!(report.downloaded, 1, "{report:?}");
    assert_eq!(f.remote.list_calls(), before, "no folder was read");
    assert_eq!(f.local.content("folder/new.txt").unwrap(), b"from there");
    // The own uploads are in the cache immediately, otherwise the
    // next run would consider them deleted remotely.
    let report = f.sync_cached(&mut cache).await;
    assert_eq!(report, SyncReport::default());
    f.assert_stable().await;
}

#[tokio::test]
async fn stale_cache_never_deletes_locally() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    let file = f.remote_upload(&root, "important.txt", b"data").await;
    let mut cache = RemoteCache::new();
    f.sync_cached(&mut cache).await;

    // Proton reports the deletion, but the restore gets lost:
    // the cache considers the file deleted, but it is there.
    f.remote.trash(&file.id).await.unwrap();
    cache.refresh(&f.remote, &root.id).await.unwrap();
    f.remote.silence_events(true);
    f.remote.restore(&file.id).await.unwrap();
    f.remote.silence_events(false);

    let report = f.sync_cached(&mut cache).await;
    assert_eq!(report.trashed_local, 0, "{report:?}");
    assert_eq!(report.failures.len(), 1);
    assert!(!cache.is_warm(), "stale cache is discarded");
    assert_eq!(f.local.content("important.txt").unwrap(), b"data");

    // The next run reads completely and finds nothing to do.
    let report = f.sync_cached(&mut cache).await;
    assert_eq!(report, SyncReport::default());
    f.assert_stable().await;
}
