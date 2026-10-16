//! Virtual drive against the fakes: reading on demand, writing and
//! uploading, conflicts, trash, renaming, pinning, freeing, restart.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use protonsync_application::drive::{
    DriveContent, DriveError, DriveSettings, Ino, ROOT_INO, VirtualDrive,
};
use protonsync_application::ports::{RemoteStorage, StateRepo};
use protonsync_application::use_cases::ensure_folder;
use protonsync_domain::cache::BLOCK_BYTES;
use protonsync_domain::node::{Node, NodeName};
use protonsync_domain::path::RelativePath;
use protonsync_domain::pin::{EvictionPolicy, PinState};
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::time::Timestamp;
use protonsync_testkit::{FakeRemote, FixedClock, InMemoryContentCache, InMemoryStateRepo};

const NOW: Timestamp = Timestamp::from_unix_seconds(1_760_000_000);
const DRIVE: &str = "mount:home";

type Drive = VirtualDrive<
    Arc<FakeRemote>,
    Arc<InMemoryContentCache>,
    Arc<InMemoryStateRepo>,
    Arc<InMemoryStateRepo>,
    Arc<FixedClock>,
>;

struct Setup {
    remote: Arc<FakeRemote>,
    cache: Arc<InMemoryContentCache>,
    state: Arc<InMemoryStateRepo>,
    clock: Arc<FixedClock>,
    root: Node,
    drive: Drive,
}

fn name(text: &str) -> NodeName {
    NodeName::new(text).unwrap()
}

fn path(text: &str) -> RelativePath {
    RelativePath::parse(text).unwrap()
}

fn bytes(size: u64, seed: u8) -> Vec<u8> {
    (0..size)
        .map(|i| seed.wrapping_add((i % 251) as u8))
        .collect()
}

impl Setup {
    async fn new(default_pin: PinState) -> Self {
        let remote = Arc::new(FakeRemote::new());
        let start = remote.root_folder().await.unwrap();
        let root = ensure_folder(&*remote, &start, &path("Drive"))
            .await
            .unwrap();
        let cache = Arc::new(InMemoryContentCache::new());
        let state = Arc::new(InMemoryStateRepo::new());
        let clock = Arc::new(FixedClock::at(NOW));
        let drive = Self::open_drive(&remote, &cache, &state, &clock, &root, default_pin);
        Self {
            remote,
            cache,
            state,
            clock,
            root,
            drive,
        }
    }

    fn open_drive(
        remote: &Arc<FakeRemote>,
        cache: &Arc<InMemoryContentCache>,
        state: &Arc<InMemoryStateRepo>,
        clock: &Arc<FixedClock>,
        root: &Node,
        default_pin: PinState,
    ) -> Drive {
        VirtualDrive::new(
            Arc::clone(remote),
            Arc::clone(cache),
            Arc::clone(state),
            Arc::clone(state),
            Arc::clone(clock),
            DriveSettings {
                id: SyncPairId::new(DRIVE),
                default_pin,
            },
            root,
        )
        .unwrap()
    }

    /// Simulates a restart of the daemon: new drive, same state.
    fn restart(&mut self) {
        self.drive = Self::open_drive(
            &self.remote,
            &self.cache,
            &self.state,
            &self.clock,
            &self.root,
            PinState::Unpinned,
        );
    }

    async fn remote_folder(&self, label: &str) -> Node {
        self.remote
            .create_folder(&self.root.id, &name(label))
            .await
            .unwrap()
    }

    async fn remote_file(&self, parent: &Node, label: &str, content: &[u8]) -> Node {
        self.remote
            .upload_file(&parent.id, &name(label), content, content.len() as u64, NOW)
            .await
            .unwrap()
    }

    async fn remote_content(&self, node: &Node) -> Vec<u8> {
        let mut out = Vec::new();
        self.remote.download_to(&node.id, &mut out).await.unwrap();
        out
    }

    async fn remote_names(&self, folder: &Node) -> Vec<String> {
        self.remote
            .list_children(&folder.id)
            .await
            .unwrap()
            .into_iter()
            .map(|node| node.name.to_string())
            .collect()
    }

    async fn ino(&self, text: &str) -> Ino {
        self.drive.resolve(&path(text)).await.unwrap()
    }

