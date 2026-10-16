//! FUSE-Laufwerke im Daemon-Kern gegen Fakes: verbinden und einhängen nach
//! der Anmeldung, Anheften, Freigeben, Papierkorb, Aushängen beim Abmelden.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use protonsync_adapter_fs::FsContentCache;
use protonsync_api::{ErrorCode, PathState, PinChoice, Request, Response, TrashSide};
use protonsync_application::drive::ROOT_INO;
use protonsync_application::ports::RemoteStorage;
use protonsync_daemon::core::{Core, DaemonDrive, DriveMounter, MountGuard, MountSetup, Settings};
use protonsync_daemon::mount_config::MountConfig;
use protonsync_domain::node::NodeName;
use protonsync_domain::path::RelativePath;
use protonsync_domain::pin::EvictionPolicy;
use protonsync_domain::time::Timestamp;
use protonsync_domain::trash::RetentionPeriod;
use protonsync_testkit::{FakeAccount, FakeLocal, FakeRemote, FixedClock, InMemoryStateRepo};

const T0: Timestamp = Timestamp::from_unix_seconds(1_760_000_000);
const MOUNTPOINT: &str = "/home/k/ProtonDrive";

type State = Arc<InMemoryStateRepo>;
type Clock = Arc<FixedClock>;
type Drive = DaemonDrive<Arc<FakeRemote>, State, Clock>;
type TestCore = Core<FakeAccount, Arc<FakeLocal>, State, Clock>;

/// Hängt nicht wirklich ein, merkt sich aber Laufwerk und Aushängen.
#[derive(Default)]
struct FakeMounter {
    drives: Mutex<Vec<(PathBuf, Arc<Drive>)>>,
    unmounted: Arc<AtomicUsize>,
    fail: bool,
}

/// Zählt beim Fallenlassen ein Aushängen.
struct Guard(Arc<AtomicUsize>);

impl Drop for Guard {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl DriveMounter<Drive> for FakeMounter {
    fn mount(&self, mountpoint: &Path, drive: Arc<Drive>) -> Result<MountGuard, String> {
        if self.fail {
            return Err("fusermount3 fehlt".into());
        }
        self.drives
            .lock()
            .unwrap()
            .push((mountpoint.to_path_buf(), drive));
        Ok(Box::new(Guard(Arc::clone(&self.unmounted))))
    }
}

impl FakeMounter {
    fn drive(&self) -> Arc<Drive> {
        Arc::clone(&self.drives.lock().unwrap().last().unwrap().1)
    }
}

struct Fixture {
    core: TestCore,
    remote: Arc<FakeRemote>,
    mounter: Arc<FakeMounter>,
    _cache_dir: tempfile::TempDir,
}

fn name(text: &str) -> NodeName {
    NodeName::new(text).unwrap()
}

async fn fixture_with(mounter: FakeMounter, dry_run: bool) -> Fixture {
    let remote = Arc::new(FakeRemote::new());
    let cache_dir = tempfile::tempdir().unwrap();
    let mounter = Arc::new(mounter);
    let shared: Arc<dyn DriveMounter<Drive>> = mounter.clone();
    let core = Core::new(
        FakeAccount::new(Arc::clone(&remote)).with_stored_session(),
        Arc::new(InMemoryStateRepo::new()),
        Arc::new(FixedClock::at(T0)),
        Settings {
            dry_run,
            retention: RetentionPeriod::default(),
        },
        Vec::new(),
    )
    .with_mounts(
        vec![MountSetup {
            config: MountConfig {
                id: "home".into(),
                mountpoint: MOUNTPOINT.into(),
                remote: RelativePath::root(),
                keep_local: false,
            },
            cache: FsContentCache::open(cache_dir.path().join("home")).unwrap(),
        }],
        EvictionPolicy::new(30, u64::MAX).unwrap(),
        Some(shared),
    );
    core.startup().await;
    Fixture {
        core,
        remote,
        mounter,
        _cache_dir: cache_dir,
    }
}

async fn fixture() -> Fixture {
    fixture_with(FakeMounter::default(), false).await
}

impl Fixture {
    async fn remote_file(&self, label: &str, content: &[u8]) {
        let root = self.remote.root_folder().await.unwrap();
        self.remote
            .upload_file(&root.id, &name(label), content, content.len() as u64, T0)
            .await
            .unwrap();
    }

