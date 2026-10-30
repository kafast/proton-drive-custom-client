//! Scenario tests of the daemon core via the API requests, against fakes.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::{Fixture, PAIR, T0, fixture_with, fixture_with_pair, logged_in};
use protonsync_api::{
    ConflictKindInfo, ErrorCode, Event, Password, PathState, Request, ResolutionChoice, Response,
    StepAction, SyncOutcomeInfo, TrashSide,
};
use protonsync_application::ports::{AccountError, RemoteError, RemoteStorage};
use protonsync_domain::node::NodeName;
use protonsync_domain::time::Timestamp;
use protonsync_testkit::{FAKE_PASSWORD, FAKE_TOTP, FAKE_USER, FakeAccount, FakeRemote};

async fn sync(f: &Fixture, apply: bool) -> SyncOutcomeInfo {
    match f.core.handle(Request::Sync { pair: None, apply }).await {
        Response::Sync { mut results } => results.remove(0).outcome,
        other => panic!("unexpected: {other:?}"),
    }
}

fn error_code(response: Response) -> ErrorCode {
    match response {
        Response::Error(error) => error.code,
        other => panic!("expected an error: {other:?}"),
    }
}

async fn remote_names(remote: &FakeRemote) -> Vec<String> {
    let root = remote.root_folder().await.unwrap();
    remote
        .list_children(&root.id)
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.name.to_string())
        .collect()
}

#[tokio::test]
async fn status_before_login_lists_pairs_and_dry_run() {
    let f = fixture_with(FakeAccount::new, true);
    f.core.startup().await;
    let Response::Status(status) = f.core.handle(Request::Status).await else {
        panic!("expected a status");
    };
    assert_eq!(status.account, None);
    assert!(status.dry_run);
    assert_eq!(status.pairs.len(), 1);
    assert_eq!(status.pairs[0].remote_path, "protonsync");
    assert_eq!(
        error_code(
            f.core
                .handle(Request::Sync {
                    pair: None,
                    apply: true
                })
                .await
        ),
        ErrorCode::NotLoggedIn
    );
    assert_eq!(f.core.scheduled_sync(0).await, None);
}

#[tokio::test]
async fn login_with_second_factor() {
    let f = fixture_with(|r| FakeAccount::new(r).with_second_factor(), true);
    let start = Request::LoginStart {
        username: FAKE_USER.into(),
        password: Password::new(FAKE_PASSWORD),
    };
    let Response::LoginStep(step) = f.core.handle(start).await else {
        panic!("expected a login step");
    };
    assert!(step.needs_second_factor);

    let wrong = Request::LoginSecondFactor {
        code: "000000".into(),
    };
    assert_eq!(
        error_code(f.core.handle(wrong).await),
        ErrorCode::LoginRejected
    );
    let right = Request::LoginSecondFactor {
        code: FAKE_TOTP.into(),
    };
    assert!(matches!(f.core.handle(right).await, Response::LoginStep(_)));

    let finish = Request::LoginFinish {
        data_password: None,
    };
    assert_eq!(
        f.core.handle(finish).await,
        Response::LoggedIn {
            username: FAKE_USER.into()
        }
    );
    let Response::Status(status) = f.core.handle(Request::Status).await else {
        panic!("expected a status");
    };
    assert_eq!(status.account.as_deref(), Some(FAKE_USER));
}

#[tokio::test]
async fn wrong_password_and_out_of_order_steps_are_rejected() {
    let f = fixture_with(FakeAccount::new, true);
    let start = Request::LoginStart {
        username: FAKE_USER.into(),
        password: Password::new("wrong"),
    };
    assert_eq!(
        error_code(f.core.handle(start).await),
        ErrorCode::LoginRejected
    );
    let finish = Request::LoginFinish {
        data_password: None,
    };
    assert_eq!(
        error_code(f.core.handle(finish).await),
        ErrorCode::InvalidRequest
    );
}

