//! Virtuelles Laufwerk gegen die Fakes: Lesen bei Bedarf, Schreiben und
//! Hochladen, Konflikte, Papierkorb, Umbenennen, Anheften, Freigabe, Neustart.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use protonsync_application::drive::{DriveError, DriveSettings, Ino, ROOT_INO, VirtualDrive};
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
        let root = ensure_folder(&*remote, &start, &path("Laufwerk"))
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

    /// Simuliert einen Neustart des Daemons: neues Laufwerk, gleicher Zustand.
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
    s.remote_file(&docs, "a.txt", b"hallo").await;
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
    assert_eq!(s.remote.list_calls() - before, 2, "je Ordner ein Aufruf");
    assert_eq!(
        s.drive.lookup(docs_ino, &name("fehlt")).await,
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
        "Block plus Vorauslesen"
    );
    s.drive.read(ino, 100, 4096).await.unwrap();
    assert_eq!(s.remote.range_bytes(), 2 * BLOCK_BYTES, "schon vorhanden");

    let tail_offset = 3 * BLOCK_BYTES + 90;
    let tail = s.drive.read(ino, tail_offset, 4096).await.unwrap();
    assert_eq!(tail, content[usize::try_from(tail_offset).unwrap()..]);
    assert_eq!(s.remote.range_bytes(), 2 * BLOCK_BYTES + 100);
    assert_eq!(s.read_all(ino).await, content);
    assert_eq!(
        s.remote.range_bytes(),
        content.len() as u64,
        "nichts doppelt"
    );
    assert_eq!(s.drive.usage().unwrap().bytes, content.len() as u64);
}

#[tokio::test]
async fn a_new_revision_in_proton_replaces_the_cached_copy() {
    let s = Setup::new(PinState::Unpinned).await;
    let file = s.remote_file(&s.root, "a.txt", b"alt").await;
    s.drive.refresh().await.unwrap();
    let ino = s.ino("a.txt").await;
    assert_eq!(s.read_all(ino).await, b"alt");

    s.remote
        .upload_revision(&file.id, &b"neuer Inhalt"[..], 12, NOW)
        .await
        .unwrap();
    let report = s.drive.refresh().await.unwrap();
    assert!(report.events > 0);
    assert_eq!(s.drive.attributes(ino).unwrap().size, 12);
    assert_eq!(s.read_all(ino).await, b"neuer Inhalt");
}

#[tokio::test]
async fn changed_files_are_uploaded_as_a_new_revision_after_closing() {
    let s = Setup::new(PinState::Unpinned).await;
    let file = s.remote_file(&s.root, "notiz.txt", b"eins").await;
    let ino = s.ino("notiz.txt").await;
    s.drive.open(ino, true).await.unwrap();
    s.drive.write(ino, 4, b" zwei").unwrap();
    assert_eq!(s.drive.attributes(ino).unwrap().size, 9);
    assert_eq!(s.remote_content(&file).await, b"eins", "noch geöffnet");
    assert_eq!(s.drive.usage().unwrap().pending_uploads, 1);
    s.drive.release(ino).await.unwrap();
    assert_eq!(s.remote_content(&file).await, b"eins zwei");
    assert_eq!(s.drive.usage().unwrap().pending_uploads, 0);
    assert_eq!(s.remote_names(&s.root).await, ["notiz.txt"]);
}

#[tokio::test]
async fn new_files_appear_in_proton_after_closing() {
    let s = Setup::new(PinState::Unpinned).await;
    let docs = s.remote_folder("Docs").await;
    let docs_ino = s.ino("Docs").await;
    let created = s.drive.create(docs_ino, &name("neu.txt")).await.unwrap();
    assert_eq!(
        s.drive.create(docs_ino, &name("neu.txt")).await,
        Err(DriveError::AlreadyExists)
    );
    s.drive.write(created.ino, 0, b"frisch").unwrap();
    assert_eq!(
        s.read_all(created.ino).await,
        b"frisch",
        "lesbar vor dem Upload"
    );
    s.drive.release(created.ino).await.unwrap();
    let uploaded = s.remote.list_children(&docs.id).await.unwrap();
    assert_eq!(uploaded.len(), 1);
    assert_eq!(s.remote_content(&uploaded[0]).await, b"frisch");
    // Danach ist es eine ganz normale Datei mit Knoten.
    s.write_file(created.ino, b"zweite Fassung").await;
    assert_eq!(s.remote_content(&uploaded[0]).await, b"zweite Fassung");
}