    async fn read_all(&self, ino: Ino) -> Vec<u8> {
        let size = self.drive.attributes(ino).unwrap().size;
        self.drive
            .read(ino, 0, u32::try_from(size).unwrap())
            .await
            .unwrap()
    }

    async fn write_file(&self, ino: Ino, content: &[u8]) {
        self.drive.open(ino, true).await.unwrap();
        self.drive.set_size(ino, 0).await.unwrap();
        self.drive.write(ino, 0, content).unwrap();
        self.drive.release(ino).await.unwrap();
    }
}

#[tokio::test]
async fn folders_are_read_once_and_only_on_demand() {
    let s = Setup::new(PinState::Unpinned).await;
    let docs = s.remote_folder("Docs").await;
    s.remote_file(&docs, "a.txt", b"hello").await;
    let before = s.remote.list_calls();
    let entries = s.drive.list(ROOT_INO).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].is_folder);
    let docs_ino = entries[0].ino;
    let file = s.drive.lookup(docs_ino, &name("a.txt")).await.unwrap();
    assert_eq!(file.size, 5);
    assert!(!file.is_folder);
    s.drive.list(docs_ino).await.unwrap();
    s.drive.lookup(docs_ino, &name("a.txt")).await.unwrap();
    assert_eq!(s.remote.list_calls() - before, 2, "one call per folder");
    assert_eq!(
        s.drive.lookup(docs_ino, &name("missing")).await,
        Err(DriveError::NotFound)
    );
    assert_eq!(s.drive.list(file.ino).await, Err(DriveError::NotAFolder));
}

#[tokio::test]
async fn reading_loads_only_the_needed_blocks() {
    let s = Setup::new(PinState::Unpinned).await;
    let content = bytes(3 * BLOCK_BYTES + 100, 7);
    s.remote_file(&s.root, "video.mkv", &content).await;
    let ino = s.ino("video.mkv").await;

    let start = s.drive.read(ino, 10, 20).await.unwrap();
    assert_eq!(start, content[10..30]);
    assert_eq!(
        s.remote.range_bytes(),
        2 * BLOCK_BYTES,
        "block plus read-ahead"
    );
    s.drive.read(ino, 100, 4096).await.unwrap();
    assert_eq!(s.remote.range_bytes(), 2 * BLOCK_BYTES, "already present");

    let tail_offset = 3 * BLOCK_BYTES + 90;
    let tail = s.drive.read(ino, tail_offset, 4096).await.unwrap();
    assert_eq!(tail, content[usize::try_from(tail_offset).unwrap()..]);
    assert_eq!(s.remote.range_bytes(), 2 * BLOCK_BYTES + 100);
    assert_eq!(s.read_all(ino).await, content);
    assert_eq!(
        s.remote.range_bytes(),
        content.len() as u64,
        "nothing twice"
    );
    assert_eq!(s.drive.usage().unwrap().bytes, content.len() as u64);
}

#[tokio::test]
async fn a_new_revision_in_proton_replaces_the_cached_copy() {
    let s = Setup::new(PinState::Unpinned).await;
    let file = s.remote_file(&s.root, "a.txt", b"old").await;
    s.drive.refresh().await.unwrap();
    let ino = s.ino("a.txt").await;
    assert_eq!(s.read_all(ino).await, b"old");

    s.remote
        .upload_revision(&file.id, &b"new contents"[..], 12, NOW)
        .await
        .unwrap();
    let report = s.drive.refresh().await.unwrap();
    assert!(report.events > 0);
    assert_eq!(s.drive.attributes(ino).unwrap().size, 12);
    assert_eq!(s.read_all(ino).await, b"new contents");
}

#[tokio::test]
async fn changed_files_are_uploaded_as_a_new_revision_after_closing() {
    let s = Setup::new(PinState::Unpinned).await;
    let file = s.remote_file(&s.root, "note.txt", b"one").await;
    let ino = s.ino("note.txt").await;
    s.drive.open(ino, true).await.unwrap();
    s.drive.write(ino, 3, b" two").unwrap();
    assert_eq!(s.drive.attributes(ino).unwrap().size, 7);
    assert_eq!(s.remote_content(&file).await, b"one", "still open");
    assert_eq!(s.drive.usage().unwrap().pending_uploads, 1);
    s.drive.release(ino).await.unwrap();
    assert_eq!(s.remote_content(&file).await, b"one two");
    assert_eq!(s.drive.usage().unwrap().pending_uploads, 0);
    assert_eq!(s.remote_names(&s.root).await, ["note.txt"]);
}

