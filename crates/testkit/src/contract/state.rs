//! Contract-Tests für [`StateRepo`].
//!
//! ```ignore
//! protonsync_testkit::state_repo_contract_tests!(protonsync_testkit::InMemoryStateRepo::new);
//! ```

use protonsync_application::ports::{BaseEntry, EventCursor, StateRepo};
use protonsync_domain::conflict::{Conflict, ConflictKind};
use protonsync_domain::node::{Fingerprint, NodeId, RevisionId};
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::time::Timestamp;
use protonsync_domain::trash::{TrashEntry, TrashOrigin};

fn pair(id: &str) -> SyncPairId {
    SyncPairId::new(id)
}

fn path(value: &str) -> RelativePath {
    RelativePath::parse(value).unwrap()
}

fn file_entry(at: &str, revision: &str) -> BaseEntry {
    BaseEntry {
        path: path(at),
        remote_id: NodeId::new(format!("id-{at}")),
        remote_revision: Some(RevisionId::new(revision)),
        local: Some(Fingerprint {
            size: 3,
            modified: Timestamp::from_unix_seconds(7),
            content_hash: Some("abc".into()),
        }),
    }
}

fn folder_entry(at: &str) -> BaseEntry {
    BaseEntry {
        path: path(at),
        remote_id: NodeId::new(format!("id-{at}")),
        remote_revision: None,
        local: None,
    }
}

/// Basiseinträge werden je Pfad ersetzt und sortiert geliefert.
pub fn base_entries_roundtrip<S: StateRepo>(repo: &S) {
    let p = pair("p");
    repo.save_base(&p, &file_entry("b", "r1")).unwrap();
    repo.save_base(&p, &folder_entry("a")).unwrap();
    repo.save_base(&p, &file_entry("b", "r2")).unwrap();
    assert_eq!(
        repo.base_entries(&p).unwrap(),
        [folder_entry("a"), file_entry("b", "r2")]
    );
    assert_eq!(repo.base_entries(&pair("andere")).unwrap(), Vec::new());
}

/// Entfernen nimmt den ganzen Unterbaum mit, aber keine Namensvettern.
pub fn remove_base_tree_removes_descendants<S: StateRepo>(repo: &S) {
    let p = pair("p");
    for entry in [
        folder_entry("a"),
        file_entry("a/x", "r"),
        file_entry("ab", "r"),
    ] {
        repo.save_base(&p, &entry).unwrap();
    }
    repo.remove_base_tree(&p, &path("a")).unwrap();
    assert_eq!(repo.base_entries(&p).unwrap(), [file_entry("ab", "r")]);
}

/// Der Ereignis-Cursor wird je Sync-Paar gespeichert.
pub fn event_cursor_roundtrip<S: StateRepo>(repo: &S) {
    let p = pair("p");
    assert_eq!(repo.event_cursor(&p).unwrap(), None);
    repo.save_event_cursor(&p, &EventCursor::new("1")).unwrap();
    repo.save_event_cursor(&p, &EventCursor::new("2")).unwrap();
    assert_eq!(repo.event_cursor(&p).unwrap(), Some(EventCursor::new("2")));
    assert_eq!(repo.event_cursor(&pair("andere")).unwrap(), None);
}

/// Konflikte stehen in der Warteschlange, bis sie entfernt werden.
pub fn conflict_queue<S: StateRepo>(repo: &S) {
    let p = pair("p");
    let conflict = Conflict {
        relative_path: path("a.txt"),
        kind: ConflictKind::BothModified,
        conflict_copy: Some(path("a (Konflikt 1).txt")),
        detected_at: Timestamp::from_unix_seconds(1),
    };
    let first = repo.add_conflict(&p, &conflict).unwrap();
    let second = repo
        .add_conflict(
            &p,
            &Conflict {
                kind: ConflictKind::LocalDeletedRemoteModified,
                conflict_copy: None,
                ..conflict.clone()
            },
        )
        .unwrap();
    assert_ne!(first, second);
    let listed = repo.conflicts(&p).unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0], (first, conflict));
    repo.remove_conflict(first).unwrap();
    repo.remove_conflict(first).unwrap();
    assert_eq!(repo.conflicts(&p).unwrap().len(), 1);
    assert_eq!(repo.conflicts(&pair("andere")).unwrap(), Vec::new());
}

/// Papierkorbeinträge beider Herkunftsarten bleiben vollständig erhalten.
pub fn trash_entries_roundtrip<S: StateRepo>(repo: &S) {
    let p = pair("p");
    let local = TrashEntry {
        original_path: path("weg.txt"),
        origin: TrashOrigin::Local {
            stored_as: "k1".into(),
        },
        trashed_at: Timestamp::from_unix_seconds(5),
    };
    let remote = TrashEntry {
        original_path: path("ordner"),
        origin: TrashOrigin::Remote {
            node: NodeId::new("n9"),
        },
        trashed_at: Timestamp::from_unix_seconds(6),
    };
    let local_id = repo.add_trash(&p, &local).unwrap();
    let remote_id = repo.add_trash(&p, &remote).unwrap();
    assert_eq!(
        repo.trash_entries(&p).unwrap(),
        [(local_id, local), (remote_id, remote.clone())]
    );
    repo.remove_trash(local_id).unwrap();
    assert_eq!(repo.trash_entries(&p).unwrap(), [(remote_id, remote)]);
}

/// Erzeugt für jeden Contract-Test eine `#[test]`-Funktion.
#[macro_export]
macro_rules! state_repo_contract_tests {
    ($factory:expr) => {
        $crate::state_repo_contract_tests!(@tests $factory;
            base_entries_roundtrip,
            remove_base_tree_removes_descendants,
            event_cursor_roundtrip,
            conflict_queue,
            trash_entries_roundtrip,
        );
    };
    (@tests $factory:expr; $($test:ident),+ $(,)?) => {
        $(
            #[test]
            fn $test() {
                let repo = ($factory)();
                $crate::contract::state::$test(&repo);
            }
        )+
    };
}
