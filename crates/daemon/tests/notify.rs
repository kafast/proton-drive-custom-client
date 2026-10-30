//! Notifications: what is reported, batching, switching topics off, retries,
//! the weekly report and the event loop with paused time.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use protonsync_api::{
    ConflictInfo, ConflictKindInfo, Event, JobOutcomeInfo, JobRunInfo, PairStatus, PairSyncResult,
    Request, Response, StatusInfo, SyncOutcomeInfo, SyncSummary,
};
use protonsync_client::Lang;
use protonsync_daemon::local_time::LocalZone;
use protonsync_daemon::notify::{
    Dispatcher, Notifications, NotifyLoop, Outgoing, ReportSnapshot, Route, Topic, report_snapshot,
    run,
};
use protonsync_daemon::notify_config::Topics;
use protonsync_domain::job::CronSchedule;
use protonsync_domain::time::Timestamp;
use protonsync_testkit::{FakeNotifier, FixedClock};
use tokio::sync::{broadcast, watch};

/// Sunday, 2026-10-18 00:00 UTC.
const SUNDAY: Timestamp = Timestamp::from_unix_seconds(1_792_281_600);

const ALL: Topics = Topics {
    conflicts: true,
    sync_failures: true,
    job_failures: true,
    login_required: true,
    weekly_report: true,
};

fn notifications(lang: Lang) -> Notifications {
    Notifications::new(lang, LocalZone::utc(), SUNDAY)
}

fn conflict(path: &str, kind: ConflictKindInfo) -> ConflictInfo {
    ConflictInfo {
        pair: "main".into(),
        id: 1,
        path: path.into(),
        kind,
        conflict_copy: Some(format!("{path} (Konflikt 1)")),
        detected_at: SUNDAY.unix_seconds(),
    }
}

fn sync(pair: &str, outcome: SyncOutcomeInfo) -> Event {
    Event::SyncFinished(PairSyncResult {
        pair: pair.into(),
        finished_at: SUNDAY.unix_seconds(),
        outcome,
    })
}

fn failed(message: &str) -> SyncOutcomeInfo {
    SyncOutcomeInfo::Failed {
        message: message.into(),
    }
}

fn applied(uploaded: usize, downloaded: usize) -> SyncOutcomeInfo {
    SyncOutcomeInfo::Applied(SyncSummary {
        uploaded,
        downloaded,
        trashed_remote: 0,
        trashed_local: 1,
        archived: 0,
        conflicts: 2,
        ignored: 0,
        skipped: Vec::new(),
        failures: Vec::new(),
    })
}

#[test]
fn conflicts_are_collected_into_one_message() {
    let mut n = notifications(Lang::De);
    let first = Event::ConflictDetected(conflict("a.txt", ConflictKindInfo::BothModified));
    let second = Event::ConflictDetected(conflict(
        "b.txt",
        ConflictKindInfo::LocalModifiedRemoteProtected,
    ));
    assert_eq!(n.on_event(&first), None);
    assert_eq!(n.on_event(&second), None);
    assert!(n.has_pending_conflicts());

    let outgoing = n.take_conflicts().unwrap();
    assert_eq!(outgoing.topic, Topic::Conflicts);
    assert_eq!(outgoing.message.subject, "protonsync: 2 neue Konflikte");
    assert!(
        outgoing
            .message
            .body
            .contains("main: a.txt (beide Seiten geändert)")
    );
    assert!(
        outgoing
            .message
            .body
            .contains("b.txt (lokal geändert, Proton-Version geschützt)")
    );
    assert!(n.take_conflicts().is_none());
}

