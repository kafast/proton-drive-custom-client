//! Szenariotests der Sync-Engine gegen die Fakes (ohne Netz und Dateisystem).

// Hilfsfunktionen in Testdateien dürfen wie Tests abbrechen.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use protonsync_application::ports::{LocalStorage, RemoteError, RemoteStorage, StateRepo};
use protonsync_application::sync::{SyncContext, SyncError, SyncOutcome, SyncReport};
use protonsync_domain::conflict::{ConflictKind, SyncAction};
use protonsync_domain::node::{Node, NodeName};
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::{StorageMode, SyncDirection, SyncPair, SyncPairId};
use protonsync_domain::time::Timestamp;
use protonsync_domain::trash::TrashOrigin;
use protonsync_testkit::{FakeLocal, FakeRemote, FixedClock, InMemoryStateRepo};

const T0: Timestamp = Timestamp::from_unix_seconds(1_760_000_000);
const T1: Timestamp = Timestamp::from_unix_seconds(1_760_000_100);

struct Fixture {
    remote: FakeRemote,
    local: FakeLocal,
    state: InMemoryStateRepo,
    clock: FixedClock,
    pair: SyncPair,
}

impl Fixture {
    async fn new(direction: SyncDirection) -> Self {
        let remote = FakeRemote::new();
        let root = remote.root_folder().await.unwrap();
        Self {
            remote,
            local: FakeLocal::new(),
            state: InMemoryStateRepo::new(),
            clock: FixedClock::at(T1),
            pair: SyncPair {
                id: SyncPairId::new("test"),
                local_root: "/nicht/benutzt".into(),
                remote_root: root.id,
                direction,
                storage_mode: StorageMode::FullLocal,
            },
        }
    }

    fn context(&self) -> SyncContext<'_, FakeRemote, FakeLocal, InMemoryStateRepo, FixedClock> {
        SyncContext {
            remote: &self.remote,
            local: &self.local,
            state: &self.state,
            clock: &self.clock,
            pair: &self.pair,
        }
    }

    async fn sync(&self) -> SyncReport {
        match self.context().run_once(false).await.unwrap() {
            SyncOutcome::Applied(report) => report,
            SyncOutcome::Planned(_) => unreachable!("kein Probelauf"),
        }
    }

    /// Ein zweiter Lauf direkt danach darf nichts mehr zu tun haben.
    async fn assert_stable(&self) {
        match self.context().run_once(true).await.unwrap() {
            SyncOutcome::Planned(plan) => assert!(plan.is_empty(), "noch offen: {plan:?}"),
            SyncOutcome::Applied(_) => unreachable!("Probelauf"),
        }
    }

    async fn remote_root(&self) -> Node {
        self.remote.root_folder().await.unwrap()
    }

    /// Simuliert eine Änderung in Proton durch ein anderes Gerät.
    async fn remote_upload(&self, parent: &Node, name: &str, content: &[u8]) -> Node {
        self.remote
            .upload_file(
                &parent.id,
                &NodeName::new(name).unwrap(),
                content,
                content.len() as u64,
                T0,
            )
            .await
            .unwrap()
    }

    async fn remote_child(&self, parent: &Node, name: &str) -> Option<Node> {
        self.remote
            .list_children(&parent.id)
            .await
            .unwrap()
            .into_iter()
            .find(|node| node.name.as_str() == name)
    }

    async fn remote_content(&self, parent: &Node, name: &str) -> Vec<u8> {
        let node = self
            .remote_child(parent, name)
            .await
            .expect("entfernte Datei fehlt");
        let mut content = Vec::new();
        self.remote
            .download_to(&node.id, &mut content)
            .await
            .unwrap();
        content
    }

    async fn remote_names(&self, parent: &Node) -> Vec<String> {
        self.remote
            .list_children(&parent.id)
            .await
            .unwrap()
            .into_iter()
            .map(|node| node.name.to_string())
            .collect()
    }
}

#[tokio::test]
async fn first_run_merges_both_sides() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("lokal.txt", b"von hier", T0);
    f.local.write("ordner/tief.txt", b"tief", T0);
    f.remote_upload(&root, "entfernt.txt", b"von dort").await;

    let report = f.sync().await;

    assert_eq!(report.uploaded, 3, "{report:?}");
    assert_eq!(report.downloaded, 1);
    assert_eq!(f.local.content("entfernt.txt").unwrap(), b"von dort");
    assert_eq!(f.remote_content(&root, "lokal.txt").await, b"von hier");
    let folder = f.remote_child(&root, "ordner").await.unwrap();
    assert_eq!(f.remote_content(&folder, "tief.txt").await, b"tief");
    f.assert_stable().await;
}

