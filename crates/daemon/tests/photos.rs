//! Photo sync and duplicates in the daemon core, against fakes.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use protonsync_api::{
    Direction, ErrorCode, Event, PHOTOS_TRASH_OWNER, PhotoOutcomeInfo, PhotoRunInfo, Request,
    Response, TrashSide,
};
use protonsync_application::photos::PHOTOS_OWNER;
use protonsync_daemon::config::Timing;
use protonsync_daemon::core::{Core, PhotoSetup, Settings};
use protonsync_daemon::local_time::LocalZone;
use protonsync_daemon::photo_config::PhotoConfig;
use protonsync_daemon::scheduler;
use protonsync_domain::sync_pair::SyncDirection;
use protonsync_domain::time::Timestamp;
use protonsync_domain::trash::RetentionPeriod;
use protonsync_testkit::{
    FakeAccount, FakeLocal, FakePhotoLibrary, FakeRemote, FixedClock, InMemoryStateRepo,
};
use tokio::sync::watch;

/// 2025-10-09 08:53 UTC.
const NOW: i64 = 1_760_000_000;
const FOLDER: &str = "/home/k/Pictures/Proton";

type PhotoCore = Core<FakeAccount, Arc<FakeLocal>, Arc<InMemoryStateRepo>, Arc<FixedClock>>;

struct Fixture {
    core: PhotoCore,
    library: Arc<FakePhotoLibrary>,
    folder: Arc<FakeLocal>,
    clock: Arc<FixedClock>,
}

fn fixture_with(dry_run: bool, logged_in: bool, photos: bool) -> Fixture {
    let mut account = FakeAccount::new(Arc::new(FakeRemote::new()));
    if logged_in {
        account = account.with_stored_session();
    }
    let library = account.photos();
    let folder = Arc::new(FakeLocal::new());
    let clock = Arc::new(FixedClock::at(Timestamp::from_unix_seconds(NOW)));
    let mut core = Core::new(
        account,
        Arc::new(InMemoryStateRepo::new()),
        Arc::clone(&clock),
        Settings {
            dry_run,
            retention: RetentionPeriod::default(),
        },
        Vec::new(),
    )
    .with_jobs(Vec::new(), LocalZone::utc());
    if photos {
        core = core.with_photos(PhotoSetup {
            config: PhotoConfig {
                folder: FOLDER.into(),
                direction: SyncDirection::DownloadOnly,
                protect_remote: false,
            },
            local: Arc::clone(&folder),
        });
    }
    Fixture {
        core,
        library,
        folder,
        clock,
    }
}

async fn fixture() -> Fixture {
    let f = fixture_with(false, true, true);
    f.core.startup().await;
    f
}

async fn sync(core: &PhotoCore, apply: bool) -> PhotoRunInfo {
    match core.handle(Request::SyncPhotos { apply }).await {
        Response::PhotoRun(run) => run,
        other => panic!("expected a run: {other:?}"),
    }
}

fn error_code(response: Response) -> ErrorCode {
    match response {
        Response::Error(error) => error.code,
        other => panic!("expected an error: {other:?}"),
    }
}

#[test]
fn trash_owner_names_match() {
    assert_eq!(PHOTOS_TRASH_OWNER, PHOTOS_OWNER);
}

#[tokio::test]
async fn photo_requests_need_the_photos_section_and_a_login() {
    let off = fixture_with(false, true, false);
    off.core.startup().await;
    assert_eq!(
        error_code(off.core.handle(Request::Photos).await),
        ErrorCode::NotFound
    );
    assert_eq!(off.core.photo_trigger().map(|_| ()), None);

    let logged_out = fixture_with(false, false, true);
    logged_out.core.startup().await;
    let Response::Photos(status) = logged_out.core.handle(Request::Photos).await else {
        panic!("expected a status");
    };
    assert_eq!(status.folder, FOLDER);
    assert_eq!(status.direction, Direction::DownloadOnly);
    assert_eq!((status.photos, status.last_run), (None, None));
    assert_eq!(
        error_code(
            logged_out
                .core
                .handle(Request::SyncPhotos { apply: true })
                .await
        ),
        ErrorCode::NotLoggedIn
    );
    assert_eq!(logged_out.core.scheduled_photo_sync(false).await, None);
}

#[tokio::test]
async fn manual_sync_plans_first_and_then_downloads_into_month_folders() {
    let f = fixture().await;
    f.library.add_from_phone("IMG_1.jpg", b"one", NOW);
    let mut events = f.core.subscribe();

    let planned = sync(&f.core, false).await;
    let PhotoOutcomeInfo::Planned(counts) = &planned.outcome else {
        panic!("expected a dry run: {planned:?}");
    };
    assert_eq!(counts.downloaded, 1);
    assert_eq!(f.folder.content("2025/10/IMG_1.jpg"), None, "dry run");

    let applied = sync(&f.core, true).await;
    let PhotoOutcomeInfo::Applied(counts) = &applied.outcome else {
        panic!("expected an applied run: {applied:?}");
    };
    assert_eq!(counts.downloaded, 1);
    assert_eq!(
        f.folder.content("2025/10/IMG_1.jpg").as_deref(),
        Some(&b"one"[..])
    );

    let Response::Photos(status) = f.core.handle(Request::Photos).await else {
        panic!("expected a status");
    };
    assert_eq!(status.photos, Some(1));
    assert_eq!(status.last_run, Some(applied.clone()));
    assert_eq!(events.recv().await.unwrap(), Event::PhotoSyncStarted);
    assert_eq!(
        events.recv().await.unwrap(),
        Event::PhotoSyncFinished(planned)
    );
}

