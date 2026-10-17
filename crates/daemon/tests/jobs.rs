//! Upload jobs in the daemon core and in the schedule, against fakes.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use protonsync_api::{
    ErrorCode, Event, JobOutcomeInfo, JobVerification, Request, Response, TrashSide,
};
use protonsync_application::ports::{Clock, JobLog, RemoteStorage};
use protonsync_daemon::config::Timing;
use protonsync_daemon::core::{Core, JobAttempt, JobSetup, Settings};
use protonsync_daemon::local_time::LocalZone;
use protonsync_daemon::scheduler;
use protonsync_domain::job::{
    CronSchedule, GlobPattern, JobId, JobOutcome, JobRun, JobTrigger, Retention, UploadJob,
    Verification,
};
use protonsync_domain::node::NodeName;
use protonsync_domain::path::RelativePath;
use protonsync_domain::time::{SECONDS_PER_DAY, Timestamp};
use protonsync_domain::trash::RetentionPeriod;
use protonsync_testkit::{FakeAccount, FakeLocal, FakeRemote, FixedClock, InMemoryStateRepo};
use tokio::sync::watch;

const NOW: Timestamp = Timestamp::from_unix_seconds(1_760_000_000);
const JOB: &str = "ha";

fn days_ago(days: i64) -> Timestamp {
    Timestamp::from_unix_seconds(NOW.unix_seconds() - days * SECONDS_PER_DAY)
}

fn upload_job(trigger: JobTrigger, retention: Retention) -> UploadJob {
    UploadJob {
        id: JobId::parse(JOB).unwrap(),
        source: "/mnt/ha".into(),
        pattern: GlobPattern::parse("*.tar").unwrap(),
        target: RelativePath::parse("Backups").unwrap(),
        trigger,
        retention,
        verification: Verification::Hash,
        max_attempts: 3,
        settle_seconds: 0,
    }
}

type JobCore<C> = Core<FakeAccount, Arc<FakeLocal>, Arc<InMemoryStateRepo>, C>;

struct Fixture<C> {
    core: JobCore<C>,
    remote: Arc<FakeRemote>,
    source: Arc<FakeLocal>,
    state: Arc<InMemoryStateRepo>,
}

fn fixture_with_clock<C: Clock + 'static>(job: UploadJob, clock: C, logged_in: bool) -> Fixture<C> {
    let remote = Arc::new(FakeRemote::new());
    let source = Arc::new(FakeLocal::new());
    let mut account = FakeAccount::new(Arc::clone(&remote));
    if logged_in {
        account = account.with_stored_session();
    }
    let state = Arc::new(InMemoryStateRepo::new());
    let core = Core::new(
        account,
        Arc::clone(&state),
        clock,
        Settings {
            dry_run: false,
            retention: RetentionPeriod::default(),
        },
        Vec::new(),
    )
    .with_jobs(
        vec![JobSetup {
            job,
            source: Arc::clone(&source),
        }],
        LocalZone::utc(),
    );
    Fixture {
        core,
        remote,
        source,
        state,
    }
}

async fn fixture(job: UploadJob) -> Fixture<Arc<FixedClock>> {
    let f = fixture_with_clock(job, Arc::new(FixedClock::at(NOW)), true);
    f.core.startup().await;
    f
}

fn daily() -> JobTrigger {
    JobTrigger::Schedule(CronSchedule::parse("0 3 * * *").unwrap())
}

async fn remote_names(remote: &FakeRemote) -> Vec<String> {
    let root = remote.root_folder().await.unwrap();
    let mut names: Vec<String> = remote
        .list_children(&root.id)
        .await
        .unwrap()
        .into_iter()
        .map(|node| node.name.to_string())
        .collect();
    names.sort();
    names
}

async fn run(core: &JobCore<Arc<FixedClock>>, apply: bool) -> Response {
    core.handle(Request::RunJob {
        job: JOB.into(),
        apply,
    })
    .await
}

#[tokio::test]
async fn manual_run_plans_first_and_then_uploads_with_history() {
    let f = fixture(upload_job(daily(), Retention::keep_all())).await;
    f.source.write("b1.tar", b"backup", days_ago(1));
    f.source.write("note.txt", b"x", days_ago(1));

    let Response::JobRun {
        run: planned,
        details,
    } = run(&f.core, false).await
    else {
        panic!("expected a run");
    };
    assert_eq!(
        planned.outcome,
        JobOutcomeInfo::Planned {
            uploads: 1,
            expired: 0
        }
    );
    assert_eq!(details.uploads, ["b1.tar"]);
    assert!(
        remote_names(&f.remote).await.is_empty(),
        "dry run uploads nothing"
    );

    let Response::JobRun { run: applied, .. } = run(&f.core, true).await else {
        panic!("expected a run");
    };
    assert_eq!(
        applied.outcome,
        JobOutcomeInfo::Succeeded {
            uploaded: 1,
            unchanged: 0,
            expired: 0
        }
    );
    assert_eq!(remote_names(&f.remote).await, ["b1.tar"]);

    let Response::JobHistory { runs } = f
        .core
        .handle(Request::JobHistory {
            job: JOB.into(),
            limit: 10,
        })
        .await
    else {
        panic!("expected the history");
    };
    assert_eq!(runs, [applied.clone(), planned]);

    let Response::Jobs { jobs } = f.core.handle(Request::Jobs).await else {
        panic!("expected jobs");
    };
    let info = &jobs[0];
    assert_eq!(
        (info.id.as_str(), info.schedule.as_str(), info.verify),
        (JOB, "0 3 * * *", JobVerification::Hash)
    );
    assert_eq!(info.last_run, Some(applied));
    // NOW is 2025-10-09 08:53 UTC; next run 2025-10-10 03:00 UTC.
    assert_eq!(info.next_run, Some(1_760_065_200));
    assert!(!info.running);
}