    async fn mounts(&self) -> Vec<protonsync_api::MountInfo> {
        match self.core.handle(Request::Mounts).await {
            Response::Mounts { mounts } => mounts,
            other => panic!("{other:?}"),
        }
    }
}

fn at(rest: &str) -> String {
    format!("{MOUNTPOINT}/{rest}")
}

#[tokio::test]
async fn drives_are_mounted_after_login_and_unmounted_on_logout() {
    let f = fixture().await;
    let mounts = f.mounts().await;
    assert_eq!(mounts.len(), 1);
    assert!(mounts[0].mounted);
    assert_eq!(mounts[0].cache.unwrap().files, 0);
    assert_eq!(
        f.mounter.drives.lock().unwrap()[0].0,
        PathBuf::from(MOUNTPOINT)
    );

    assert_eq!(f.core.handle(Request::Logout).await, Response::Done);
    assert_eq!(f.mounter.unmounted.load(Ordering::SeqCst), 1);
    let mounts = f.mounts().await;
    assert!(!mounts[0].mounted);
    assert_eq!(mounts[0].cache, None);
}

#[tokio::test]
async fn a_failed_mount_still_allows_pinning() {
    let f = fixture_with(
        FakeMounter {
            fail: true,
            ..FakeMounter::default()
        },
        false,
    )
    .await;
    f.remote_file("a.txt", b"x").await;
    let mounts = f.mounts().await;
    assert!(!mounts[0].mounted);
    assert!(mounts[0].cache.is_some(), "verbunden");
    let pin = Request::Pin {
        path: at("a.txt"),
        state: PinChoice::Pinned,
    };
    assert_eq!(f.core.handle(pin).await, Response::Done);
}

#[tokio::test]
async fn pinning_checks_the_path_and_records_the_rule() {
    let f = fixture().await;
    f.remote_file("Fotos.zip", b"bilder").await;
    let pin = |path: String, state| Request::Pin { path, state };

    assert_eq!(
        f.core.handle(pin(at("Fotos.zip"), PinChoice::Pinned)).await,
        Response::Done
    );
    let mounts = f.mounts().await;
    assert_eq!(mounts[0].pins.len(), 1);
    assert_eq!(mounts[0].pins[0].path, "Fotos.zip");
    assert_eq!(mounts[0].pins[0].state, PinChoice::Pinned);

    for path in [at("fehlt"), "/anderswo/a".into(), at("../x")] {
        match f.core.handle(pin(path.clone(), PinChoice::Pinned)).await {
            Response::Error(error) => assert_eq!(error.code, ErrorCode::NotFound, "{path}"),
            other => panic!("{path}: {other:?}"),
        }
    }
}

#[tokio::test]
async fn freeing_plans_first_and_then_removes_local_copies() {
    let f = fixture().await;
    f.remote_file("video.mkv", &[7; 1000]).await;
    let drive = f.mounter.drive();
    let ino = drive
        .lookup(ROOT_INO, &name("video.mkv"))
        .await
        .unwrap()
        .ino;
    drive.read(ino, 0, 1000).await.unwrap();
    assert_eq!(f.mounts().await[0].cache.unwrap().bytes, 1000);

    let free = |apply| Request::Free {
        path: Some(MOUNTPOINT.into()),
        apply,
    };
    let Response::Freed { results } = f.core.handle(free(false)).await else {
        panic!("Freigabe erwartet");
    };
    assert!(results[0].dry_run);
    assert_eq!(results[0].files, ["video.mkv"]);
    assert_eq!(f.mounts().await[0].cache.unwrap().bytes, 1000, "Probelauf");

    let Response::Freed { results } = f.core.handle(free(true)).await else {
        panic!("Freigabe erwartet");
    };
    assert_eq!(results[0].bytes, 1000);
    assert_eq!(f.mounts().await[0].cache.unwrap().bytes, 0);

    // Nach der Regel (30 Tage) ist nichts fällig.
    let Response::Freed { results } = f
        .core
        .handle(Request::Free {
            path: None,
            apply: true,
        })
        .await
    else {
        panic!("Freigabe erwartet");
    };
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].files, Vec::<String>::new());
}

