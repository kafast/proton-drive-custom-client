//! Szenariotests für den Foto-Abgleich und die Duplikate (ADR 0012).

#![allow(clippy::unwrap_used, clippy::panic)]

use protonsync_application::photos::{
    DuplicateService, IndexRefresh, PhotoError, PhotoIndex, PhotoOutcome, PhotoReport,
    PhotoStepKind, PhotoSync, photos_owner,
};
use protonsync_application::ports::{PhotoBin, PhotoLibrary, RemoteError, StateRepo, UtcCalendar};
use protonsync_application::use_cases::TrashService;
use protonsync_domain::node::NodeId;
use protonsync_domain::photo::duplicates::{DuplicateReason, DuplicateSelectionError};
use protonsync_domain::sync_pair::SyncDirection;
use protonsync_domain::time::Timestamp;
use protonsync_testkit::{FakeLocal, FakePhotoLibrary, FixedClock, InMemoryStateRepo};

/// 2026-10-16T13:45:59Z
const OCTOBER: i64 = 1_792_158_359;
/// 2026-03-09T12:00:00Z
const MARCH: i64 = 1_773_057_600;
const NOW: Timestamp = Timestamp::from_unix_seconds(1_800_000_000);

struct Fixture {
    library: FakePhotoLibrary,
    local: FakeLocal,
    state: InMemoryStateRepo,
    clock: FixedClock,
}

impl Fixture {
    fn new() -> Self {
        Self {
            library: FakePhotoLibrary::new(),
            local: FakeLocal::new(),
            state: InMemoryStateRepo::new(),
            clock: FixedClock::at(NOW),
        }
    }

    fn sync(
        &self,
        direction: SyncDirection,
    ) -> PhotoSync<'_, FakePhotoLibrary, FakeLocal, InMemoryStateRepo, FixedClock> {
        PhotoSync {
            library: &self.library,
            local: &self.local,
            state: &self.state,
            clock: &self.clock,
            calendar: &UtcCalendar,
            direction,
        }
    }

    async fn run(&self, index: &mut PhotoIndex, direction: SyncDirection) -> PhotoReport {
        match self.sync(direction).run(index, false).await.unwrap() {
            PhotoOutcome::Applied(report) => report,
            PhotoOutcome::Planned(plan) => panic!("Plan statt Ergebnis: {plan:?}"),
        }
    }

    fn duplicates(&self) -> DuplicateService<'_, FakePhotoLibrary, InMemoryStateRepo, FixedClock> {
        DuplicateService {
            library: &self.library,
            state: &self.state,
            clock: &self.clock,
            calendar: &UtcCalendar,
        }
    }

    fn trash_entries(&self) -> usize {
        self.state.trash_entries(&photos_owner()).unwrap().len()
    }
}

const DOWNLOAD: SyncDirection = SyncDirection::DownloadOnly;
const UPLOAD: SyncDirection = SyncDirection::UploadOnly;
const TWO_WAY: SyncDirection = SyncDirection::TwoWay;

#[tokio::test]
async fn phone_photos_are_filed_by_month_and_fetched_once() {
    let f = Fixture::new();
    let mut index = PhotoIndex::new();
    f.library.add_from_phone("IMG_1.jpg", b"eins", OCTOBER);
    f.library.add_from_phone("IMG_2.jpg", b"zwei", MARCH);

    let report = f.run(&mut index, DOWNLOAD).await;
    assert_eq!(report.downloaded, 2, "{report:?}");
    assert_eq!(
        f.local.content("2026/10/IMG_1.jpg").as_deref(),
        Some(&b"eins"[..])
    );
    assert_eq!(
        f.local.content("2026/03/IMG_2.jpg").as_deref(),
        Some(&b"zwei"[..])
    );

    assert_eq!(f.run(&mut index, DOWNLOAD).await, PhotoReport::default());
    assert_eq!(f.library.list_calls(), 1);
}