#[tokio::test]
async fn dry_run_plans_and_apply_uploads() {
    let f = logged_in().await;
    f.local.write("report.pdf", b"content", T0);

    let SyncOutcomeInfo::Planned { steps, .. } = sync(&f, false).await else {
        panic!("expected a plan");
    };
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].action, StepAction::Upload);
    assert_eq!(remote_names(&f.remote).await, Vec::<String>::new());

    let SyncOutcomeInfo::Applied(summary) = sync(&f, true).await else {
        panic!("expected an applied run");
    };
    assert_eq!(summary.uploaded, 1);
    assert_eq!(remote_names(&f.remote).await, ["report.pdf"]);
}

#[tokio::test]
async fn scheduled_runs_follow_dry_run_setting_and_events() {
    let f = fixture_with(|r| FakeAccount::new(r).with_stored_session(), true);
    f.core.startup().await;
    assert!(f.core.remote_changed(0).await, "never synced");

    let result = f.core.scheduled_sync(0).await.unwrap();
    assert!(matches!(result.outcome, SyncOutcomeInfo::Planned { .. }));
    assert!(
        !f.core.remote_changed(0).await,
        "dry run remembers the cursor"
    );

    let root = f.remote.root_folder().await.unwrap();
    let name = NodeName::new("new.txt").unwrap();
    f.remote
        .upload_file(&root.id, &name, &b"x"[..], 1, T0)
        .await
        .unwrap();
    assert!(f.core.remote_changed(0).await);
}

#[tokio::test]
async fn conflicts_are_listed_and_resolved() {
    let f = logged_in().await;
    f.local.write("a.txt", b"one", T0);
    sync(&f, true).await;

    // Both sides change the same file.
    let root = f.remote.root_folder().await.unwrap();
    let file = f.remote.list_children(&root.id).await.unwrap().remove(0);
    f.remote
        .upload_revision(&file.id, &b"remotely"[..], 8, T0)
        .await
        .unwrap();
    f.clock.advance_days(1);
    f.local.write("a.txt", b"changed locally", f.clock_now());
    let SyncOutcomeInfo::Applied(summary) = sync(&f, true).await else {
        panic!("expected an applied run");
    };
    assert_eq!(summary.conflicts, 1);

    let Response::Conflicts { conflicts } = f.core.handle(Request::Conflicts).await else {
        panic!("expected conflicts");
    };
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].kind, ConflictKindInfo::BothModified);

    let unknown_pair = Request::ResolveConflict {
        pair: "does-not-exist".into(),
        id: conflicts[0].id,
        resolution: ResolutionChoice::KeepBoth,
    };
    assert_eq!(
        error_code(f.core.handle(unknown_pair).await),
        ErrorCode::NotFound
    );

    let resolve = Request::ResolveConflict {
        pair: PAIR.into(),
        id: conflicts[0].id,
        resolution: ResolutionChoice::KeepBoth,
    };
    assert_eq!(f.core.handle(resolve).await, Response::Done);
    assert_eq!(
        f.core.handle(Request::Conflicts).await,
        Response::Conflicts {
            conflicts: Vec::new()
        }
    );
}

#[tokio::test]
async fn deletions_go_to_trash_and_can_be_restored() {
    let f = logged_in().await;
    f.local.write("old.txt", b"x", T0);
    sync(&f, true).await;
    f.local.remove("old.txt");
    let SyncOutcomeInfo::Applied(summary) = sync(&f, true).await else {
        panic!("expected an applied run");
    };
    assert_eq!(summary.trashed_remote, 1);

    let Response::Trash { entries } = f.core.handle(Request::Trash).await else {
        panic!("expected the trash");
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].side, TrashSide::Remote);

    let restore = Request::RestoreTrash {
        pair: PAIR.into(),
        id: entries[0].id,
    };
    assert_eq!(f.core.handle(restore).await, Response::Done);
    assert_eq!(remote_names(&f.remote).await, ["old.txt"]);
}