#[test]
fn a_failing_pair_is_reported_once_until_it_works_again() {
    let mut n = notifications(Lang::En);
    let report = n.on_event(&sync("main", failed("network down"))).unwrap();
    assert_eq!(report.topic, Topic::SyncFailure);
    assert_eq!(report.message.subject, "protonsync: Sync failed: main");
    assert!(report.message.body.contains("network down"));
    assert_eq!(n.on_event(&sync("main", failed("network down"))), None);
    // Another pair is reported on its own.
    assert!(n.on_event(&sync("docs", failed("disk full"))).is_some());

    n.on_event(&sync("main", applied(0, 0)));
    assert!(n.on_event(&sync("main", failed("again"))).is_some());
}

#[test]
fn an_expired_login_is_reported_once_and_silences_sync_failures() {
    let mut n = notifications(Lang::De);
    let login = Event::LoginRequired {
        message: "session expired".into(),
    };
    let report = n.on_event(&login).unwrap();
    assert_eq!(report.topic, Topic::LoginRequired);
    assert!(report.message.body.contains("protonsync login"));
    assert_eq!(n.on_event(&login), None);
    assert_eq!(n.on_event(&sync("main", failed("not logged in"))), None);

    n.on_event(&sync("main", applied(0, 0)));
    assert!(n.on_event(&login).is_some());
}

#[test]
fn a_job_that_gave_up_is_reported() {
    let mut n = notifications(Lang::En);
    let report = n
        .on_event(&Event::JobGaveUp {
            job: "home-assistant".into(),
            message: "source missing".into(),
            attempts: 3,
        })
        .unwrap();
    assert_eq!(report.topic, Topic::JobFailure);
    assert!(report.message.subject.ends_with("home-assistant"));
    assert!(report.message.body.contains("Attempts: 3"));
}

#[test]
fn weekly_report_sums_up_the_period_and_restarts_the_counters() {
    let mut n = notifications(Lang::De);
    n.on_event(&sync("main", applied(3, 4)));
    n.on_event(&sync("main", applied(2, 0)));
    n.on_event(&Event::JobFinished(JobRunInfo {
        job: "ha".into(),
        started_at: 0,
        finished_at: 0,
        outcome: JobOutcomeInfo::Succeeded {
            uploaded: 1,
            unchanged: 0,
            expired: 0,
        },
    }));
    let snapshot = ReportSnapshot {
        account: None,
        dry_run: true,
        last_syncs: vec![("main".into(), None)],
        conflicts: vec![conflict("a.txt", ConflictKindInfo::BothModified)],
        ..ReportSnapshot::default()
    };
    let week = Timestamp::from_unix_seconds(SUNDAY.unix_seconds() + 7 * 86_400);

    let report = n.weekly_report(&snapshot, week);

    assert_eq!(report.topic, Topic::WeeklyReport);
    assert_eq!(report.message.subject, "protonsync: Wochenbericht");
    let body = &report.message.body;
    assert!(
        body.contains("Zeitraum: 2026-10-18 00:00 bis 2026-10-25 00:00"),
        "{body}"
    );
    assert!(body.contains("Nicht angemeldet"), "{body}");
    assert!(body.contains("Probelauf aktiv"), "{body}");
    assert!(
        body.contains(
            "- main: 5 hochgeladen, 4 heruntergeladen, 2 in den Papierkorb, 4 Konflikte, 0 Fehler"
        ),
        "{body}"
    );
    assert!(body.contains("Offene Konflikte: 1"), "{body}");

    let next = n.weekly_report(&snapshot, week);
    assert!(
        next.message.body.contains("- main: 0 hochgeladen"),
        "{}",
        next.message.body
    );
}

fn outgoing(topic: Topic) -> Outgoing {
    Outgoing {
        topic,
        message: protonsync_application::ports::Message {
            subject: "s".into(),
            body: "b".into(),
        },
    }
}