#[tokio::test]
async fn same_name_in_the_same_month_gets_a_distinct_name() {
    let f = Fixture::new();
    let mut index = PhotoIndex::new();
    f.library.add_from_phone("IMG_1.jpg", b"Handy A", OCTOBER);
    let second = f
        .library
        .add_from_phone("IMG_1.jpg", b"Handy B", OCTOBER + 60);
    f.run(&mut index, DOWNLOAD).await;
    let suffix = &second.id.as_str()[second.id.as_str().len().saturating_sub(8)..];
    assert_eq!(
        f.local
            .content(&format!("2026/10/IMG_1 ({suffix}).jpg"))
            .as_deref(),
        Some(&b"Handy B"[..]),
        "{:?}",
        f.local.paths()
    );
}

#[tokio::test]
async fn photos_deleted_in_proton_go_to_the_local_trash() {
    let f = Fixture::new();
    let mut index = PhotoIndex::new();
    let photo = f.library.add_from_phone("weg.jpg", b"weg", OCTOBER);
    f.run(&mut index, DOWNLOAD).await;
    f.library.trash_from_phone(&photo.id);

    let report = f.run(&mut index, DOWNLOAD).await;
    assert_eq!(report.trashed_local, 1, "{report:?}");
    assert_eq!(f.local.content("2026/10/weg.jpg"), None);
    assert_eq!(f.local.trash_len(), 1);
    assert_eq!(f.trash_entries(), 1);
}

#[tokio::test]
async fn edited_copies_survive_a_deletion_in_proton() {
    let f = Fixture::new();
    let mut index = PhotoIndex::new();
    let photo = f.library.add_from_phone("bild.jpg", b"original", OCTOBER);
    f.run(&mut index, TWO_WAY).await;
    f.local.write(
        "2026/10/bild.jpg",
        b"bearbeitet",
        Timestamp::from_unix_seconds(OCTOBER + 99),
    );
    f.library.trash_from_phone(&photo.id);

    let report = f.run(&mut index, TWO_WAY).await;
    // Die bearbeitete Datei bleibt und gilt jetzt als neues Foto.
    assert_eq!(report.trashed_local, 0, "{report:?}");
    assert_eq!(report.uploaded, 1, "{report:?}");
    assert_eq!(
        f.local.content("2026/10/bild.jpg").as_deref(),
        Some(&b"bearbeitet"[..])
    );
}

#[tokio::test]
async fn download_only_leaves_local_changes_alone() {
    let f = Fixture::new();
    let mut index = PhotoIndex::new();
    f.library.add_from_phone("a.jpg", b"a", OCTOBER);
    f.run(&mut index, DOWNLOAD).await;
    f.local.remove("2026/10/a.jpg");
    f.local.write("eigene/neu.jpg", b"neu", NOW);

    let report = f.run(&mut index, DOWNLOAD).await;
    assert_eq!(report.ignored, 1, "{report:?}");
    assert_eq!((report.uploaded, report.trashed_remote), (0, 0));
    assert_eq!(f.library.active_ids().len(), 1);
}

#[tokio::test]
async fn two_way_uploads_new_files_and_trashes_deleted_ones() {
    let f = Fixture::new();
    let mut index = PhotoIndex::new();
    let photo = f.library.add_from_phone("handy.jpg", b"handy", OCTOBER);
    f.run(&mut index, TWO_WAY).await;
    f.local.write(
        "Urlaub/strand.JPG",
        b"strand",
        Timestamp::from_unix_seconds(MARCH),
    );
    f.local
        .write("Urlaub/notizen.txt", b"keine Mediendatei", NOW);
    f.local.remove("2026/10/handy.jpg");

    let report = f.run(&mut index, TWO_WAY).await;
    assert_eq!(
        (report.uploaded, report.trashed_remote),
        (1, 1),
        "{report:?}"
    );
    assert!(f.library.is_trashed(&photo.id));
    let uploaded = f
        .library
        .photos()
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.name.as_str() == "strand.JPG")
        .unwrap();
    assert_eq!(uploaded.captured, Timestamp::from_unix_seconds(MARCH));
    assert_eq!(uploaded.media_type, "image/jpeg");
    assert_eq!(f.trash_entries(), 1);

    // Danach ist alles im Gleichgewicht; das hochgeladene Foto wird nicht erneut geladen.
    assert_eq!(f.run(&mut index, TWO_WAY).await, PhotoReport::default());
}