#[tokio::test]
async fn unknown_job_and_missing_login_are_errors() {
    let f = fixture(upload_job(daily(), Retention::keep_all())).await;
    let Response::Error(error) = f
        .core
        .handle(Request::RunJob {
            job: "missing".into(),
            apply: true,
        })
        .await
    else {
        panic!("expected an error");
    };
    assert_eq!(error.code, ErrorCode::NotFound);

    let f = fixture_with_clock(
        upload_job(daily(), Retention::keep_all()),
        Arc::new(FixedClock::at(NOW)),
        false,
    );
    let Response::Error(error) = run(&f.core, true).await else {
        panic!("expected an error");
    };
    assert_eq!(error.code, ErrorCode::NotLoggedIn);
    assert_eq!(f.core.scheduled_job(0, 1, false).await, JobAttempt::NotRun);
}

#[tokio::test]
async fn retention_goes_to_the_trash_and_can_be_restored() {
    let f = fixture(upload_job(daily(), Retention::new(Some(1), None).unwrap())).await;
    f.source.write("old.tar", b"1", days_ago(2));
    f.source.write("recent.tar", b"2", days_ago(1));
    let Response::JobRun { details, .. } = run(&f.core, true).await else {
        panic!("expected a run");
    };
    assert_eq!(details.expired, ["old.tar"]);
    assert_eq!(remote_names(&f.remote).await, ["recent.tar"]);

    let Response::Trash { entries } = f.core.handle(Request::Trash).await else {
        panic!("expected the trash");
    };
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(
        (entry.pair.as_str(), entry.path.as_str(), entry.side),
        ("job:ha", "old.tar", TrashSide::Remote)
    );
    let restored = f
        .core
        .handle(Request::RestoreTrash {
            pair: entry.pair.clone(),
            id: entry.id,
        })
        .await;
    assert_eq!(restored, Response::Done);
    assert_eq!(remote_names(&f.remote).await, ["old.tar", "recent.tar"]);
}

#[tokio::test]
async fn new_file_jobs_only_run_when_the_source_changed() {
    let f = fixture(upload_job(JobTrigger::OnNewFile, Retention::keep_all())).await;
    f.source.write("a.tar", b"1", days_ago(1));
    assert_eq!(
        f.core.scheduled_job(0, 1, true).await,
        JobAttempt::Done { waiting: false }
    );
    assert_eq!(f.core.scheduled_job(0, 1, true).await, JobAttempt::NotRun);
    // Other files do not count.
    f.source.write("note.txt", b"x", days_ago(1));
    assert_eq!(f.core.scheduled_job(0, 1, true).await, JobAttempt::NotRun);

    f.source.write("b.tar", b"2", NOW);
    assert_eq!(
        f.core.scheduled_job(0, 1, true).await,
        JobAttempt::Done { waiting: false }
    );
    assert_eq!(remote_names(&f.remote).await, ["a.tar", "b.tar"]);
}

#[tokio::test]
async fn files_in_their_settle_time_are_reported_as_waiting() {
    let mut job = upload_job(JobTrigger::OnNewFile, Retention::keep_all());
    job.settle_seconds = 60;
    let f = fixture(job).await;
    f.source.write("fresh.tar", b"1", NOW);
    assert_eq!(
        f.core.scheduled_job(0, 1, true).await,
        JobAttempt::Done { waiting: true }
    );
    assert_eq!(remote_names(&f.remote).await, Vec::<String>::new());
}

#[tokio::test]
async fn last_failed_attempt_notifies_the_user() {
    let f = fixture(upload_job(JobTrigger::OnNewFile, Retention::keep_all())).await;
    f.source.write("a.tar", b"1", days_ago(1));
    // A folder with the same name in the target makes the run fail.
    let root = f.remote.root_folder().await.unwrap();
    f.remote
        .create_folder(&root.id, &NodeName::new("a.tar").unwrap())
        .await
        .unwrap();
    let mut events = f.core.subscribe();

    let JobAttempt::Failed { .. } = f.core.scheduled_job(0, 1, true).await else {
        panic!("expected a failure");
    };
    // After a failure the state does not count as done.
    let JobAttempt::Failed { message } = f.core.scheduled_job(0, 3, true).await else {
        panic!("expected a failure");
    };
    let mut gave_up = None;
    while let Ok(event) = events.try_recv() {
        if let Event::JobGaveUp { attempts, .. } = event {
            gave_up = Some(attempts);
        }
    }
    assert_eq!(gave_up, Some(3), "{message}");
    // After giving up, only new files trigger a run again.
    assert_eq!(f.core.scheduled_job(0, 1, true).await, JobAttempt::NotRun);
}

