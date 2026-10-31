//! The change log via the API: every applied sync is logged, against fakes.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::{Fixture, PAIR, T0, logged_in};
use protonsync_api::{ChangeInfo, ChangeKindInfo, ChangedSideInfo, ErrorCode, Request, Response};

async fn sync(f: &Fixture, apply: bool) {
    let response = f.core.handle(Request::Sync { pair: None, apply }).await;
    assert!(matches!(response, Response::Sync { .. }), "{response:?}");
}

async fn changes(f: &Fixture, since: Option<i64>) -> (Vec<ChangeInfo>, u32) {
    match f
        .core
        .handle(Request::Changes {
            since,
            pair: None,
            limit: 100,
        })
        .await
    {
        Response::Changes { changes, keep_days } => (changes, keep_days),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn applied_syncs_are_logged_and_dry_runs_are_not() {
    let f = logged_in().await;
    f.local.write("report.pdf", b"content", T0);
    sync(&f, false).await;
    assert_eq!(changes(&f, None).await.0, Vec::new());

    sync(&f, true).await;

    let (logged, keep_days) = changes(&f, None).await;
    assert_eq!(keep_days, 30);
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert_eq!(logged[0].pair, PAIR);
    assert_eq!(logged[0].path, "report.pdf");
    assert_eq!(logged[0].side, ChangedSideInfo::Remote);
    assert_eq!(logged[0].kind, ChangeKindInfo::Created);
    assert_eq!(logged[0].at, T0.unix_seconds());
}

#[tokio::test]
async fn old_changes_drop_out_after_the_retention() {
    let f = logged_in().await;
    f.local.write("old.txt", b"1", T0);
    sync(&f, true).await;

    f.clock.advance_days(31);
    f.local.write("new.txt", b"2", T0);
    sync(&f, true).await;

    let paths: Vec<String> = changes(&f, None)
        .await
        .0
        .into_iter()
        .map(|c| c.path)
        .collect();
    assert_eq!(paths, vec!["new.txt"]);
    let since_now = changes(&f, Some(T0.unix_seconds() + 1)).await.0;
    assert_eq!(since_now.len(), 1);
}

#[tokio::test]
async fn a_retention_of_zero_turns_the_log_off() {
    let f = logged_in().await;
    let f = Fixture {
        core: f.core.with_change_log(0),
        ..f
    };
    f.local.write("a.txt", b"1", T0);
    sync(&f, true).await;

    let (logged, keep_days) = changes(&f, None).await;
    assert_eq!(logged, Vec::new());
    assert_eq!(keep_days, 0);
}

#[tokio::test]
async fn an_unknown_pair_is_an_error() {
    let f = logged_in().await;
    let response = f
        .core
        .handle(Request::Changes {
            since: None,
            pair: Some("nope".into()),
            limit: 10,
        })
        .await;
    assert!(
        matches!(&response, Response::Error(e) if e.code == ErrorCode::NotFound),
        "{response:?}"
    );
}