#[tokio::test]
async fn existing_copies_are_linked_instead_of_transferred_again() {
    let f = Fixture::new();
    let mut index = PhotoIndex::new();
    f.library.add_from_phone("IMG_9.jpg", b"schon da", OCTOBER);
    // Dieselbe Datei liegt schon lokal, nur an anderer Stelle.
    f.local.write("Import/kopie.jpg", b"schon da", NOW);

    let report = f.run(&mut index, TWO_WAY).await;
    assert_eq!(report.linked, 1, "{report:?}");
    assert_eq!((report.downloaded, report.uploaded), (0, 0));
    assert_eq!(f.local.content("2026/10/IMG_9.jpg"), None);
    assert_eq!(f.run(&mut index, TWO_WAY).await, PhotoReport::default());
}

#[tokio::test]
async fn upload_only_does_not_upload_photos_that_are_already_there() {
    let f = Fixture::new();
    let mut index = PhotoIndex::new();
    f.library.add_from_phone("doppelt.jpg", b"gleich", OCTOBER);
    f.local.write("doppelt.jpg", b"gleich", NOW);
    f.local.write("neu.mov", b"video", NOW);

    let report = f.run(&mut index, UPLOAD).await;
    assert_eq!((report.linked, report.uploaded), (1, 1), "{report:?}");
    assert_eq!(f.library.active_ids().len(), 2);
}

#[tokio::test]
async fn changes_on_both_sides_keep_both_versions() {
    let f = Fixture::new();
    let mut index = PhotoIndex::new();
    let photo = f.library.add_from_phone("beide.jpg", b"v1", OCTOBER);
    f.run(&mut index, DOWNLOAD).await;
    f.local.write(
        "2026/10/beide.jpg",
        b"lokal",
        Timestamp::from_unix_seconds(OCTOBER + 5),
    );
    f.library.modify(&photo.id, |photo, content| {
        photo.revision = protonsync_domain::node::RevisionId::new("neu");
        *content = b"proton".to_vec();
    });

    let report = f.run(&mut index, DOWNLOAD).await;
    assert_eq!(report.downloaded, 1, "{report:?}");
    assert_eq!(
        f.local.content("2026/10/beide.jpg").as_deref(),
        Some(&b"lokal"[..])
    );
    let copies: Vec<String> = f
        .local
        .paths()
        .into_iter()
        .filter(|path| path.starts_with("2026/10/beide ("))
        .collect();
    assert_eq!(copies.len(), 1, "{:?}", f.local.paths());
    assert_eq!(f.local.content(&copies[0]).as_deref(), Some(&b"proton"[..]));
}

#[tokio::test]
async fn new_revisions_replace_unchanged_copies() {
    let f = Fixture::new();
    let mut index = PhotoIndex::new();
    let photo = f.library.add_from_phone("neu.jpg", b"alt", OCTOBER);
    f.run(&mut index, DOWNLOAD).await;
    f.library.modify(&photo.id, |photo, content| {
        photo.revision = protonsync_domain::node::RevisionId::new("r2");
        *content = b"neu".to_vec();
    });
    assert_eq!(f.run(&mut index, DOWNLOAD).await.downloaded, 1);
    assert_eq!(
        f.local.content("2026/10/neu.jpg").as_deref(),
        Some(&b"neu"[..])
    );
}

#[tokio::test]
async fn local_edits_are_reported_not_uploaded() {
    let f = Fixture::new();
    let mut index = PhotoIndex::new();
    f.library.add_from_phone("edit.jpg", b"alt", OCTOBER);
    f.run(&mut index, TWO_WAY).await;
    f.local.write("2026/10/edit.jpg", b"neu", NOW);
    let report = f.run(&mut index, TWO_WAY).await;
    assert_eq!(report.skipped.len(), 1, "{report:?}");
    assert_eq!(report.uploaded, 0);
}