#[tokio::test]
async fn deleted_files_can_be_restored_from_the_trash() {
    let f = fixture().await;
    f.remote_file("wichtig.txt", b"bitte behalten").await;
    let drive = f.mounter.drive();
    drive
        .remove_file(ROOT_INO, &name("wichtig.txt"))
        .await
        .unwrap();

    let Response::Trash { entries } = f.core.handle(Request::Trash).await else {
        panic!("Papierkorb erwartet");
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].pair, "mount:home");
    assert_eq!(entries[0].path, "wichtig.txt");
    assert_eq!(entries[0].side, TrashSide::Remote);

    let restore = Request::RestoreTrash {
        pair: "mount:home".into(),
        id: entries[0].id,
    };
    assert_eq!(f.core.handle(restore).await, Response::Done);
    let root = f.remote.root_folder().await.unwrap();
    let names: Vec<String> = f
        .remote
        .list_children(&root.id)
        .await
        .unwrap()
        .into_iter()
        .map(|node| node.name.to_string())
        .collect();
    assert_eq!(names, ["wichtig.txt"]);
}

#[tokio::test]
async fn maintenance_uploads_pending_changes() {
    let f = fixture().await;
    let drive = f.mounter.drive();
    let created = drive.create(ROOT_INO, &name("neu.txt")).await.unwrap();
    drive.write(created.ino, 0, b"offline geschrieben").unwrap();
    f.remote
        .fail_next_call(protonsync_application::ports::RemoteError::Unavailable(
            "Netz weg".into(),
        ));
    // Das Hochladen beim Schließen scheitert; die Datei bleibt vorgemerkt.
    assert!(drive.release(created.ino).await.is_err());
    assert_eq!(f.mounts().await[0].cache.unwrap().pending_uploads, 1);

    f.core.maintain_mount(0).await;
    assert_eq!(f.mounts().await[0].cache.unwrap().pending_uploads, 0);
}

async fn path_states(core: &TestCore, paths: &[String]) -> Vec<(PathState, bool, bool)> {
    match core
        .handle(Request::PathStatus {
            paths: paths.to_vec(),
        })
        .await
    {
        Response::PathStatus { entries } => entries
            .into_iter()
            .map(|e| (e.state, e.in_drive, e.pinned))
            .collect(),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn path_status_reports_placeholders_downloads_and_pins() {
    let f = fixture().await;
    f.remote_file("a.txt", b"hallo").await;
    f.remote_file("b.txt", b"welt").await;
    let drive = f.mounter.drive();
    let ino = drive
        .resolve(&RelativePath::parse("a.txt").unwrap())
        .await
        .unwrap();
    drive.read(ino, 0, 5).await.unwrap();
    f.core
        .handle(Request::Pin {
            path: at("b.txt"),
            state: PinChoice::Pinned,
        })
        .await;

    let paths = [at("a.txt"), at("b.txt"), MOUNTPOINT.into(), "/tmp/x".into()];
    let states = path_states(&f.core, &paths).await;
    assert_eq!(states[0], (PathState::Local, true, false));
    assert!(states[1].2, "angeheftet");
    assert_eq!(states[2], (PathState::Folder, true, false));
    assert_eq!(states[3], (PathState::Outside, false, false));
}
