//! Safety tests of the photo sync (ADR 0012): with `download_only` (the
//! default) nothing in Proton Photos is ever uploaded, trashed, restored or
//! deleted; with `protect_remote` nothing is ever trashed or deleted.
//!
//! Every run goes through a [`RecordingPhotoLibrary`]; random changes from the
//! phone and on the local side are mixed by property tests.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;

use proptest::prelude::*;
use protonsync_application::photos::{
    PhotoIndex, PhotoOutcome, PhotoReport, PhotoSync, photos_owner,
};
use protonsync_application::ports::{PhotoBin, RemoteError, UtcCalendar};
use protonsync_application::use_cases::TrashService;
use protonsync_domain::node::{NodeId, RevisionId};
use protonsync_domain::sync_pair::SyncDirection;
use protonsync_domain::time::Timestamp;
use protonsync_domain::trash::RetentionPeriod;
use protonsync_testkit::{
    FakeLocal, FakePhotoLibrary, FixedClock, InMemoryStateRepo, PhotoWrite, RecordingPhotoLibrary,
};

/// 2026-10-16T13:45:59Z
const OCTOBER: i64 = 1_792_158_359;
const NOW: Timestamp = Timestamp::from_unix_seconds(1_800_000_000);

struct Fixture {
    library: FakePhotoLibrary,
    local: FakeLocal,
    state: InMemoryStateRepo,
    clock: FixedClock,
    direction: SyncDirection,
    protect_remote: bool,
}

impl Fixture {
    fn new(direction: SyncDirection, protect_remote: bool) -> Self {
        Self {
            library: FakePhotoLibrary::new(),
            local: FakeLocal::new(),
            state: InMemoryStateRepo::new(),
            clock: FixedClock::at(NOW),
            direction,
            protect_remote,
        }
    }

    /// Content of every photo outside the Proton trash.
    fn library_state(&self) -> BTreeMap<NodeId, Option<Vec<u8>>> {
        self.library
            .active_ids()
            .into_iter()
            .map(|id| {
                let content = self.library.content(&id);
                (id, content)
            })
            .collect()
    }

    /// A run through the recorder; returns the report (if the run got that far) and the calls.
    async fn run(&self, index: &mut PhotoIndex) -> (Option<PhotoReport>, Vec<PhotoWrite>) {
        let recorder = RecordingPhotoLibrary::new(&self.library);
        let sync = PhotoSync {
            library: &recorder,
            local: &self.local,
            state: &self.state,
            clock: &self.clock,
            calendar: &UtcCalendar,
            direction: self.direction,
            protect_remote: self.protect_remote,
        };
        let report = match sync.run(index, false).await {
            Ok(PhotoOutcome::Applied(report)) => Some(report),
            Ok(PhotoOutcome::Planned(plan)) => panic!("plan instead of result: {plan:?}"),
            Err(_) => None,
        };
        (report, recorder.writes())
    }

    /// A run that must not change Proton Photos at all.
    async fn run_read_only(&self, index: &mut PhotoIndex) -> Option<PhotoReport> {
        let before = self.library_state();
        let (report, writes) = self.run(index).await;
        assert_eq!(
            writes,
            [],
            "{:?} changed Proton Photos: {report:?}",
            self.direction
        );
        assert_eq!(self.library_state(), before, "library changed: {report:?}");
        report
    }

    /// A run with `protect_remote`: it may upload, but never remove or replace anything.
    async fn run_protected(&self, index: &mut PhotoIndex) {
        let before = self.library_state();
        let (report, writes) = self.run(index).await;
        for write in &writes {
            assert!(
                matches!(write, PhotoWrite::Upload(_)),
                "protect_remote removed something: {write:?}\n{report:?}"
            );
        }
        let after = self.library_state();
        for (id, content) in before {
            assert_eq!(after.get(&id), Some(&content), "{id:?} gone or changed");
        }
    }

    fn new_revision(&self, id: &NodeId, content: &[u8]) {
        self.library.modify(id, |photo, data| {
            photo.revision =
                RevisionId::new(format!("{}-{}", photo.revision.as_str(), content.len()));
            *data = content.to_vec();
        });
    }
}

#[tokio::test]
async fn download_only_never_changes_proton_photos() {
    for protect_remote in [false, true] {
        let f = Fixture::new(SyncDirection::DownloadOnly, protect_remote);
        let mut index = PhotoIndex::new();
        let kept = f.library.add_from_phone("kept.jpg", b"kept", OCTOBER);
        let edited = f.library.add_from_phone("edited.jpg", b"v1", OCTOBER);
        let gone = f.library.add_from_phone("gone.jpg", b"gone", OCTOBER);
        f.run_read_only(&mut index).await;

        // Local: delete, edit, add new photos and other files.
        f.local.remove("2026/10/kept.jpg");
        f.local.write("2026/10/edited.jpg", b"local edit", NOW);
        f.local.write("2026/10/gone.jpg", b"local edit too", NOW);
        f.local.write("own/new.jpg", b"new", NOW);
        f.local.write("own/clip.mp4", b"clip", NOW);
        // Phone: new revision and a deletion of edited files.
        f.new_revision(&edited.id, b"v2");
        f.library.trash_from_phone(&gone.id);

        for _ in 0..3 {
            f.run_read_only(&mut index).await;
        }
        assert!(!f.library.is_trashed(&kept.id));
        assert_eq!(f.library.active_ids().len(), 2);
        assert_eq!(
            f.local.content("2026/10/gone.jpg").as_deref(),
            Some(&b"local edit too"[..])
        );
        // Restart: the index is gone, the stored state remains.
        let mut fresh = PhotoIndex::new();
        f.run_read_only(&mut fresh).await;
    }
}

