//! Szenariotests für Papierkorb und Konfliktauflösung.

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

/// Legt eine synchronisierte Datei an und löscht sie lokal und eine entfernt.
async fn fixture_with_trash() -> Fixture {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("ordner/lokal.txt", b"lokal", common::T0);
    f.local.write("entfernt.txt", b"entfernt", common::T0);
    f.sync().await;
    // Lokal gelöscht → Proton-Papierkorb; entfernt gelöscht → lokaler Papierkorb.
    f.local.remove("ordner");
    let remote_file = f.remote_child(&root, "entfernt.txt").await.unwrap();
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

    assert_eq!(f.local.content("ordner/lokal.txt").unwrap(), b"lokal");
    assert_eq!(f.local.content("entfernt.txt").unwrap(), b"entfernt");
    assert_eq!(f.remote_names(&root).await, ["entfernt.txt", "ordner"]);
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
        "Probelauf löscht nichts"
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

/// Erzeugt einen Konflikt „beide geändert“ an `bericht.txt`.
async fn fixture_with_conflict() -> Fixture {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("bericht.txt", b"basis", common::T0);
    f.sync().await;
    f.local.write("bericht.txt", b"lokal", T1);
    let node = f.remote_child(&root, "bericht.txt").await.unwrap();
    f.remote
        .upload_revision(&node.id, &b"entfernt"[..], 8, T1)
        .await
        .unwrap();
    f.sync().await;
    f
}

fn copy_name() -> String {
    format!("bericht (Konflikt {}).txt", T1.unix_seconds())
}

#[tokio::test]
async fn keep_local_replaces_remote_version_everywhere() {
    let f = fixture_with_conflict().await;
    let root = f.remote_root().await;
    let (id, _) = conflicts(&f).list().unwrap()[0].clone();

    conflicts(&f).resolve(id, Resolution::KeepLocal).unwrap();
    f.sync().await;

    assert_eq!(f.local.content("bericht.txt").unwrap(), b"lokal");
    assert_eq!(f.remote_content(&root, "bericht.txt").await, b"lokal");
    assert_eq!(f.remote_names(&root).await, ["bericht.txt"]);
    assert_eq!(conflicts(&f).list().unwrap(), []);
    // Die verworfene entfernte Fassung liegt im Papierkorb.
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
    assert_eq!(f.remote_names(&root).await, ["bericht.txt"]);
    assert_eq!(f.remote_content(&root, "bericht.txt").await, b"entfernt");
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
        [copy_name(), "bericht.txt".into()]
    );
    assert_eq!(f.state.conflicts(&f.pair.id).unwrap(), []);
}