#[tokio::test]
async fn purge_lists_expired_entries_and_deletes_only_with_apply() {
    let f = logged_in().await;
    f.local.write("old.txt", b"x", T0);
    sync(&f, true).await;
    f.local.remove("old.txt");
    sync(&f, true).await;

    let purge = |apply| Request::PurgeTrash { apply };
    assert_eq!(
        f.core.handle(purge(true)).await,
        Response::Trash {
            entries: Vec::new()
        }
    );
    f.clock.advance_days(31);
    let Response::Trash { entries } = f.core.handle(purge(false)).await else {
        panic!("expected a list");
    };
    assert_eq!(entries.len(), 1, "dry run reports the entry");
    let Response::Trash { entries } = f.core.handle(Request::Trash).await else {
        panic!("expected a list");
    };
    assert_eq!(entries.len(), 1, "dry run deletes nothing");

    f.core.handle(purge(true)).await;
    assert_eq!(
        f.core.handle(Request::Trash).await,
        Response::Trash {
            entries: Vec::new()
        }
    );
}

#[tokio::test]
async fn subscribers_see_sync_and_login_events() {
    let f = logged_in().await;
    let mut events = f.core.subscribe();
    sync(&f, true).await;
    assert_eq!(
        events.recv().await.unwrap(),
        Event::SyncStarted { pair: PAIR.into() }
    );
    assert!(matches!(
        events.recv().await.unwrap(),
        Event::SyncFinished(_)
    ));

    f.remote.fail_next_call(RemoteError::Unauthorized);
    let outcome = sync(&f, true).await;
    assert!(matches!(outcome, SyncOutcomeInfo::Failed { .. }));
    let mut saw_login_required = false;
    while let Ok(event) = events.try_recv() {
        saw_login_required |= matches!(event, Event::LoginRequired { .. });
    }
    assert!(saw_login_required);
}

#[tokio::test]
async fn logout_disconnects() {
    let f = logged_in().await;
    assert_eq!(f.core.handle(Request::Logout).await, Response::Done);
    assert_eq!(
        error_code(
            f.core
                .handle(Request::Sync {
                    pair: None,
                    apply: false
                })
                .await
        ),
        ErrorCode::NotLoggedIn
    );
    assert!(!f.core.remote_changed(0).await);
}

#[tokio::test]
async fn failed_folder_connection_reports_the_cause_and_is_retried() {
    let refused = AccountError::Unavailable("HTTP 422: folder limit reached".into());
    let f = fixture_with(
        |r| {
            FakeAccount::new(r)
                .with_stored_session()
                .failing_open_remote(refused, 2)
        },
        false,
    );
    // The first attempt at startup fails, the second one at the manual sync.
    f.core.startup().await;
    let SyncOutcomeInfo::Failed { message } = sync(&f, false).await else {
        panic!("expected a failure");
    };
    assert!(message.contains("folder limit reached"), "{message}");
    assert!(!message.contains("not logged in"), "{message}");

    // The next attempt connects, so the pair works without a new login.
    f.local.write("report.pdf", b"content", T0);
    let SyncOutcomeInfo::Applied(summary) = sync(&f, true).await else {
        panic!("expected an applied run");
    };
    assert_eq!(summary.uploaded, 1);
}

#[tokio::test]
async fn unknown_pair_in_sync_request_is_not_found() {
    let f = logged_in().await;
    let request = Request::Sync {
        pair: Some("x".into()),
        apply: false,
    };
    assert_eq!(
        error_code(f.core.handle(request).await),
        ErrorCode::NotFound
    );
}

impl Fixture {
    fn clock_now(&self) -> Timestamp {
        use protonsync_application::ports::Clock;
        self.clock.now()
    }
}

#[tokio::test]
async fn scheduled_runs_read_only_events_and_the_safety_net_reads_everything() {
    let f = logged_in().await;
    f.core.scheduled_sync(0).await.unwrap();
    let root = f.remote.root_folder().await.unwrap();
    let name = NodeName::new("new.txt").unwrap();
    f.remote
        .upload_file(&root.id, &name, &b"x"[..], 1, T0)
        .await
        .unwrap();

    let before = f.remote.list_calls();
    let result = f.core.scheduled_sync(0).await.unwrap();
    assert!(matches!(result.outcome, SyncOutcomeInfo::Applied(ref s) if s.downloaded == 1));
    assert_eq!(f.remote.list_calls(), before, "only events caught up");

    f.core.scheduled_full_sync(0).await.unwrap();
    assert!(
        f.remote.list_calls() > before,
        "safety net reads everything"
    );
}