#[tokio::test]
async fn photo_failures_and_trash_never_write_in_download_only() {
    let f = Fixture::new(SyncDirection::DownloadOnly, false);
    let mut index = PhotoIndex::new();
    let photo = f.library.add_from_phone("a.jpg", b"a", OCTOBER);
    f.library.add_from_phone("b.jpg", b"b", OCTOBER);
    f.run_read_only(&mut index).await;

    f.library
        .fail_next_call(RemoteError::Unavailable("network".into()));
    f.run_read_only(&mut index).await;
    f.library.trash_from_phone(&photo.id);
    f.library.push_gap();
    f.run_read_only(&mut index).await;

    // The local copy went to the local trash; restoring and purging stay local.
    let recorder = RecordingPhotoLibrary::new(&f.library);
    let bin = PhotoBin(&recorder);
    let trash = TrashService {
        remote: &bin,
        local: &f.local,
        state: &f.state,
        clock: &f.clock,
        pair: &photos_owner(),
    };
    let entries = trash.list().unwrap();
    assert_eq!(entries.len(), 1);
    trash.restore(entries[0].0).await.unwrap();
    f.clock.advance_days(400);
    trash
        .purge_expired(RetentionPeriod::default(), false)
        .await
        .unwrap();
    assert_eq!(recorder.writes(), []);
}

// ---------------------------------------------------------------------------
// Property tests.

#[derive(Debug, Clone)]
enum Op {
    PhoneAdd(u8, u8),
    PhoneTrash(usize),
    PhoneNewRevision(usize, u8),
    LocalWrite(usize, u8),
    LocalRemove(usize),
    LocalNew(u8, u8),
    Sync,
    Restart,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0..4_u8, any::<u8>()).prop_map(|(n, c)| Op::PhoneAdd(n, c)),
        (0..8_usize).prop_map(Op::PhoneTrash),
        (0..8_usize, any::<u8>()).prop_map(|(i, c)| Op::PhoneNewRevision(i, c)),
        (0..8_usize, any::<u8>()).prop_map(|(i, c)| Op::LocalWrite(i, c)),
        (0..8_usize).prop_map(Op::LocalRemove),
        (0..4_u8, any::<u8>()).prop_map(|(n, c)| Op::LocalNew(n, c)),
        Just(Op::Sync),
        Just(Op::Restart),
    ]
}

fn pick<T: Clone>(items: &[T], index: usize) -> Option<T> {
    (!items.is_empty()).then(|| items[index % items.len()].clone())
}

/// Applies a change that is not a run.
fn change(f: &Fixture, op: &Op, step: i64) {
    let now = Timestamp::from_unix_seconds(NOW.unix_seconds() + step);
    match op {
        Op::PhoneAdd(name, content) => {
            f.library
                .add_from_phone(&format!("p{name}.jpg"), &[*content, 1], OCTOBER);
        }
        Op::PhoneTrash(index) => {
            if let Some(id) = pick(&f.library.active_ids(), *index) {
                f.library.trash_from_phone(&id);
            }
        }
        Op::PhoneNewRevision(index, content) => {
            if let Some(id) = pick(&f.library.active_ids(), *index) {
                f.new_revision(&id, &[*content, 2, 2]);
            }
        }
        Op::LocalWrite(index, content) => {
            if let Some(path) = pick(&f.local.paths(), *index)
                && f.local.content(&path).is_some()
            {
                f.local.write(&path, &[*content, 3], now);
            }
        }
        Op::LocalRemove(index) => {
            if let Some(path) = pick(&f.local.paths(), *index) {
                f.local.remove(&path);
            }
        }
        Op::LocalNew(name, content) => {
            f.local
                .write(&format!("own/l{name}.jpg"), &[*content, 4], now);
        }
        Op::Sync | Op::Restart => {}
    }
}

fn run_scenario(direction: SyncDirection, protect_remote: bool, ops: &[Op]) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(async {
        let f = Fixture::new(direction, protect_remote);
        let mut index = PhotoIndex::new();
        for (step, op) in (0_i64..).zip(ops) {
            if matches!(op, Op::Restart) {
                index = PhotoIndex::new();
            }
            if matches!(op, Op::Sync | Op::Restart) {
                if direction.allows_upload() {
                    f.run_protected(&mut index).await;
                } else {
                    f.run_read_only(&mut index).await;
                }
            } else {
                change(&f, op, step);
            }
        }
    });
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn download_only_photos_never_change_proton(ops in prop::collection::vec(op(), 1..30)) {
        run_scenario(SyncDirection::DownloadOnly, false, &ops);
    }

    #[test]
    fn protected_two_way_photos_never_remove_from_proton(
        ops in prop::collection::vec(op(), 1..30)
    ) {
        run_scenario(SyncDirection::TwoWay, true, &ops);
    }

    #[test]
    fn protected_upload_only_photos_never_remove_from_proton(
        ops in prop::collection::vec(op(), 1..30)
    ) {
        run_scenario(SyncDirection::UploadOnly, true, &ops);
    }
}