#[tokio::test]
async fn a_conflicting_change_in_proton_keeps_both_versions() {
    let s = Setup::new(PinState::Unpinned).await;
    let file = s.remote_file(&s.root, "plan.txt", b"basis").await;
    let ino = s.ino("plan.txt").await;
    s.drive.open(ino, true).await.unwrap();
    s.drive.write(ino, 0, b"LOKAL").unwrap();
    s.remote
        .upload_revision(&file.id, &b"entfernt"[..], 8, NOW)
        .await
        .unwrap();
    s.drive.release(ino).await.unwrap();

    assert_eq!(
        s.remote_content(&file).await,
        b"entfernt",
        "nichts überschrieben"
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
    assert_eq!(s.remote_content(&copy).await, b"LOKAL");
    // Im Laufwerk: Original zeigt Proton, die Kopie den lokalen Inhalt.
    assert_eq!(s.read_all(ino).await, b"entfernt");
    let copy_ino = s.drive.lookup(ROOT_INO, &copy.name).await.unwrap().ino;
    assert_eq!(s.read_all(copy_ino).await, b"LOKAL");
}

#[tokio::test]
async fn failed_uploads_stay_pending_and_survive_a_restart() {
    let mut s = Setup::new(PinState::Unpinned).await;
    s.remote_folder("Docs").await;
    let docs_ino = s.ino("Docs").await;
    let created = s
        .drive
        .create(docs_ino, &name("wichtig.txt"))
        .await
        .unwrap();
    s.drive.write(created.ino, 0, b"nicht verlieren").unwrap();
    s.remote
        .fail_next_call(protonsync_application::ports::RemoteError::Unavailable(
            "Netz".into(),
        ));
    assert!(s.drive.release(created.ino).await.is_err());
    assert_eq!(s.drive.usage().unwrap().pending_uploads, 1);

    s.restart();
    assert_eq!(
        s.drive.usage().unwrap().pending_uploads,
        1,
        "aus dem Zustand"
    );
    assert_eq!(s.drive.upload_pending().await, vec![]);
    assert_eq!(s.drive.usage().unwrap().pending_uploads, 0);
    let file = s.ino("Docs/wichtig.txt").await;
    assert_eq!(s.read_all(file).await, b"nicht verlieren");
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
    let archive = s.remote_folder("Archiv").await;
    s.remote_file(&docs, "a.txt", b"x").await;
    let docs_ino = s.ino("Docs").await;
    let archive_ino = s.ino("Archiv").await;
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
    let file = s.remote_file(&s.root, "brief.odt", b"Fassung 1").await;
    let original = s.ino("brief.odt").await;
    // So speichern viele Programme: Hilfsdatei schreiben, dann darüber umbenennen.
    let temp = s
        .drive
        .create(ROOT_INO, &name(".~brief.tmp"))
        .await
        .unwrap();
    s.drive.write(temp.ino, 0, b"Fassung 2").unwrap();
    s.drive.release(temp.ino).await.unwrap();
    s.drive
        .rename(ROOT_INO, &name(".~brief.tmp"), ROOT_INO, &name("brief.odt"))
        .await
        .unwrap();
    assert_eq!(s.remote_content(&file).await, b"Fassung 2");
    assert_eq!(
        s.remote_names(&s.root).await,
        ["brief.odt"],
        "Hilfsdatei im Papierkorb"
    );
    assert_eq!(s.read_all(original).await, b"Fassung 2");
    assert_eq!(
        s.drive.lookup(ROOT_INO, &name(".~brief.tmp")).await,
        Err(DriveError::NotFound)
    );
}

#[tokio::test]
async fn pinned_folders_are_loaded_and_never_evicted() {
    let s = Setup::new(PinState::Unpinned).await;
    let keep = s.remote_folder("Behalten").await;
    let sub = s
        .remote
        .create_folder(&keep.id, &name("Unter"))
        .await
        .unwrap();
    s.remote_file(&keep, "a.bin", &bytes(1000, 1)).await;
    s.remote_file(&sub, "b.bin", &bytes(2000, 2)).await;
    s.remote_file(&s.root, "lose.bin", &bytes(500, 3)).await;
    s.drive
        .set_pin(&path("Behalten"), Some(PinState::Pinned))
        .unwrap();
    assert_eq!(
        s.drive.pin_state(&path("Behalten/Unter/b.bin")).unwrap(),
        PinState::Pinned
    );
    assert_eq!(
        s.drive.pin_state(&path("lose.bin")).unwrap(),
        PinState::Unpinned
    );

    let report = s.drive.prefetch_pinned().await.unwrap();
    assert_eq!((report.files, report.bytes), (2, 3000));
    assert_eq!(
        s.drive.prefetch_pinned().await.unwrap().files,
        0,
        "schon da"
    );

    let loose = s.ino("lose.bin").await;
    s.read_all(loose).await;
    s.clock.advance_days(40);
    let policy = EvictionPolicy::new(30, u64::MAX).unwrap();
    let freed = s.drive.evict(policy, false).await.unwrap();
    assert_eq!(freed.files, [path("lose.bin")]);
    assert_eq!(freed.bytes, 500);
    assert_eq!(s.drive.usage().unwrap().bytes, 3000);
    // Freigegeben heißt: Platzhalter bleibt, Inhalt kommt beim Lesen wieder.
    assert_eq!(s.read_all(loose).await, bytes(500, 3));
}

#[tokio::test]
async fn eviction_plans_first_and_keeps_changed_and_open_files() {
    let s = Setup::new(PinState::Unpinned).await;
    s.remote_file(&s.root, "alt.bin", &bytes(100, 1)).await;
    s.remote_file(&s.root, "offen.bin", &bytes(100, 2)).await;
    let old = s.ino("alt.bin").await;
    let open = s.ino("offen.bin").await;
    s.read_all(old).await;
    s.drive.open(open, false).await.unwrap();
    s.read_all(open).await;
    let created = s.drive.create(ROOT_INO, &name("neu.txt")).await.unwrap();
    s.drive.write(created.ino, 0, b"ungesichert").unwrap();
    s.clock.advance_days(40);
    let policy = EvictionPolicy::new(30, 0).unwrap();

    let planned = s.drive.evict(policy, true).await.unwrap();
    assert_eq!(planned.files, [path("alt.bin")]);
    assert_eq!(s.drive.usage().unwrap().files, 3, "Probelauf ändert nichts");
    let done = s.drive.evict(policy, false).await.unwrap();
    assert_eq!(done.files, [path("alt.bin")]);
    assert_eq!(s.drive.usage().unwrap().files, 2);
}

#[tokio::test]
async fn pins_follow_renamed_folders_and_can_be_cleared() {
    let s = Setup::new(PinState::Pinned).await;
    s.remote_folder("A").await;
    assert_eq!(
        s.drive.pin_state(&path("A/x")).unwrap(),
        PinState::Pinned,
        "Voreinstellung"
    );
    s.drive
        .set_pin(&path("A"), Some(PinState::Unpinned))
        .unwrap();
    s.drive
        .set_pin(&path("A/wichtig"), Some(PinState::Pinned))
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
            (path("B/wichtig"), PinState::Pinned)
        ]
    );
    s.drive.set_pin(&path("B"), None).unwrap();
    assert_eq!(s.drive.pin_rules().unwrap(), vec![]);
}