#[tokio::test]
async fn channels_only_get_the_topics_they_want_and_retry_outages() {
    let mail = Arc::new(FakeNotifier::new());
    let webhook = Arc::new(FakeNotifier::new());
    let dispatcher = Dispatcher::with_retry_delays(
        vec![
            Route {
                notifier: Arc::clone(&mail),
                topics: ALL,
            },
            Route {
                notifier: Arc::clone(&webhook),
                topics: Topics {
                    weekly_report: false,
                    ..ALL
                },
            },
        ],
        vec![Duration::ZERO, Duration::ZERO],
    );
    assert!(dispatcher.wants(Topic::WeeklyReport));

    dispatcher.deliver(&outgoing(Topic::WeeklyReport)).await;
    assert_eq!((mail.sent().len(), webhook.sent().len()), (1, 0));

    // Two outages are survived, a third attempt is the last one.
    mail.fail_next(2);
    dispatcher.deliver(&outgoing(Topic::Conflicts)).await;
    assert_eq!(mail.sent().len(), 2);
    mail.fail_next(3);
    dispatcher.deliver(&outgoing(Topic::Conflicts)).await;
    assert_eq!(mail.sent().len(), 2);
    assert_eq!(webhook.sent().len(), 2);
}

/// Answers like a daemon with one sync pair and nothing else.
struct ScriptedDaemon;

#[allow(clippy::unused_async_trait_impl)]
impl protonsync_web::Backend for ScriptedDaemon {
    async fn handle(&self, request: Request) -> Response {
        match request {
            Request::Status => Response::Status(StatusInfo {
                daemon_version: "test".into(),
                account: Some("krischan@proton.me".into()),
                login_pending: false,
                dry_run: false,
                pairs: vec![PairStatus {
                    id: "main".into(),
                    local_root: "/home/k/Proton".into(),
                    remote_path: "protonsync".into(),
                    direction: protonsync_api::Direction::TwoWay,
                    syncing: false,
                    last_sync: None,
                    open_conflicts: 0,
                    trash_entries: 0,
                }],
            }),
            Request::Conflicts => Response::Conflicts {
                conflicts: Vec::new(),
            },
            Request::Trash => Response::Trash {
                entries: Vec::new(),
            },
            Request::Jobs => Response::Jobs { jobs: Vec::new() },
            _ => Response::Done,
        }
    }
}

#[tokio::test]
async fn snapshot_comes_from_the_daemon_api() {
    let snapshot = report_snapshot(&ScriptedDaemon).await;
    assert_eq!(snapshot.account.as_deref(), Some("krischan@proton.me"));
    assert_eq!(snapshot.last_syncs, vec![("main".to_owned(), None)]);
}

#[tokio::test(start_paused = true)]
async fn the_loop_batches_conflicts_and_sends_the_weekly_report() {
    let notifier = Arc::new(FakeNotifier::new());
    let dispatcher = Arc::new(Dispatcher::new(vec![Route {
        notifier: Arc::clone(&notifier),
        topics: ALL,
    }]));
    let (events, receiver) = broadcast::channel(16);
    let (stop, shutdown) = watch::channel(false);
    let task = tokio::spawn(run(
        NotifyLoop {
            backend: Arc::new(ScriptedDaemon),
            clock: Arc::new(FixedClock::at(SUNDAY)),
            zone: LocalZone::utc(),
            // Monday 8:00, i.e. 32 hours after the fixed clock.
            report_schedule: CronSchedule::parse("0 8 * * 1").unwrap(),
            notifications: notifications(Lang::En),
        },
        receiver,
        dispatcher,
        shutdown,
    ));

    for path in ["a.txt", "b.txt"] {
        events
            .send(Event::ConflictDetected(conflict(
                path,
                ConflictKindInfo::BothModified,
            )))
            .unwrap();
    }
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert!(notifier.sent().is_empty(), "still collecting");
    tokio::time::sleep(Duration::from_secs(40)).await;
    let sent = notifier.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].subject, "protonsync: 2 new conflicts");

    tokio::time::sleep(Duration::from_hours(32)).await;
    let sent = notifier.sent();
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert_eq!(sent[1].subject, "protonsync: Weekly report");
    assert!(sent[1].body.contains("Logged in: krischan@proton.me"));

    stop.send(true).unwrap();
    task.await.unwrap();
}
