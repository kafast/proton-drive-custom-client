//! Vertragstests: halten das JSON-Format der API fest.
//!
//! Schlägt ein Test fehl, hat sich das Protokoll geändert. Dann entweder die
//! Änderung rückgängig machen oder bewusst `PROTOCOL_VERSION` erhöhen und die
//! erwarteten Texte anpassen.

#![allow(clippy::unwrap_used)]

use protonsync_api::wire::{decode, encode};
use protonsync_api::*;

/// Prüft beide Richtungen: Wert → JSON und JSON → Wert.
fn assert_wire<T>(value: &T, expected_json: &str)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let line = encode(value).unwrap();
    assert_eq!(line.trim_end(), expected_json);
    assert_eq!(&decode::<T>(expected_json).unwrap(), value);
}

#[test]
fn protocol_version_is_one() {
    assert_eq!(PROTOCOL_VERSION, 1);
}

#[test]
fn requests() {
    assert_wire(
        &ClientMessage::new(Request::Status),
        r#"{"version":1,"request":{"type":"status"}}"#,
    );
    assert_wire(
        &ClientMessage::new(Request::LoginStart {
            username: "k".into(),
            password: Password::new("pw"),
        }),
        r#"{"version":1,"request":{"type":"login_start","username":"k","password":"pw"}}"#,
    );
    assert_wire(
        &Request::LoginSecondFactor {
            code: "123456".into(),
        },
        r#"{"type":"login_second_factor","code":"123456"}"#,
    );
    assert_wire(
        &Request::LoginFinish {
            data_password: None,
        },
        r#"{"type":"login_finish","data_password":null}"#,
    );
    assert_wire(&Request::Logout, r#"{"type":"logout"}"#);
    assert_wire(
        &Request::Sync {
            pair: Some("main".into()),
            apply: true,
        },
        r#"{"type":"sync","pair":"main","apply":true}"#,
    );
    assert_wire(&Request::Conflicts, r#"{"type":"conflicts"}"#);
    assert_wire(
        &Request::ResolveConflict {
            pair: "main".into(),
            id: 7,
            resolution: ResolutionChoice::KeepBoth,
        },
        r#"{"type":"resolve_conflict","pair":"main","id":7,"resolution":"keep_both"}"#,
    );
    assert_wire(&Request::Trash, r#"{"type":"trash"}"#);
    assert_wire(
        &Request::RestoreTrash {
            pair: "main".into(),
            id: 3,
        },
        r#"{"type":"restore_trash","pair":"main","id":3}"#,
    );
    assert_wire(
        &Request::PurgeTrash { apply: false },
        r#"{"type":"purge_trash","apply":false}"#,
    );
    assert_wire(&Request::Subscribe, r#"{"type":"subscribe"}"#);
}

fn sample_result() -> PairSyncResult {
    PairSyncResult {
        pair: "main".into(),
        finished_at: 100,
        outcome: SyncOutcomeInfo::Applied(SyncSummary {
            uploaded: 1,
            failures: vec![FailureInfo {
                path: "a".into(),
                message: "m".into(),
            }],
            ..SyncSummary::default()
        }),
    }
}

#[test]
fn status_response() {
    assert_wire(
        &ServerMessage::Response(Response::Status(StatusInfo {
            daemon_version: "0.1.0".into(),
            account: Some("k".into()),
            login_pending: false,
            dry_run: true,
            pairs: vec![PairStatus {
                id: "main".into(),
                local_root: "/home/k/Proton".into(),
                remote_path: "protonsync".into(),
                direction: Direction::TwoWay,
                syncing: false,
                last_sync: None,
                open_conflicts: 0,
                trash_entries: 2,
            }],
        })),
        concat!(
            r#"{"kind":"response","body":{"type":"status","daemon_version":"0.1.0","#,
            r#""account":"k","login_pending":false,"dry_run":true,"pairs":[{"id":"main","#,
            r#""local_root":"/home/k/Proton","remote_path":"protonsync","direction":"two_way","#,
            r#""syncing":false,"last_sync":null,"open_conflicts":0,"trash_entries":2}]}}"#
        ),
    );
}

#[test]
fn login_and_sync_responses() {
    assert_wire(
        &Response::LoginStep(LoginStep {
            needs_second_factor: true,
            needs_data_password: false,
        }),
        r#"{"type":"login_step","needs_second_factor":true,"needs_data_password":false}"#,
    );
    assert_wire(
        &Response::LoggedIn {
            username: "k".into(),
        },
        r#"{"type":"logged_in","username":"k"}"#,
    );
    assert_wire(
        &Response::Sync {
            results: vec![sample_result()],
        },
        concat!(
            r#"{"type":"sync","results":[{"pair":"main","finished_at":100,"outcome":"#,
            r#"{"type":"applied","uploaded":1,"downloaded":0,"trashed_remote":0,"#,
            r#""trashed_local":0,"conflicts":0,"ignored":0,"skipped":[],"#,
            r#""failures":[{"path":"a","message":"m"}]}}]}"#
        ),
    );
    assert_wire(
        &SyncOutcomeInfo::Planned {
            steps: vec![PlannedStepInfo {
                path: "a".into(),
                action: StepAction::TrashRemote,
            }],
            skipped: vec![SkippedInfo {
                path: "b".into(),
                reason: "r".into(),
            }],
        },
        concat!(
            r#"{"type":"planned","steps":[{"path":"a","action":"trash_remote"}],"#,
            r#""skipped":[{"path":"b","reason":"r"}]}"#
        ),
    );
    assert_wire(
        &SyncOutcomeInfo::Failed {
            message: "m".into(),
        },
        r#"{"type":"failed","message":"m"}"#,
    );
}

#[test]
fn list_responses() {
    assert_wire(
        &Response::Conflicts {
            conflicts: vec![ConflictInfo {
                pair: "main".into(),
                id: 1,
                path: "a.txt".into(),
                kind: ConflictKindInfo::BothModified,
                conflict_copy: Some("a (Konflikt 5).txt".into()),
                detected_at: 5,
            }],
        },
        concat!(
            r#"{"type":"conflicts","conflicts":[{"pair":"main","id":1,"path":"a.txt","#,
            r#""kind":"both_modified","conflict_copy":"a (Konflikt 5).txt","detected_at":5}]}"#
        ),
    );
    assert_wire(
        &Response::Trash {
            entries: vec![TrashInfo {
                pair: "main".into(),
                id: 2,
                path: "x".into(),
                side: TrashSide::Remote,
                trashed_at: 1,
                expires_at: 2,
            }],
        },
        concat!(
            r#"{"type":"trash","entries":[{"pair":"main","id":2,"path":"x","side":"remote","#,
            r#""trashed_at":1,"expires_at":2}]}"#
        ),
    );
    assert_wire(&Response::Done, r#"{"type":"done"}"#);
    assert_wire(
        &Response::Error(ApiError::new(ErrorCode::NotLoggedIn, "m")),
        r#"{"type":"error","code":"not_logged_in","message":"m"}"#,
    );
}

#[test]
fn events() {
    assert_wire(
        &ServerMessage::Event(Event::SyncStarted {
            pair: "main".into(),
        }),
        r#"{"kind":"event","body":{"type":"sync_started","pair":"main"}}"#,
    );
    assert_wire(
        &Event::LoginRequired {
            message: "m".into(),
        },
        r#"{"type":"login_required","message":"m"}"#,
    );
    let line = encode(&Event::SyncFinished(sample_result())).unwrap();
    assert!(line.starts_with(r#"{"type":"sync_finished","pair":"main""#));
}

#[test]
fn passwords_never_appear_in_debug_output() {
    let request = Request::LoginStart {
        username: "k".into(),
        password: Password::new("geheim"),
    };
    assert!(!format!("{request:?}").contains("geheim"));
}