#[tokio::test]
async fn edits_flow_in_both_directions() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("a.txt", b"1", T0);
    let remote_b = f.remote_upload(&root, "b.txt", b"1").await;
    f.sync().await;

    f.local.write("a.txt", b"lokal neu", T1);
    f.remote
        .upload_revision(&remote_b.id, &b"entfernt neu"[..], 12, T1)
        .await
        .unwrap();
    let report = f.sync().await;

    assert_eq!((report.uploaded, report.downloaded), (1, 1));
    assert_eq!(f.remote_content(&root, "a.txt").await, b"lokal neu");
    assert_eq!(f.local.content("b.txt").unwrap(), b"entfernt neu");
    f.assert_stable().await;
}

#[tokio::test]
async fn deletions_go_to_the_trash_on_the_other_side() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("lokal-weg.txt", b"x", T0);
    f.local.write("entfernt-weg.txt", b"y", T0);
    f.sync().await;

    f.local.remove("lokal-weg.txt");
    let gone = f.remote_child(&root, "entfernt-weg.txt").await.unwrap();
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
    f.local.write("ordner/a", b"a", T0);
    f.local.write("ordner/b", b"b", T0);
    f.sync().await;

    f.local.remove("ordner");
    let report = f.sync().await;

    assert_eq!(report.trashed_remote, 1, "{report:?}");
    assert_eq!(f.remote_names(&root).await, Vec::<String>::new());
    f.assert_stable().await;
}

#[tokio::test]
async fn both_sides_edited_keeps_both_versions_and_queues_a_conflict() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("bericht.txt", b"basis", T0);
    f.sync().await;

    f.local.write("bericht.txt", b"lokale Fassung", T1);
    let node = f.remote_child(&root, "bericht.txt").await.unwrap();
    f.remote
        .upload_revision(&node.id, &b"entfernte Fassung"[..], 17, T1)
        .await
        .unwrap();
    let report = f.sync().await;

    assert_eq!(report.conflicts, 1, "{report:?}");
    let copy = format!("bericht (Konflikt {}).txt", T1.unix_seconds());
    assert_eq!(
        f.local.content("bericht.txt").unwrap(),
        b"entfernte Fassung"
    );
    assert_eq!(f.local.content(&copy).unwrap(), b"lokale Fassung");
    let conflicts = f.state.conflicts(&f.pair.id).unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].1.kind, ConflictKind::BothModified);
    assert_eq!(
        conflicts[0].1.conflict_copy.as_ref().unwrap().as_str(),
        copy
    );

    // Der nächste Lauf lädt die Konfliktkopie hoch; danach ist alles ruhig.
    let report = f.sync().await;
    assert_eq!(report.uploaded, 1);
    assert_eq!(f.remote_content(&root, &copy).await, b"lokale Fassung");
    f.assert_stable().await;
}

#[tokio::test]
async fn edit_beats_delete_and_is_reported() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("a.txt", b"basis", T0);
    f.local.write("b.txt", b"basis", T0);
    f.sync().await;

    // a: lokal geändert, entfernt gelöscht. b: lokal gelöscht, entfernt geändert.
    f.local.write("a.txt", b"lokal neu", T1);
    let a = f.remote_child(&root, "a.txt").await.unwrap();
    f.remote.trash(&a.id).await.unwrap();
    f.local.remove("b.txt");
    let b = f.remote_child(&root, "b.txt").await.unwrap();
    f.remote
        .upload_revision(&b.id, &b"entfernt neu"[..], 12, T1)
        .await
        .unwrap();
    let report = f.sync().await;

    assert_eq!(report.conflicts, 2, "{report:?}");
    assert_eq!(f.remote_content(&root, "a.txt").await, b"lokal neu");
    assert_eq!(f.local.content("b.txt").unwrap(), b"entfernt neu");
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
        unreachable!("Probelauf")
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
    f.local.write("ordner/alt.txt", b"alt", T0);
    f.sync().await;

    f.local.remove("ordner");
    let folder = f.remote_child(&root, "ordner").await.unwrap();
    f.remote_upload(&folder, "neu.txt", b"neu").await;
    let report = f.sync().await;

    // Nichts Neues geht verloren: Der Ordner kommt mit der neuen Datei zurück,
    // nur die lokal gelöschte alte Datei wandert in den Papierkorb.
    assert_eq!(f.local.content("ordner/neu.txt").unwrap(), b"neu");
    assert_eq!(f.remote_names(&folder).await, ["neu.txt"]);
    assert_eq!(report.trashed_remote, 1, "{report:?}");
    f.assert_stable().await;
}

#[tokio::test]
async fn failed_step_is_reported_and_retried() {
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("a.txt", b"x", T0);

    // Die ersten Aufrufe (Cursor, Auflisten) gelingen; der Upload scheitert.
    let failing = FailingOnUpload {
        inner: &f.remote,
        fail_uploads: std::sync::atomic::AtomicBool::new(true),
    };
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
    f.remote_upload(&root, "spiegel.txt", b"remote").await;
    f.sync().await;

    f.local.write("spiegel.txt", b"lokal verbastelt", T1);
    f.local.write("nur-lokal.txt", b"x", T1);
    let report = f.sync().await;

    assert_eq!(report.ignored, 2, "{report:?}");
    assert_eq!(f.remote_names(&root).await, ["spiegel.txt"]);
    assert_eq!(f.remote_content(&root, "spiegel.txt").await, b"remote");
}

