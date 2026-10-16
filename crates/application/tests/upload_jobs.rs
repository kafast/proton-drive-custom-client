//! Upload-Jobs gegen die Fakes: Muster, Ruhezeit, Prüfung, Aufbewahrung, Probelauf.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::FailingOnUpload;
use protonsync_application::ports::{RemoteStorage, StateRepo};
use protonsync_application::use_cases::{JobReport, JobResult, TrashService, UploadJobRunner};
use protonsync_domain::job::{GlobPattern, JobId, JobTrigger, Retention, UploadJob, Verification};
use protonsync_domain::node::NodeName;
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::time::{SECONDS_PER_DAY, Timestamp};
use protonsync_testkit::{FakeLocal, FakeRemote, FixedClock, InMemoryStateRepo};

const NOW: Timestamp = Timestamp::from_unix_seconds(1_760_000_000);
const SETTLE_SECONDS: u32 = 60;

fn days_ago(days: i64) -> Timestamp {
    Timestamp::from_unix_seconds(NOW.unix_seconds() - days * SECONDS_PER_DAY)
}

fn job(retention: Retention) -> UploadJob {
    UploadJob {
        id: JobId::parse("ha").unwrap(),
        source: "/backup".into(),
        pattern: GlobPattern::parse("*.tar").unwrap(),
        target: RelativePath::parse("Backups").unwrap(),
        trigger: JobTrigger::OnNewFile,
        retention,
        verification: Verification::Hash,
        max_attempts: 3,
        settle_seconds: SETTLE_SECONDS,
    }
}

struct Setup {
    remote: FakeRemote,
    source: FakeLocal,
    state: InMemoryStateRepo,
    clock: FixedClock,
    job: UploadJob,
    owner: SyncPairId,
}

impl Setup {
    fn new(retention: Retention) -> Self {
        Self {
            remote: FakeRemote::new(),
            source: FakeLocal::new(),
            state: InMemoryStateRepo::new(),
            clock: FixedClock::at(NOW),
            job: job(retention),
            owner: SyncPairId::new("job:ha"),
        }
    }

    async fn run_on<R: RemoteStorage>(&self, remote: &R, dry_run: bool) -> JobResult {
        UploadJobRunner {
            remote,
            source: &self.source,
            state: &self.state,
            clock: &self.clock,
            job: &self.job,
            trash_owner: &self.owner,
        }
        .run(dry_run)
        .await
        .unwrap()
    }

    async fn apply(&self) -> JobReport {
        match self.run_on(&self.remote, false).await {
            JobResult::Applied(report) => report,
            JobResult::Planned(_) => unreachable!("kein Probelauf"),
        }
    }

    async fn remote_names(&self) -> Vec<String> {
        let root = self.remote.root_folder().await.unwrap();
        self.remote
            .list_children(&root.id)
            .await
            .unwrap()
            .into_iter()
            .map(|node| node.name.to_string())
            .collect()
    }
}

fn names(list: &[NodeName]) -> Vec<&str> {
    list.iter().map(NodeName::as_str).collect()
}

#[tokio::test]
async fn uploads_settled_matching_top_level_files_only() {
    let s = Setup::new(Retention::keep_all());
    s.source.write("backup-1.tar", b"eins", days_ago(1));
    s.source.write("notizen.txt", b"nein", days_ago(1));
    s.source
        .write("alt/backup-0.tar", b"unterordner", days_ago(1));
    s.source.write("frisch.tar", b"wird noch geschrieben", NOW);

    let report = s.apply().await;
    assert_eq!(names(&report.uploaded), ["backup-1.tar"]);
    assert_eq!(names(&report.waiting), ["frisch.tar"]);
    assert!(report.succeeded());
    assert_eq!(s.remote_names().await, ["backup-1.tar"]);

    // Ein zweiter Lauf erkennt die Datei als unverändert.
    let report = s.apply().await;
    assert_eq!((report.uploaded.len(), report.unchanged), (0, 1));
}

