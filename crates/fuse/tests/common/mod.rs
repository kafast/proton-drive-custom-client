//! Gemeinsamer Aufbau: virtuelles Laufwerk über den Fakes.

#![allow(dead_code, clippy::unwrap_used)]

use std::sync::Arc;

use protonsync_application::drive::{DriveSettings, VirtualDrive};
use protonsync_application::ports::RemoteStorage;
use protonsync_application::use_cases::ensure_folder;
use protonsync_domain::node::{Node, NodeName};
use protonsync_domain::path::RelativePath;
use protonsync_domain::pin::PinState;
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::time::Timestamp;
use protonsync_fuse::{DriveOps, Owner};
use protonsync_testkit::{FakeRemote, FixedClock, InMemoryContentCache, InMemoryStateRepo};

pub const NOW: Timestamp = Timestamp::from_unix_seconds(1_760_000_000);
pub const OWNER: Owner = Owner {
    uid: 1000,
    gid: 1000,
};

pub type Drive = VirtualDrive<
    Arc<FakeRemote>,
    Arc<InMemoryContentCache>,
    Arc<InMemoryStateRepo>,
    Arc<InMemoryStateRepo>,
    Arc<FixedClock>,
>;

pub type Ops = DriveOps<
    Arc<FakeRemote>,
    Arc<InMemoryContentCache>,
    Arc<InMemoryStateRepo>,
    Arc<InMemoryStateRepo>,
    Arc<FixedClock>,
>;

pub struct Setup {
    pub remote: Arc<FakeRemote>,
    pub root: Node,
    pub drive: Arc<Drive>,
}

pub fn name(text: &str) -> NodeName {
    NodeName::new(text).unwrap()
}

impl Setup {
    pub async fn new() -> Self {
        let remote = Arc::new(FakeRemote::new());
        let start = remote.root_folder().await.unwrap();
        let root = ensure_folder(&*remote, &start, &RelativePath::parse("Laufwerk").unwrap())
            .await
            .unwrap();
        let state = Arc::new(InMemoryStateRepo::new());
        let drive = VirtualDrive::new(
            Arc::clone(&remote),
            Arc::new(InMemoryContentCache::new()),
            Arc::clone(&state),
            state,
            Arc::new(FixedClock::at(NOW)),
            DriveSettings {
                id: SyncPairId::new("mount:test"),
                default_pin: PinState::Unpinned,
            },
            &root,
        )
        .unwrap();
        Self {
            remote,
            root,
            drive: Arc::new(drive),
        }
    }

    pub fn ops(&self) -> Ops {
        DriveOps::new(Arc::clone(&self.drive), OWNER)
    }

    pub async fn remote_file(&self, parent: &Node, label: &str, content: &[u8]) -> Node {
        self.remote
            .upload_file(&parent.id, &name(label), content, content.len() as u64, NOW)
            .await
            .unwrap()
    }

    pub async fn remote_folder(&self, label: &str) -> Node {
        self.remote
            .create_folder(&self.root.id, &name(label))
            .await
            .unwrap()
    }

    pub async fn remote_content(&self, node: &Node) -> Vec<u8> {
        let mut out = Vec::new();
        self.remote.download_to(&node.id, &mut out).await.unwrap();
        out
    }

    pub async fn remote_names(&self, folder: &Node) -> Vec<String> {
        self.remote
            .list_children(&folder.id)
            .await
            .unwrap()
            .into_iter()
            .map(|node| node.name.to_string())
            .collect()
    }
}