#[tokio::test]
async fn new_files_appear_in_proton_after_closing() {
    let s = Setup::new(PinState::Unpinned).await;
    let docs = s.remote_folder("Docs").await;
    let docs_ino = s.ino("Docs").await;
    let created = s.drive.create(docs_ino, &name("new.txt")).await.unwrap();
    assert_eq!(
        s.drive.create(docs_ino, &name("new.txt")).await,
        Err(DriveError::AlreadyExists)
    );
    s.drive.write(created.ino, 0, b"fresh").unwrap();
    assert_eq!(
        s.read_all(created.ino).await,
        b"fresh",
        "readable before the upload"
    );
    s.drive.release(created.ino).await.unwrap();
    let uploaded = s.remote.list_children(&docs.id).await.unwrap();
    assert_eq!(uploaded.len(), 1);
    assert_eq!(s.remote_content(&uploaded[0]).await, b"fresh");
    // Afterwards it is a completely normal file with a node.
    s.write_file(created.ino, b"second version").await;
    assert_eq!(s.remote_content(&uploaded[0]).await, b"second version");
}

#[tokio::test]
async fn a_conflicting_change_in_proton_keeps_both_versions() {
    let s = Setup::new(PinState::Unpinned).await;
    let file = s.remote_file(&s.root, "plan.txt", b"base").await;
    let ino = s.ino("plan.txt").await;
    s.drive.open(ino, true).await.unwrap();
    s.drive.write(ino, 0, b"LOCAL").unwrap();
    s.remote
        .upload_revision(&file.id, &b"remote"[..], 6, NOW)
        .await
        .unwrap();
    s.drive.release(ino).await.unwrap();

    assert_eq!(
        s.remote_content(&file).await,
        b"remote",
        "nothing overwritten"
    );
    let names = s.remote_names(&s.root).await;
    assert_eq!(names.len(), 2, "{names:?}");
    let copy = s
        .remote
        .list_children(&s.root.id)
        .await
        .unwrap()
        .into_iter()
        .find(|node| node.name.as_str().contains("Konflikt"))
        .unwrap();
    assert_eq!(s.remote_content(&copy).await, b"LOCAL");
    // In the drive: the original shows Proton, the copy the local content.
    assert_eq!(s.read_all(ino).await, b"remote");
    let copy_ino = s.drive.lookup(ROOT_INO, &copy.name).await.unwrap().ino;
    assert_eq!(s.read_all(copy_ino).await, b"LOCAL");
}

#[tokio::test]
async fn failed_uploads_stay_pending_and_survive_a_restart() {
    let mut s = Setup::new(PinState::Unpinned).await;
    s.remote_folder("Docs").await;
    let docs_ino = s.ino("Docs").await;
    let created = s
        .drive
        .create(docs_ino, &name("important.txt"))
        .await
        .unwrap();
    s.drive.write(created.ino, 0, b"do not lose").unwrap();
    s.remote
        .fail_next_call(protonsync_application::ports::RemoteError::Unavailable(
            "network".into(),
        ));
    assert!(s.drive.release(created.ino).await.is_err());
    assert_eq!(s.drive.usage().unwrap().pending_uploads, 1);

    s.restart();
    assert_eq!(
        s.drive.usage().unwrap().pending_uploads,
        1,
        "from the state"
    );
    assert_eq!(s.drive.upload_pending().await, vec![]);
    assert_eq!(s.drive.usage().unwrap().pending_uploads, 0);
    let file = s.ino("Docs/important.txt").await;
    assert_eq!(s.read_all(file).await, b"do not lose");
}

