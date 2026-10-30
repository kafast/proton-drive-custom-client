//! Scenario tests for the trash and conflict resolution.

#![allow(clippy::unwrap_used)]

mod common;

use common::{Fixture, T1};
use protonsync_application::ports::{RemoteStorage, StateRepo};
use protonsync_application::use_cases::{ConflictService, TrashService, UseCaseError};
use protonsync_domain::conflict::Resolution;
use protonsync_domain::sync_pair::SyncDirection;
use protonsync_domain::trash::RetentionPeriod;
use protonsync_testkit::{FakeLocal, FakeRemote, FixedClock, InMemoryStateRepo};

fn trash(f: &Fixture) -> TrashService<'_, FakeRemote, FakeLocal, InMemoryStateRepo, FixedClock> {
    TrashService {
        remote: &f.remote,
        local: &f.local,
        state: &f.state,
        clock: &f.clock,
        pair: &f.pair.id,
    }
}

fn conflicts(f: &Fixture) -> ConflictService<'_, FakeLocal, InMemoryStateRepo, FixedClock> {
    ConflictService {
        local: &f.local,
        state: &f.state,
        clock: &f.clock,
        pair: &f.pair.id,
    }
}

/// Creates a synced file and deletes it locally, and another one remotely.
async fn fixture_with_trash() -> Fixture {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("folder/local.txt", b"local", common::T0);
    f.local.write("remote.txt", b"remote", common::T0);
    f.sync().await;
    // Deleted locally → Proton trash; deleted remotely → local trash.
    f.local.remove("folder");
    let remote_file = f.remote_child(&root, "remote.txt").await.unwrap();
    f.remote.trash(&remote_file.id).await.unwrap();
    f.sync().await;
    f
}

#[tokio::test]
async fn restore_brings_entries_back_on_both_sides() {
    let f = fixture_with_trash().await;
    let root = f.remote_root().await;
    let entries = trash(&f).list().unwrap();
    assert_eq!(entries.len(), 2);

    for (id, _) in entries {
        trash(&f).restore(id).await.unwrap();
    }
    assert_eq!(trash(&f).list().unwrap(), []);
    f.sync().await;

    assert_eq!(f.local.content("folder/local.txt").unwrap(), b"local");
    assert_eq!(f.local.content("remote.txt").unwrap(), b"remote");
    assert_eq!(f.remote_names(&root).await, ["folder", "remote.txt"]);
    f.assert_stable().await;
}

#[tokio::test]
async fn purge_waits_for_retention_and_supports_dry_run() {
    let f = fixture_with_trash().await;
    let retention = RetentionPeriod::days(30).unwrap();

    f.clock.advance_days(29);
    assert_eq!(trash(&f).purge_expired(retention, false).await.unwrap(), []);

    f.clock.advance_days(1);
    let planned = trash(&f).purge_expired(retention, true).await.unwrap();
    assert_eq!(planned.len(), 2);
    assert_eq!(
        trash(&f).list().unwrap().len(),
        2,
        "dry run deletes nothing"
    );

    let purged = trash(&f).purge_expired(retention, false).await.unwrap();
    assert_eq!(purged.len(), 2);
    assert_eq!(trash(&f).list().unwrap(), []);
    assert_eq!(f.local.trash_len(), 0);
}

#[tokio::test]
async fn restoring_unknown_entry_fails() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let unknown = protonsync_application::ports::TrashId(99);
    assert_eq!(
        trash(&f).restore(unknown).await,
        Err(UseCaseError::UnknownTrashEntry(unknown))
    );
}

/// Creates a "both modified" conflict on `report.txt`.
async fn fixture_with_conflict() -> Fixture {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("report.txt", b"base", common::T0);
    f.sync().await;
    f.local.write("report.txt", b"local", T1);
    let node = f.remote_child(&root, "report.txt").await.unwrap();
    f.remote
        .upload_revision(&node.id, &b"remote"[..], 6, T1)
        .await
        .unwrap();
    f.sync().await;
    f
}

fn copy_name() -> String {
    format!("report (Konflikt {}).txt", T1.unix_seconds())
}

