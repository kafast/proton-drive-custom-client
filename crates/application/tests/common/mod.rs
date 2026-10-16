//! Gemeinsame Testumgebung für die Szenariotests.

// Nicht jede Testdatei nutzt jede Hilfsfunktion.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use protonsync_application::ports::RemoteStorage;
use protonsync_application::sync::{RemoteCache, SyncContext, SyncOutcome, SyncReport};
use protonsync_domain::node::{Node, NodeName};
use protonsync_domain::sync_pair::{StorageMode, SyncDirection, SyncPair, SyncPairId};
use protonsync_domain::time::Timestamp;
use protonsync_testkit::{FakeLocal, FakeRemote, FixedClock, InMemoryStateRepo};

pub const T0: Timestamp = Timestamp::from_unix_seconds(1_760_000_000);
pub const T1: Timestamp = Timestamp::from_unix_seconds(1_760_000_100);

pub struct Fixture {
    pub remote: FakeRemote,
    pub local: FakeLocal,
    pub state: InMemoryStateRepo,
    pub clock: FixedClock,
    pub pair: SyncPair,
}

impl Fixture {
    pub async fn new(direction: SyncDirection) -> Self {
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

    pub fn context(&self) -> SyncContext<'_, FakeRemote, FakeLocal, InMemoryStateRepo, FixedClock> {
        SyncContext {
            remote: &self.remote,
            local: &self.local,
            state: &self.state,
            clock: &self.clock,
            pair: &self.pair,
        }
    }

    pub async fn sync(&self) -> SyncReport {
        match self.context().run_once(false).await.unwrap() {
            SyncOutcome::Applied(report) => report,
            SyncOutcome::Planned(_) => unreachable!("kein Probelauf"),
        }
    }

    /// Ein Lauf mit zwischengespeichertem entfernten Baum.
    pub async fn sync_cached(&self, cache: &mut RemoteCache) -> SyncReport {
        match self.context().run(cache, false).await.unwrap() {
            SyncOutcome::Applied(report) => report,
            SyncOutcome::Planned(_) => unreachable!("kein Probelauf"),
        }
    }

    /// Ein zweiter Lauf direkt danach darf nichts mehr zu tun haben.
    pub async fn assert_stable(&self) {
        match self.context().run_once(true).await.unwrap() {
            SyncOutcome::Planned(plan) => assert!(plan.is_empty(), "noch offen: {plan:?}"),
            SyncOutcome::Applied(_) => unreachable!("Probelauf"),
        }
    }

    pub async fn remote_root(&self) -> Node {
        self.remote.root_folder().await.unwrap()
    }

    /// Simuliert eine Änderung in Proton durch ein anderes Gerät.
    pub async fn remote_upload(&self, parent: &Node, name: &str, content: &[u8]) -> Node {
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

    pub async fn remote_child(&self, parent: &Node, name: &str) -> Option<Node> {
        self.remote
            .list_children(&parent.id)
            .await
            .unwrap()
            .into_iter()
            .find(|node| node.name.as_str() == name)
    }

    pub async fn remote_content(&self, parent: &Node, name: &str) -> Vec<u8> {
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

    pub async fn remote_names(&self, parent: &Node) -> Vec<String> {
        self.remote
            .list_children(&parent.id)
            .await
            .unwrap()
            .into_iter()
            .map(|node| node.name.to_string())
            .collect()
    }
}

/// Hülle um den Fake, deren Uploads scheitern, solange `fail_uploads` gesetzt ist,
/// bzw. eine falsche Größe melden, solange `corrupt_uploads` gesetzt ist.
pub struct FailingOnUpload<'a> {
    pub inner: &'a FakeRemote,
    pub fail_uploads: std::sync::atomic::AtomicBool,
    pub corrupt_uploads: std::sync::atomic::AtomicBool,
}

impl<'a> FailingOnUpload<'a> {
    pub fn new(inner: &'a FakeRemote) -> Self {
        Self {
            inner,
            fail_uploads: false.into(),
            corrupt_uploads: false.into(),
        }
    }
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

    /// Meldet eine um ein Byte zu große Datei, wie nach einem kaputten Upload.
    fn corrupt(mut node: Node) -> Node {
        if let protonsync_domain::node::NodeKind::File { fingerprint, .. } = &mut node.kind {
            fingerprint.size += 1;
        }
        node
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
            let node = self
                .inner
                .upload_file(parent, name, content, size, modified)
                .await?;
            Ok(if self.corrupt_uploads.load(Ordering::SeqCst) {
                corrupt(node)
            } else {
                node
            })
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