#[tokio::test]
async fn deleting_moves_to_both_trashes() {
    let s = Setup::new(PinState::Unpinned).await;
    let docs = s.remote_folder("Docs").await;
    s.remote_file(&docs, "a.txt", b"x").await;
    let docs_ino = s.ino("Docs").await;
    assert_eq!(
        s.drive.remove_folder(ROOT_INO, &name("Docs")).await,
        Err(DriveError::NotEmpty)
    );
    assert_eq!(
        s.drive.remove_file(ROOT_INO, &name("Docs")).await,
        Err(DriveError::IsAFolder)
    );
    s.drive.remove_file(docs_ino, &name("a.txt")).await.unwrap();
    assert_eq!(s.remote_names(&docs).await, Vec::<String>::new());
    s.drive
        .remove_folder(ROOT_INO, &name("Docs"))
        .await
        .unwrap();
    assert_eq!(s.remote_names(&s.root).await, Vec::<String>::new());
    let trash = s.state.trash_entries(&SyncPairId::new(DRIVE)).unwrap();
    let paths: Vec<&str> = trash
        .iter()
        .map(|(_, e)| e.original_path.as_str())
        .collect();
    assert_eq!(paths, ["Docs/a.txt", "Docs"]);
}

#[tokio::test]
async fn renaming_and_moving_go_to_proton() {
    let s = Setup::new(PinState::Unpinned).await;
    let docs = s.remote_folder("Docs").await;
    let archive = s.remote_folder("Archive").await;
    s.remote_file(&docs, "a.txt", b"x").await;
    let docs_ino = s.ino("Docs").await;
    let archive_ino = s.ino("Archive").await;
    s.drive
        .rename(docs_ino, &name("a.txt"), archive_ino, &name("b.txt"))
        .await
        .unwrap();
    assert_eq!(s.remote_names(&docs).await, Vec::<String>::new());
    assert_eq!(s.remote_names(&archive).await, ["b.txt"]);
    assert!(s.drive.lookup(archive_ino, &name("b.txt")).await.is_ok());
    assert_eq!(
        s.drive.lookup(docs_ino, &name("a.txt")).await,
        Err(DriveError::NotFound)
    );
}

#[tokio::test]
async fn saving_through_a_temporary_file_creates_a_new_revision() {
    let s = Setup::new(PinState::Unpinned).await;
    let file = s.remote_file(&s.root, "letter.odt", b"version 1").await;
    let original = s.ino("letter.odt").await;
    // This is how many programs save: write a helper file, then rename it over the original.
    let temp = s
        .drive
        .create(ROOT_INO, &name(".~letter.tmp"))
        .await
        .unwrap();
    s.drive.write(temp.ino, 0, b"version 2").unwrap();
    s.drive.release(temp.ino).await.unwrap();
    s.drive
        .rename(
            ROOT_INO,
            &name(".~letter.tmp"),
            ROOT_INO,
            &name("letter.odt"),
        )
        .await
        .unwrap();
    assert_eq!(s.remote_content(&file).await, b"version 2");
    assert_eq!(
        s.remote_names(&s.root).await,
        ["letter.odt"],
        "helper file in the trash"
    );
    assert_eq!(s.read_all(original).await, b"version 2");
    assert_eq!(
        s.drive.lookup(ROOT_INO, &name(".~letter.tmp")).await,
        Err(DriveError::NotFound)
    );
}

#[tokio::test]
async fn pinned_folders_are_loaded_and_never_evicted() {
    let s = Setup::new(PinState::Unpinned).await;
    let keep = s.remote_folder("Keep").await;
    let sub = s
        .remote
        .create_folder(&keep.id, &name("Sub"))
        .await
        .unwrap();
    s.remote_file(&keep, "a.bin", &bytes(1000, 1)).await;
    s.remote_file(&sub, "b.bin", &bytes(2000, 2)).await;
    s.remote_file(&s.root, "loose.bin", &bytes(500, 3)).await;
    s.drive
        .set_pin(&path("Keep"), Some(PinState::Pinned))
        .unwrap();
    assert_eq!(
        s.drive.pin_state(&path("Keep/Sub/b.bin")).unwrap(),
        PinState::Pinned
    );
    assert_eq!(
        s.drive.pin_state(&path("loose.bin")).unwrap(),
        PinState::Unpinned
    );

    let report = s.drive.prefetch_pinned().await.unwrap();
    assert_eq!((report.files, report.bytes), (2, 3000));
    assert_eq!(
        s.drive.prefetch_pinned().await.unwrap().files,
        0,
        "already there"
    );

    let loose = s.ino("loose.bin").await;
    s.read_all(loose).await;
    s.clock.advance_days(40);
    let policy = EvictionPolicy::new(30, u64::MAX).unwrap();
    let freed = s.drive.evict(policy, false).await.unwrap();
    assert_eq!(freed.files, [path("loose.bin")]);
    assert_eq!(freed.bytes, 500);
    assert_eq!(s.drive.usage().unwrap().bytes, 3000);
    // Freed means: the placeholder stays, the content comes back when read.
    assert_eq!(s.read_all(loose).await, bytes(500, 3));
}