#[tokio::test]
async fn stale_cache_triggers_an_immediate_full_run() {
    let f = logged_in().await;
    let root = f.remote.root_folder().await.unwrap();
    let name = NodeName::new("important.txt").unwrap();
    let file = f
        .remote
        .upload_file(&root.id, &name, &b"x"[..], 1, T0)
        .await
        .unwrap();
    f.core.scheduled_sync(0).await.unwrap();
    f.remote.trash(&file.id).await.unwrap();
    f.remote.silence_events(true);
    f.remote.restore(&file.id).await.unwrap();
    f.remote.silence_events(false);

    // The run sees the deletion, but the check still finds the file.
    let result = f.core.scheduled_sync(0).await.unwrap();
    assert!(matches!(result.outcome, SyncOutcomeInfo::Applied(ref s) if s.failures.len() == 1));
    assert!(f.local.content("important.txt").is_some());
    let trigger = f.core.trigger(0).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), trigger.notified())
        .await
        .expect("immediate rerun triggered");
}

#[tokio::test]
async fn path_status_compares_with_the_last_sync() {
    let f = logged_in().await;
    f.local.write("old.txt", b"old", T0);
    sync(&f, true).await;
    f.local.write("new.txt", b"new", T0);
    f.local.mkdir("Folder");

    let root = "/home/k/Proton";
    let paths: Vec<String> = ["old.txt", "new.txt", "Folder", ".protonsync-trash/x"]
        .iter()
        .map(|rest| format!("{root}/{rest}"))
        .chain([root.to_owned(), "/home/k/elsewhere".to_owned()])
        .collect();
    let Response::PathStatus { entries } = f
        .core
        .handle(Request::PathStatus {
            paths: paths.clone(),
        })
        .await
    else {
        panic!("expected a state");
    };
    let states: Vec<PathState> = entries.iter().map(|entry| entry.state).collect();
    assert_eq!(
        states,
        [
            PathState::Synced,
            PathState::Pending,
            PathState::Folder,
            PathState::Outside,
            PathState::Folder,
            PathState::Outside,
        ]
    );
    assert_eq!(entries[1].path, paths[1], "order of the request");

    f.local.write("old.txt", b"changed", T0);
    let Response::PathStatus { entries } = f
        .core
        .handle(Request::PathStatus {
            paths: vec![paths[0].clone()],
        })
        .await
    else {
        panic!("expected a state");
    };
    assert_eq!(entries[0].state, PathState::Pending, "changed locally");
}

#[tokio::test]
async fn shared_pairs_open_the_folder_in_shared_with_me_and_only_download() {
    use protonsync_domain::path::RelativePath;
    use protonsync_domain::sync_pair::SyncDirection;

    let mut opened = None;
    let f = fixture_with_pair(
        |remote| {
            let account = FakeAccount::new(remote).with_stored_session();
            opened = Some(account.shared_opened());
            account
        },
        false,
        |config| {
            config.shared = true;
            config.remote = RelativePath::parse("Family/Photos").unwrap();
            config.direction = SyncDirection::BackupOnly;
        },
    );
    f.core.startup().await;
    let root = f.remote.root_folder().await.unwrap();
    f.remote
        .upload_file(
            &root.id,
            &NodeName::new("from-owner.txt").unwrap(),
            &b"shared"[..],
            6,
            T0,
        )
        .await
        .unwrap();
    f.local.write("local-only.txt", b"mine", T0);

    let SyncOutcomeInfo::Applied(summary) = sync(&f, true).await else {
        panic!("expected an applied run");
    };
    assert_eq!(summary.downloaded, 1);
    assert_eq!(summary.uploaded, 0);
    assert_eq!(remote_names(&f.remote).await, ["from-owner.txt"]);
    let opened = opened.unwrap();
    assert_eq!(
        opened.lock().unwrap().first().map(String::as_str),
        Some("Family/Photos")
    );
}