#[tokio::test]
async fn dry_runs_only_plan() {
    let f = Fixture::new();
    let mut index = PhotoIndex::new();
    f.library.add_from_phone("plan.jpg", b"p", OCTOBER);
    let outcome = f.sync(DOWNLOAD).run(&mut index, true).await.unwrap();
    let PhotoOutcome::Planned(plan) = outcome else {
        panic!("Probelauf erwartet");
    };
    assert_eq!(plan.steps.len(), 1);
    assert!(matches!(plan.steps[0].kind, PhotoStepKind::Download(_)));
    assert_eq!(plan.steps[0].path.as_str(), "2026/10/plan.jpg");
    assert_eq!(f.local.paths(), Vec::<String>::new());
}

#[tokio::test]
async fn failed_downloads_are_retried() {
    let f = Fixture::new();
    let mut index = PhotoIndex::new();
    f.library.add_from_phone("netz.jpg", b"n", OCTOBER);
    index.refresh(&f.library).await.unwrap();
    f.library
        .fail_next_call(RemoteError::Unavailable("Netz".into()));
    // Der Index ist aktuell, also trifft der Fehler das Herunterladen … oder den
    // Ereignisabruf; beides muss beim nächsten Lauf aufgeholt werden.
    let first = f.sync(DOWNLOAD).run(&mut index, false).await;
    match first {
        Ok(PhotoOutcome::Applied(report)) => assert_eq!(report.failures.len(), 1),
        Err(PhotoError::Remote(_)) => {}
        other => panic!("unerwartet: {other:?}"),
    }
    assert_eq!(f.run(&mut index, DOWNLOAD).await.downloaded, 1);
}

#[tokio::test]
async fn deletion_is_verified_before_trashing_locally() {
    let f = Fixture::new();
    let mut index = PhotoIndex::new();
    let photo = f.library.add_from_phone("da.jpg", b"da", OCTOBER);
    f.run(&mut index, DOWNLOAD).await;
    f.library.trash_from_phone(&photo.id);
    index.refresh(&f.library).await.unwrap();
    // Das Foto kommt ohne Ereignis zurück: Der Index hält es für gelöscht.
    f.library.silence_events(true);
    f.library.restore(&photo.id).await.unwrap();
    f.library.silence_events(false);

    let report = f.run(&mut index, DOWNLOAD).await;
    assert_eq!(report.trashed_local, 0, "{report:?}");
    assert_eq!(report.failures.len(), 1, "{report:?}");
    assert_eq!(
        f.local.content("2026/10/da.jpg").as_deref(),
        Some(&b"da"[..])
    );
    // Der nächste Lauf liest die Bibliothek neu und ist zufrieden.
    assert_eq!(f.run(&mut index, DOWNLOAD).await, PhotoReport::default());
}

#[tokio::test]
async fn duplicates_are_suggested_and_trashed_only_on_request() {
    let f = Fixture::new();
    let mut index = PhotoIndex::new();
    let first = f.library.add_from_phone("IMG_1.jpg", b"gleich", OCTOBER);
    let copy = f
        .library
        .add_from_phone("IMG_1 Kopie.jpg", b"gleich", OCTOBER + 9);
    f.library.add_from_phone("anders.jpg", b"anders", OCTOBER);
    f.run(&mut index, DOWNLOAD).await;

    let groups = f.duplicates().find(&mut index).await.unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].reason, DuplicateReason::SameContent);
    assert_eq!(groups[0].keep, first.id);
    assert_eq!(groups[0].others, vec![copy.id.clone()]);

    // Alles löschen oder Fremdes löschen geht nicht.
    let service = f.duplicates();
    let whole = [first.id.clone(), copy.id.clone()];
    assert!(matches!(
        service.trash(&mut index, &whole, false).await,
        Err(PhotoError::Selection(DuplicateSelectionError::WholeGroup(
            _
        )))
    ));
    assert!(matches!(
        service
            .trash(&mut index, &[NodeId::new("fremd")], false)
            .await,
        Err(PhotoError::Selection(
            DuplicateSelectionError::NotADuplicate(_)
        ))
    ));

    // Probelauf ändert nichts.
    let planned = service
        .trash(&mut index, std::slice::from_ref(&copy.id), true)
        .await
        .unwrap();
    assert_eq!(planned[0].path.as_str(), "2026/10/IMG_1 Kopie.jpg");
    assert!(!f.library.is_trashed(&copy.id));

    service
        .trash(&mut index, std::slice::from_ref(&copy.id), false)
        .await
        .unwrap();
    assert!(f.library.is_trashed(&copy.id));
    assert_eq!(f.trash_entries(), 1);

    // Die lokale Kopie folgt beim nächsten Abgleich in den Papierkorb.
    assert_eq!(f.run(&mut index, DOWNLOAD).await.trashed_local, 1);
    assert_eq!(f.trash_entries(), 2);

    // Über den gemeinsamen Papierkorb kommt alles zurück.
    let bin = PhotoBin(&f.library);
    let trash = TrashService {
        remote: &bin,
        local: &f.local,
        state: &f.state,
        clock: &f.clock,
        pair: &photos_owner(),
    };
    for (id, _) in trash.list().unwrap() {
        trash.restore(id).await.unwrap();
    }
    assert!(!f.library.is_trashed(&copy.id));
    assert_eq!(
        f.local.content("2026/10/IMG_1 Kopie.jpg").as_deref(),
        Some(&b"gleich"[..])
    );
}