#[tokio::test]
async fn eviction_plans_first_and_keeps_changed_and_open_files() {
    let s = Setup::new(PinState::Unpinned).await;
    s.remote_file(&s.root, "old.bin", &bytes(100, 1)).await;
    s.remote_file(&s.root, "open.bin", &bytes(100, 2)).await;
    let old = s.ino("old.bin").await;
    let open = s.ino("open.bin").await;
    s.read_all(old).await;
    s.drive.open(open, false).await.unwrap();
    s.read_all(open).await;
    let created = s.drive.create(ROOT_INO, &name("new.txt")).await.unwrap();
    s.drive.write(created.ino, 0, b"unsaved").unwrap();
    s.clock.advance_days(40);
    let policy = EvictionPolicy::new(30, 0).unwrap();

    let planned = s.drive.evict(policy, true).await.unwrap();
    assert_eq!(planned.files, [path("old.bin")]);
    assert_eq!(s.drive.usage().unwrap().files, 3, "dry run changes nothing");
    let done = s.drive.evict(policy, false).await.unwrap();
    assert_eq!(done.files, [path("old.bin")]);
    assert_eq!(s.drive.usage().unwrap().files, 2);
}

#[tokio::test]
async fn pins_follow_renamed_folders_and_can_be_cleared() {
    let s = Setup::new(PinState::Pinned).await;
    s.remote_folder("A").await;
    assert_eq!(
        s.drive.pin_state(&path("A/x")).unwrap(),
        PinState::Pinned,
        "default"
    );
    s.drive
        .set_pin(&path("A"), Some(PinState::Unpinned))
        .unwrap();
    s.drive
        .set_pin(&path("A/important"), Some(PinState::Pinned))
        .unwrap();
    s.drive
        .rename(ROOT_INO, &name("A"), ROOT_INO, &name("B"))
        .await
        .unwrap();
    let rules = s.drive.pin_rules().unwrap();
    assert_eq!(
        rules,
        vec![
            (path("B"), PinState::Unpinned),
            (path("B/important"), PinState::Pinned)
        ]
    );
    s.drive.set_pin(&path("B"), None).unwrap();
    assert_eq!(s.drive.pin_rules().unwrap(), vec![]);
}

#[tokio::test]
async fn local_changes_survive_a_deletion_in_proton() {
    let s = Setup::new(PinState::Unpinned).await;
    let file = s.remote_file(&s.root, "doc.txt", b"old").await;
    s.drive.refresh().await.unwrap();
    let ino = s.ino("doc.txt").await;
    s.drive.open(ino, true).await.unwrap();
    s.drive.write(ino, 0, b"new").unwrap();
    s.remote.trash(&file.id).await.unwrap();
    s.drive.refresh().await.unwrap();
    s.drive.release(ino).await.unwrap();
    assert_eq!(s.drive.upload_pending().await, vec![]);
    let restored = s.ino("doc.txt").await;
    assert_eq!(s.read_all(restored).await, b"new");
    assert_eq!(s.remote_names(&s.root).await, ["doc.txt"]);
}