#[tokio::test]
async fn interrupted_upload_is_recognised_as_identical() {
    // Simuliert einen Absturz nach dem Upload, aber vor dem Speichern der Basis:
    // Beide Seiten haben dieselbe Datei, die Basis kennt sie nicht.
    let f = Fixture::new(SyncDirection::TwoWay).await;
    let root = f.remote_root().await;
    f.local.write("a.txt", b"gleich", T0);
    f.remote_upload(&root, "a.txt", b"gleich").await;

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
    f.local.mkdir("doppelt");
    f.remote_upload(&root, "doppelt", b"datei").await;

    let report = f.sync().await;

    assert_eq!(report.skipped.len(), 1);
    assert_eq!(
        report.skipped[0].path,
        RelativePath::parse("doppelt").unwrap()
    );
    assert!(
        f.local
            .entry(&RelativePath::parse("doppelt").unwrap())
            .unwrap()
            .is_some()
    );
}

/// Hülle um den Fake, deren Uploads scheitern, solange `fail_uploads` gesetzt ist.
struct FailingOnUpload<'a> {
    inner: &'a FakeRemote,
    fail_uploads: std::sync::atomic::AtomicBool,
}

mod failing_impl {
    use std::io::{Read, Write};
    use std::sync::atomic::Ordering;

    use protonsync_application::ports::{EventCursor, RemoteError, RemoteEvent, RemoteStorage};
    use protonsync_domain::node::{Node, NodeId, NodeName};
    use protonsync_domain::time::Timestamp;

    use super::FailingOnUpload;

    fn network_down() -> RemoteError {
        RemoteError::Unavailable("Netz weg".into())
    }

    impl RemoteStorage for FailingOnUpload<'_> {
        async fn root_folder(&self) -> Result<Node, RemoteError> {
            self.inner.root_folder().await
        }
        async fn node(&self, id: &NodeId) -> Result<Node, RemoteError> {
            self.inner.node(id).await
        }
        async fn list_children(&self, folder: &NodeId) -> Result<Vec<Node>, RemoteError> {
            self.inner.list_children(folder).await
        }
        async fn create_folder(
            &self,
            parent: &NodeId,
            name: &NodeName,
        ) -> Result<Node, RemoteError> {
            self.inner.create_folder(parent, name).await
        }
        async fn upload_file<R: Read + Send>(
            &self,
            parent: &NodeId,
            name: &NodeName,
            content: R,
            size: u64,
            modified: Timestamp,
        ) -> Result<Node, RemoteError> {
            if self.fail_uploads.load(Ordering::SeqCst) {
                return Err(network_down());
            }
            self.inner
                .upload_file(parent, name, content, size, modified)
                .await
        }
        async fn upload_revision<R: Read + Send>(
            &self,
            file: &NodeId,
            content: R,
            size: u64,
            modified: Timestamp,
        ) -> Result<Node, RemoteError> {
            if self.fail_uploads.load(Ordering::SeqCst) {
                return Err(network_down());
            }
            self.inner
                .upload_revision(file, content, size, modified)
                .await
        }
        async fn download_to<W: Write + Send>(
            &self,
            file: &NodeId,
            target: &mut W,
        ) -> Result<u64, RemoteError> {
            self.inner.download_to(file, target).await
        }
        async fn download_range(
            &self,
            file: &NodeId,
            offset: u64,
            length: u64,
        ) -> Result<Vec<u8>, RemoteError> {
            self.inner.download_range(file, offset, length).await
        }
        async fn rename(&self, id: &NodeId, new_name: &NodeName) -> Result<Node, RemoteError> {
            self.inner.rename(id, new_name).await
        }
        async fn move_node(&self, id: &NodeId, new_parent: &NodeId) -> Result<Node, RemoteError> {
            self.inner.move_node(id, new_parent).await
        }
        async fn trash(&self, id: &NodeId) -> Result<(), RemoteError> {
            self.inner.trash(id).await
        }
        async fn restore(&self, id: &NodeId) -> Result<Node, RemoteError> {
            self.inner.restore(id).await
        }
        async fn delete_permanently(&self, id: &NodeId) -> Result<(), RemoteError> {
            self.inner.delete_permanently(id).await
        }
        async fn events_since(
            &self,
            cursor: Option<&EventCursor>,
        ) -> Result<(Vec<RemoteEvent>, EventCursor), RemoteError> {
            self.inner.events_since(cursor).await
        }
    }
}