#[tokio::test]
async fn scheduled_sync_follows_dry_run_and_events() {
    let f = fixture_with(true, true, true);
    f.core.startup().await;
    assert!(f.core.photos_changed().await, "first read");
    assert!(!f.core.photos_changed().await, "nothing new");

    f.library.add_from_phone("IMG_2.jpg", b"two", NOW);
    assert!(f.core.photos_changed().await, "new photo from the phone");
    let run = f.core.scheduled_photo_sync(false).await.unwrap();
    assert!(
        matches!(run.outcome, PhotoOutcomeInfo::Planned(ref c) if c.downloaded == 1),
        "{run:?}"
    );
    assert_eq!(f.folder.paths(), Vec::<String>::new(), "dry_run only plans");
}

#[tokio::test]
async fn duplicates_are_only_suggested_and_trashed_on_request() {
    let f = fixture().await;
    let keep = f.library.add_from_phone("IMG_3.jpg", b"same", NOW);
    let copy = f.library.add_from_phone("IMG_3.jpg", b"same", NOW);
    sync(&f.core, true).await;

    let Response::PhotoDuplicates { groups } = f.core.handle(Request::PhotoDuplicates).await else {
        panic!("expected groups");
    };
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].keep.id, keep.id.as_str());
    assert_eq!(groups[0].others[0].id, copy.id.as_str());

    let whole_group = Request::TrashPhotoDuplicates {
        ids: vec![keep.id.to_string(), copy.id.to_string()],
        apply: true,
    };
    assert_eq!(
        error_code(f.core.handle(whole_group).await),
        ErrorCode::InvalidRequest
    );

    let trash = |apply| Request::TrashPhotoDuplicates {
        ids: vec![copy.id.to_string()],
        apply,
    };
    let Response::PhotosTrashed { dry_run, photos } = f.core.handle(trash(false)).await else {
        panic!("expected a result");
    };
    assert!(dry_run);
    assert_eq!(photos.len(), 1);
    assert!(!f.library.is_trashed(&copy.id), "dry run deletes nothing");

    f.core.handle(trash(true)).await;
    assert!(f.library.is_trashed(&copy.id));
    let Response::Trash { entries } = f.core.handle(Request::Trash).await else {
        panic!("expected the trash");
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].pair, PHOTOS_TRASH_OWNER);
    assert_eq!(entries[0].side, TrashSide::Remote);

    let restore = Request::RestoreTrash {
        pair: PHOTOS_TRASH_OWNER.into(),
        id: entries[0].id,
    };
    assert_eq!(f.core.handle(restore).await, Response::Done);
    assert!(!f.library.is_trashed(&copy.id), "restored");
}

#[tokio::test]
async fn expired_photo_trash_is_purged_only_with_apply() {
    let f = fixture().await;
    f.library.add_from_phone("IMG_4.jpg", b"a", NOW);
    let copy = f.library.add_from_phone("IMG_4.jpg", b"a", NOW);
    f.core
        .handle(Request::TrashPhotoDuplicates {
            ids: vec![copy.id.to_string()],
            apply: true,
        })
        .await;
    f.clock.advance_days(31);

    let Response::Trash { entries } = f.core.handle(Request::PurgeTrash { apply: false }).await
    else {
        panic!("expected a list");
    };
    assert_eq!(entries.len(), 1);
    assert!(f.library.is_trashed(&copy.id), "only listed");

    f.core.handle(Request::PurgeTrash { apply: true }).await;
    assert!(!f.library.is_trashed(&copy.id));
    assert!(!f.library.active_ids().contains(&copy.id), "gone for good");
}

#[tokio::test]
async fn logout_forgets_the_library() {
    let f = fixture().await;
    sync(&f.core, true).await;
    assert_eq!(f.core.handle(Request::Logout).await, Response::Done);
    assert_eq!(
        error_code(f.core.handle(Request::PhotoDuplicates).await),
        ErrorCode::NotLoggedIn
    );
    let Response::Photos(status) = f.core.handle(Request::Photos).await else {
        panic!("expected a status");
    };
    assert_eq!(status.photos, None, "index forgotten");
}

#[tokio::test(start_paused = true)]
async fn scheduler_syncs_photos_at_start_and_after_events() {
    let f = fixture().await;
    f.library.add_from_phone("IMG_5.jpg", b"5", NOW);
    let core = Arc::new(f.core);
    let (stop, shutdown) = watch::channel(false);
    let timing = Timing {
        event_poll: Duration::from_secs(60),
        full_sync: Duration::from_secs(3600),
        local_debounce: Duration::from_secs(3),
    };
    let running = tokio::spawn(scheduler::run(Arc::clone(&core), timing, shutdown));

    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        f.folder.paths(),
        ["2025", "2025/10", "2025/10/IMG_5.jpg"],
        "at startup"
    );

    f.library.add_from_phone("IMG_6.jpg", b"6", NOW);
    tokio::time::sleep(Duration::from_secs(61)).await;
    assert!(
        f.folder.content("2025/10/IMG_6.jpg").is_some(),
        "after the next event: {:?}",
        f.folder.paths()
    );

    stop.send(true).unwrap();
    running.await.unwrap();
}
