//! Szenariotests des Daemon-Kerns über die API-Anfragen, gegen Fakes.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::{Fixture, PAIR, T0, fixture_with, logged_in};
use protonsync_api::{
    ConflictKindInfo, ErrorCode, Event, Password, Request, ResolutionChoice, Response, StepAction,
    SyncOutcomeInfo, TrashSide,
};
use protonsync_application::ports::{RemoteError, RemoteStorage};
use protonsync_domain::node::NodeName;
use protonsync_domain::time::Timestamp;
use protonsync_testkit::{FAKE_PASSWORD, FAKE_TOTP, FAKE_USER, FakeAccount, FakeRemote};

async fn sync(f: &Fixture, apply: bool) -> SyncOutcomeInfo {
    match f.core.handle(Request::Sync { pair: None, apply }).await {
        Response::Sync { mut results } => results.remove(0).outcome,
        other => panic!("unerwartet: {other:?}"),
    }
}

fn error_code(response: Response) -> ErrorCode {
    match response {
        Response::Error(error) => error.code,
        other => panic!("Fehler erwartet: {other:?}"),
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
        panic!("Status erwartet");
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
        panic!("Anmeldeschritt erwartet");
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
        panic!("Status erwartet");
    };
    assert_eq!(status.account.as_deref(), Some(FAKE_USER));
}

#[tokio::test]
async fn wrong_password_and_out_of_order_steps_are_rejected() {
    let f = fixture_with(FakeAccount::new, true);
    let start = Request::LoginStart {
        username: FAKE_USER.into(),
        password: Password::new("falsch"),
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
    f.local.write("bericht.pdf", b"inhalt", T0);

    let SyncOutcomeInfo::Planned { steps, .. } = sync(&f, false).await else {
        panic!("Plan erwartet");
    };
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].action, StepAction::Upload);
    assert_eq!(remote_names(&f.remote).await, Vec::<String>::new());

    let SyncOutcomeInfo::Applied(summary) = sync(&f, true).await else {
        panic!("Ausführung erwartet");
    };
    assert_eq!(summary.uploaded, 1);
    assert_eq!(remote_names(&f.remote).await, ["bericht.pdf"]);
}

#[tokio::test]
async fn scheduled_runs_follow_dry_run_setting_and_events() {
    let f = fixture_with(|r| FakeAccount::new(r).with_stored_session(), true);
    f.core.startup().await;
    assert!(f.core.remote_changed(0).await, "nie abgeglichen");

    let result = f.core.scheduled_sync(0).await.unwrap();
    assert!(matches!(result.outcome, SyncOutcomeInfo::Planned { .. }));
    assert!(
        !f.core.remote_changed(0).await,
        "Probelauf merkt sich den Cursor"
    );

    let root = f.remote.root_folder().await.unwrap();
    let name = NodeName::new("neu.txt").unwrap();
    f.remote
        .upload_file(&root.id, &name, &b"x"[..], 1, T0)
        .await
        .unwrap();
    assert!(f.core.remote_changed(0).await);
}

#[tokio::test]
async fn conflicts_are_listed_and_resolved() {
    let f = logged_in().await;
    f.local.write("a.txt", b"eins", T0);
    sync(&f, true).await;

    // Beide Seiten ändern dieselbe Datei.
    let root = f.remote.root_folder().await.unwrap();
    let file = f.remote.list_children(&root.id).await.unwrap().remove(0);
    f.remote
        .upload_revision(&file.id, &b"entfernt"[..], 8, T0)
        .await
        .unwrap();
    f.clock.advance_days(1);
    f.local.write("a.txt", b"lokal geaendert", f.clock_now());
    let SyncOutcomeInfo::Applied(summary) = sync(&f, true).await else {
        panic!("Ausführung erwartet");
    };
    assert_eq!(summary.conflicts, 1);

    let Response::Conflicts { conflicts } = f.core.handle(Request::Conflicts).await else {
        panic!("Konflikte erwartet");
    };
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].kind, ConflictKindInfo::BothModified);

    let unknown_pair = Request::ResolveConflict {
        pair: "gibts-nicht".into(),
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
    f.local.write("alt.txt", b"x", T0);
    sync(&f, true).await;
    f.local.remove("alt.txt");
    let SyncOutcomeInfo::Applied(summary) = sync(&f, true).await else {
        panic!("Ausführung erwartet");
    };
    assert_eq!(summary.trashed_remote, 1);

    let Response::Trash { entries } = f.core.handle(Request::Trash).await else {
        panic!("Papierkorb erwartet");
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].side, TrashSide::Remote);

    let restore = Request::RestoreTrash {
        pair: PAIR.into(),
        id: entries[0].id,
    };
    assert_eq!(f.core.handle(restore).await, Response::Done);
    assert_eq!(remote_names(&f.remote).await, ["alt.txt"]);
}

#[tokio::test]
async fn purge_lists_expired_entries_and_deletes_only_with_apply() {
    let f = logged_in().await;
    f.local.write("alt.txt", b"x", T0);
    sync(&f, true).await;
    f.local.remove("alt.txt");
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
        panic!("Liste erwartet");
    };
    assert_eq!(entries.len(), 1, "Probelauf meldet den Eintrag");
    let Response::Trash { entries } = f.core.handle(Request::Trash).await else {
        panic!("Liste erwartet");
    };
    assert_eq!(entries.len(), 1, "Probelauf löscht nichts");

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