#[tokio::test]
async fn truncating_loads_only_what_is_kept() {
    let s = Setup::new(PinState::Unpinned).await;
    let content = bytes(3 * BLOCK_BYTES, 3);
    let other = bytes(3 * BLOCK_BYTES, 5);
    let file = s.remote_file(&s.root, "big.bin", &content).await;
    s.remote_file(&s.root, "part.bin", &other).await;
    let ino = s.ino("big.bin").await;

    s.drive.set_size(ino, 0).await.unwrap();
    assert_eq!(s.remote.range_bytes(), 0, "truncating to 0 loads nothing");
    s.drive.open(ino, true).await.unwrap();
    s.drive.write(ino, 0, b"new").unwrap();
    s.drive.release(ino).await.unwrap();
    assert_eq!(s.remote.range_bytes(), 0);
    assert_eq!(s.remote_content(&file).await, b"new");

    let ino = s.ino("part.bin").await;
    s.drive.set_size(ino, 10).await.unwrap();
    assert_eq!(s.remote.range_bytes(), BLOCK_BYTES, "only the first block");
    assert_eq!(s.read_all(ino).await, other[..10]);
}

#[tokio::test]
async fn freeing_a_folder_removes_its_contents_at_once() {
    let s = Setup::new(PinState::Unpinned).await;
    let videos = s.remote_folder("Videos").await;
    s.remote_file(&videos, "a.mkv", &bytes(300, 1)).await;
    s.remote_file(&videos, "b.mkv", &bytes(200, 2)).await;
    s.remote_file(&videos, "fixed.mkv", &bytes(100, 3)).await;
    s.remote_file(&s.root, "other.txt", &bytes(50, 4)).await;
    for file in [
        "Videos/a.mkv",
        "Videos/b.mkv",
        "Videos/fixed.mkv",
        "other.txt",
    ] {
        let ino = s.ino(file).await;
        s.read_all(ino).await;
    }
    s.drive
        .set_pin(&path("Videos/fixed.mkv"), Some(PinState::Pinned))
        .unwrap();

    let planned = s.drive.free(&path("Videos"), true).await.unwrap();
    assert_eq!(planned.bytes, 500);
    assert_eq!(planned.skipped, 1, "pinned stays");
    assert_eq!(s.drive.usage().unwrap().bytes, 650, "dry run");

    let freed = s.drive.free(&path("Videos"), false).await.unwrap();
    let mut files = freed.files.clone();
    files.sort();
    assert_eq!(files, [path("Videos/a.mkv"), path("Videos/b.mkv")]);
    assert_eq!(s.drive.usage().unwrap().bytes, 150);
    assert_eq!(
        s.drive.free(&path("Videos"), false).await.unwrap().bytes,
        0,
        "nothing left"
    );
}

#[tokio::test]
async fn path_status_comes_from_memory_without_asking_proton() {
    let mut s = Setup::new(PinState::Unpinned).await;
    let docs = s.remote_folder("Docs").await;
    let content = bytes(4 * BLOCK_BYTES, 3);
    s.remote_file(&docs, "big.bin", &content).await;
    s.remote_file(&docs, "small.txt", b"small").await;
    let status = |s: &Setup, text: &str| s.drive.path_status(&path(text)).unwrap();
    let content_of = |s: &Setup, text: &str| status(s, text).content;

    let before = s.remote.list_calls();
    assert_eq!(
        content_of(&s, "Docs/small.txt"),
        DriveContent::OnlineOnly,
        "never read"
    );
    assert_eq!(s.remote.list_calls(), before, "no network traffic");

    let big = s.ino("Docs/big.bin").await;
    s.drive.read(big, 0, 10).await.unwrap();
    let small = s.ino("Docs/small.txt").await;
    s.read_all(small).await;
    assert_eq!(content_of(&s, "Docs"), DriveContent::Folder);
    assert_eq!(content_of(&s, "Docs/big.bin"), DriveContent::Partial);
    assert_eq!(content_of(&s, "Docs/small.txt"), DriveContent::Local);

    s.drive.open(small, true).await.unwrap();
    s.drive.write(small, 0, b"new").unwrap();
    assert_eq!(
        content_of(&s, "Docs/small.txt"),
        DriveContent::PendingUpload
    );
    s.drive.release(small).await.unwrap();

    s.drive
        .set_pin(&path("Docs"), Some(PinState::Pinned))
        .unwrap();
    assert!(
        status(&s, "Docs/small.txt").pinned,
        "the folder's rule applies"
    );
    assert!(!status(&s, "another.txt").pinned);

    s.restart();
    assert_eq!(
        content_of(&s, "Docs/small.txt"),
        DriveContent::Local,
        "from the index after the restart"
    );
}