#[tokio::test]
async fn changed_content_becomes_a_new_revision() {
    let s = Setup::new(Retention::keep_all());
    s.source.write("backup.tar", b"alt!", days_ago(2));
    s.apply().await;
    s.source.write("backup.tar", b"neu!", days_ago(1));

    let JobResult::Planned(plan) = s.run_on(&s.remote, true).await else {
        panic!("Probelauf erwartet");
    };
    assert_eq!(plan.uploads.len(), 1);
    assert!(plan.uploads[0].replaces, "{plan:?}");

    let report = s.apply().await;
    assert_eq!(names(&report.uploaded), ["backup.tar"]);
    let root = s.remote.root_folder().await.unwrap();
    let node = &s.remote.list_children(&root.id).await.unwrap()[0];
    let mut content = Vec::new();
    s.remote.download_to(&node.id, &mut content).await.unwrap();
    assert_eq!(content, b"neu!");
}

#[tokio::test]
async fn retention_trashes_old_files_restorably() {
    let s = Setup::new(Retention::new(Some(2), None).unwrap());
    for day in 1..=4 {
        s.source
            .write(&format!("backup-{day}.tar"), b"x", days_ago(10 - day));
    }
    let report = s.apply().await;
    assert_eq!(report.uploaded.len(), 4);
    assert_eq!(names(&report.expired), ["backup-1.tar", "backup-2.tar"]);
    assert_eq!(s.remote_names().await, ["backup-3.tar", "backup-4.tar"]);

    let entries = s.state.trash_entries(&s.owner).unwrap();
    assert_eq!(entries.len(), 2);
    let service = TrashService {
        remote: &s.remote,
        local: &s.source,
        state: &s.state,
        clock: &s.clock,
        pair: &s.owner,
    };
    service.restore(entries[0].0).await.unwrap();
    assert_eq!(s.remote_names().await.len(), 3);
}

#[tokio::test]
async fn dry_run_changes_nothing_but_shows_everything() {
    let s = Setup::new(Retention::new(Some(1), None).unwrap());
    s.source.write("a.tar", b"a", days_ago(3));
    s.source.write("b.tar", b"b", days_ago(2));

    let JobResult::Planned(plan) = s.run_on(&s.remote, true).await else {
        panic!("Probelauf erwartet");
    };
    assert_eq!(plan.uploads.len(), 2);
    assert_eq!(names(&plan.expired), ["a.tar"]);
    assert_eq!(s.remote_names().await, Vec::<String>::new());
    assert_eq!(s.state.trash_entries(&s.owner).unwrap().len(), 0);
}

#[tokio::test]
async fn failed_uploads_are_reported_and_block_retention() {
    let s = Setup::new(Retention::new(Some(1), None).unwrap());
    s.source.write("alt.tar", b"alt", days_ago(5));
    s.apply().await;
    s.source.write("neu.tar", b"neu", days_ago(1));

    let failing = FailingOnUpload::new(&s.remote);
    failing
        .fail_uploads
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let JobResult::Applied(report) = s.run_on(&failing, false).await else {
        panic!("Ausführung erwartet");
    };
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.expired, Vec::<NodeName>::new(), "nichts löschen");
    assert_eq!(s.remote_names().await, ["alt.tar"]);
}

#[tokio::test]
async fn verification_catches_a_wrong_size() {
    let s = Setup::new(Retention::keep_all());
    s.source.write("kaputt.tar", b"inhalt", days_ago(1));
    let corrupting = FailingOnUpload::new(&s.remote);
    corrupting
        .corrupt_uploads
        .store(true, std::sync::atomic::Ordering::SeqCst);

    let JobResult::Applied(report) = s.run_on(&corrupting, false).await else {
        panic!("Ausführung erwartet");
    };
    assert_eq!(report.uploaded, Vec::<NodeName>::new());
    assert!(report.failures[0].message.contains("Prüfung"), "{report:?}");
}