/// Clock that runs with the Tokio time (paused in tests).
struct TokioClock {
    start: tokio::time::Instant,
}

impl Clock for TokioClock {
    fn now(&self) -> Timestamp {
        let elapsed = i64::try_from(self.start.elapsed().as_secs()).unwrap();
        Timestamp::from_unix_seconds(NOW.unix_seconds() + elapsed)
    }
}

#[tokio::test(start_paused = true)]
async fn scheduler_runs_cron_jobs_at_their_time() {
    // Every full hour; NOW is 08:53:20 UTC, so after a bit more than six minutes.
    let job = upload_job(
        JobTrigger::Schedule(CronSchedule::parse("0 * * * *").unwrap()),
        Retention::keep_all(),
    );
    let clock = TokioClock {
        start: tokio::time::Instant::now(),
    };
    let f = fixture_with_clock(job, clock, true);
    f.core.startup().await;
    f.source.write("a.tar", b"1", days_ago(1));
    let core = Arc::new(f.core);
    let (stop, shutdown) = watch::channel(false);
    let timing = Timing {
        event_poll: Duration::from_secs(60),
        full_sync: Duration::from_secs(3600),
        local_debounce: Duration::from_secs(3),
    };
    let running = tokio::spawn(scheduler::run(Arc::clone(&core), timing, shutdown));

    tokio::time::sleep(Duration::from_mins(5)).await;
    assert_eq!(
        remote_names(&f.remote).await,
        Vec::<String>::new(),
        "not due yet"
    );
    tokio::time::sleep(Duration::from_mins(2)).await;
    assert_eq!(remote_names(&f.remote).await, ["a.tar"]);

    stop.send(true).unwrap();
    running.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn scheduler_catches_up_a_missed_run_at_start() {
    // Daily at 3 o'clock; the last run was two days ago, so one was missed.
    let clock = TokioClock {
        start: tokio::time::Instant::now(),
    };
    let f = fixture_with_clock(upload_job(daily(), Retention::keep_all()), clock, true);
    f.core.startup().await;
    f.source.write("a.tar", b"1", days_ago(1));
    let core = Arc::new(f.core);
    assert!(
        !core.missed_job_run(0),
        "nothing to catch up without a previous run"
    );
    let earlier = JobRun {
        started_at: days_ago(2),
        finished_at: days_ago(2),
        outcome: JobOutcome::Succeeded {
            uploaded: 0,
            unchanged: 0,
            expired: 0,
        },
    };
    f.state
        .record_job_run(&JobId::parse(JOB).unwrap(), &earlier)
        .unwrap();
    assert!(core.missed_job_run(0));
    let (stop, shutdown) = watch::channel(false);
    let timing = Timing {
        event_poll: Duration::from_secs(60),
        full_sync: Duration::from_secs(3600),
        local_debounce: Duration::from_secs(3),
    };
    let running = tokio::spawn(scheduler::run(Arc::clone(&core), timing, shutdown));

    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(remote_names(&f.remote).await, ["a.tar"]);
    assert!(!core.missed_job_run(0), "the new run counts as done");

    stop.send(true).unwrap();
    running.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn scheduler_runs_new_file_jobs_at_start_and_after_changes() {
    let job = upload_job(JobTrigger::OnNewFile, Retention::keep_all());
    let clock = TokioClock {
        start: tokio::time::Instant::now(),
    };
    let f = fixture_with_clock(job, clock, true);
    f.core.startup().await;
    f.source.write("a.tar", b"1", days_ago(1));
    let core = Arc::new(f.core);
    let (stop, shutdown) = watch::channel(false);
    let timing = Timing {
        event_poll: Duration::from_secs(60),
        full_sync: Duration::from_secs(3600),
        local_debounce: Duration::from_secs(3),
    };
    let running = tokio::spawn(scheduler::run(Arc::clone(&core), timing, shutdown));

    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(remote_names(&f.remote).await, ["a.tar"], "at startup");

    f.source.write("b.tar", b"2", days_ago(1));
    core.job_trigger(0).unwrap().notify_one();
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(
        remote_names(&f.remote).await,
        ["a.tar", "b.tar"],
        "after a signal"
    );

    // Without a signal (e.g. network drive) after five minutes at the latest.
    f.source.write("c.tar", b"3", days_ago(1));
    tokio::time::sleep(Duration::from_secs(5 * 60 + 1)).await;
    assert_eq!(remote_names(&f.remote).await, ["a.tar", "b.tar", "c.tar"]);

    stop.send(true).unwrap();
    running.await.unwrap();
}