#[tokio::test]
async fn local_changes_survive_a_deletion_in_proton() {
    let s = Setup::new(PinState::Unpinned).await;
    let file = s.remote_file(&s.root, "doku.txt", b"alt").await;
    s.drive.refresh().await.unwrap();
    let ino = s.ino("doku.txt").await;
    s.drive.open(ino, true).await.unwrap();
    s.drive.write(ino, 0, b"neu").unwrap();
    s.remote.trash(&file.id).await.unwrap();
    s.drive.refresh().await.unwrap();
    s.drive.release(ino).await.unwrap();
    assert_eq!(s.drive.upload_pending().await, vec![]);
    let restored = s.ino("doku.txt").await;
    assert_eq!(s.read_all(restored).await, b"neu");
    assert_eq!(s.remote_names(&s.root).await, ["doku.txt"]);
}

#[tokio::test]
async fn truncating_loads_only_what_is_kept() {
    let s = Setup::new(PinState::Unpinned).await;
    let content = bytes(3 * BLOCK_BYTES, 3);
    let other = bytes(3 * BLOCK_BYTES, 5);
    let file = s.remote_file(&s.root, "gross.bin", &content).await;
    s.remote_file(&s.root, "teil.bin", &other).await;
    let ino = s.ino("gross.bin").await;

    s.drive.set_size(ino, 0).await.unwrap();
    assert_eq!(s.remote.range_bytes(), 0, "Abschneiden auf 0 lädt nichts");
    s.drive.open(ino, true).await.unwrap();
    s.drive.write(ino, 0, b"neu").unwrap();
    s.drive.release(ino).await.unwrap();
    assert_eq!(s.remote.range_bytes(), 0);
    assert_eq!(s.remote_content(&file).await, b"neu");

    let ino = s.ino("teil.bin").await;
    s.drive.set_size(ino, 10).await.unwrap();
    assert_eq!(s.remote.range_bytes(), BLOCK_BYTES, "nur der erste Block");
    assert_eq!(s.read_all(ino).await, other[..10]);
}