#[tokio::test]
async fn expired_photo_trash_is_purged_through_the_library() {
    let f = Fixture::new();
    let mut index = PhotoIndex::new();
    let photo = f.library.add_from_phone("alt.jpg", b"alt", OCTOBER);
    f.run(&mut index, TWO_WAY).await;
    f.local.remove("2026/10/alt.jpg");
    f.run(&mut index, TWO_WAY).await;
    f.clock.advance_days(31);
    let bin = PhotoBin(&f.library);
    let trash = TrashService {
        remote: &bin,
        local: &f.local,
        state: &f.state,
        clock: &f.clock,
        pair: &photos_owner(),
    };
    let purged = trash
        .purge_expired(protonsync_domain::trash::RetentionPeriod::default(), false)
        .await
        .unwrap();
    assert_eq!(purged.len(), 1);
    assert_eq!(f.library.content(&photo.id), None);
}

mod index {
    use super::*;

    #[tokio::test]
    async fn first_refresh_reads_everything_then_only_events() {
        let library = FakePhotoLibrary::new();
        let first = library.add_from_phone("a.jpg", b"a", 1);
        let mut index = PhotoIndex::new();
        assert!(!index.is_loaded());
        assert_eq!(index.refresh(&library).await, Ok(IndexRefresh::Full));
        assert_eq!(index.len(), 1);

        assert_eq!(index.refresh(&library).await, Ok(IndexRefresh::Unchanged));
        let second = library.add_from_phone("b.jpg", b"b", 2);
        library.modify(&first.id, |photo, _| photo.favorite = true);
        library.trash_from_phone(&second.id);
        assert_eq!(index.refresh(&library).await, Ok(IndexRefresh::Events(3)));
        assert_eq!(index.len(), 1);
        assert!(index.get(&first.id).is_some_and(|photo| photo.favorite));
        assert_eq!(library.list_calls(), 1);
    }

    #[tokio::test]
    async fn a_gap_or_invalidation_reads_everything_again() {
        let library = FakePhotoLibrary::new();
        let mut index = PhotoIndex::new();
        index.refresh(&library).await.unwrap();
        library.push_gap();
        assert_eq!(index.refresh(&library).await, Ok(IndexRefresh::Full));
        index.invalidate();
        assert_eq!(index.refresh(&library).await, Ok(IndexRefresh::Full));
        assert_eq!(library.list_calls(), 3);
    }

    #[tokio::test]
    async fn failures_keep_the_cursor_so_events_are_retried() {
        let library = FakePhotoLibrary::new();
        let mut index = PhotoIndex::new();
        index.refresh(&library).await.unwrap();
        let photo = library.add_from_phone("c.jpg", b"c", 3);
        library.fail_next_call(RemoteError::Unavailable("Netz".into()));
        assert!(index.refresh(&library).await.is_err());
        assert_eq!(index.refresh(&library).await, Ok(IndexRefresh::Events(1)));
        assert!(index.get(&photo.id).is_some());
    }
}