#[tokio::test]
async fn keep_local_replaces_remote_version_everywhere() {
    let f = fixture_with_conflict().await;
    let root = f.remote_root().await;
    let (id, _) = conflicts(&f).list().unwrap()[0].clone();

    conflicts(&f).resolve(id, Resolution::KeepLocal).unwrap();
    f.sync().await;

    assert_eq!(f.local.content("report.txt").unwrap(), b"local");
    assert_eq!(f.remote_content(&root, "report.txt").await, b"local");
    assert_eq!(f.remote_names(&root).await, ["report.txt"]);
    assert_eq!(conflicts(&f).list().unwrap(), []);
    // The discarded remote version is in the trash.
    assert_eq!(f.local.trash_len(), 1);
    f.assert_stable().await;
}

#[tokio::test]
async fn keep_remote_discards_the_conflict_copy() {
    let f = fixture_with_conflict().await;
    let root = f.remote_root().await;
    let (id, _) = conflicts(&f).list().unwrap()[0].clone();

    conflicts(&f).resolve(id, Resolution::KeepRemote).unwrap();
    f.sync().await;

    assert_eq!(f.local.content(&copy_name()), None);
    assert_eq!(f.remote_names(&root).await, ["report.txt"]);
    assert_eq!(f.remote_content(&root, "report.txt").await, b"remote");
    f.assert_stable().await;
}

#[tokio::test]
async fn keep_both_only_closes_the_conflict() {
    let f = fixture_with_conflict().await;
    let root = f.remote_root().await;
    let (id, _) = conflicts(&f).list().unwrap()[0].clone();

    conflicts(&f).resolve(id, Resolution::KeepBoth).unwrap();
    f.sync().await;

    assert_eq!(
        f.remote_names(&root).await,
        [copy_name(), "report.txt".into()]
    );
    assert_eq!(f.state.conflicts(&f.pair.id).unwrap(), []);
}

#[tokio::test]
async fn ensure_folder_finds_or_creates_nested_folders() {
    use protonsync_application::use_cases::ensure_folder;
    use protonsync_domain::path::RelativePath;

    let remote = FakeRemote::new();
    let root = remote.root_folder().await.unwrap();
    let path = RelativePath::parse("Backups/Server").unwrap();

    let created = ensure_folder(&remote, &root, &path).await.unwrap();
    let found = ensure_folder(&remote, &root, &path).await.unwrap();
    assert_eq!(created.id, found.id);
    assert_eq!(created.name.as_str(), "Server");
    assert_eq!(
        ensure_folder(&remote, &root, &RelativePath::root())
            .await
            .unwrap()
            .id,
        root.id
    );
}

#[tokio::test]
async fn ensure_folder_refuses_files_in_the_way() {
    use protonsync_application::ports::RemoteError;
    use protonsync_application::use_cases::ensure_folder;
    use protonsync_domain::node::NodeName;
    use protonsync_domain::path::RelativePath;
    use protonsync_domain::time::Timestamp;

    let remote = FakeRemote::new();
    let root = remote.root_folder().await.unwrap();
    let name = NodeName::new("Backups").unwrap();
    remote
        .upload_file(
            &root.id,
            &name,
            &b"x"[..],
            1,
            Timestamp::from_unix_seconds(0),
        )
        .await
        .unwrap();
    let result = ensure_folder(&remote, &root, &RelativePath::parse("Backups/a").unwrap()).await;
    assert_eq!(result, Err(RemoteError::NotAFolder(root.id)));
}

#[tokio::test]
async fn find_folder_finds_but_never_creates() {
    use protonsync_application::use_cases::{ensure_folder, find_folder};
    use protonsync_domain::path::RelativePath;

    let remote = FakeRemote::new();
    let root = remote.root_folder().await.unwrap();
    let path = RelativePath::parse("Shared/Photos").unwrap();

    assert_eq!(find_folder(&remote, &root, &path).await, Ok(None));
    assert_eq!(remote.list_children(&root.id).await.unwrap(), Vec::new());

    let created = ensure_folder(&remote, &root, &path).await.unwrap();
    let found = find_folder(&remote, &root, &path).await.unwrap().unwrap();
    assert_eq!(found.id, created.id);
    assert_eq!(
        find_folder(&remote, &root, &RelativePath::root())
            .await
            .unwrap()
            .unwrap()
            .id,
        root.id
    );
}
