//! Contract tests: pin down the JSON format of the API.
//!
//! If a test fails, the protocol has changed. Then either revert the
//! change or deliberately increase `PROTOCOL_VERSION` and adjust the
//! expected texts.

#![allow(clippy::unwrap_used)]

use protonsync_api::wire::{decode, encode};
use protonsync_api::*;

/// Checks both directions: value → JSON and JSON → value.
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
        password: Password::new("secret"),
    };
    assert!(!format!("{request:?}").contains("secret"));
}

#[test]
fn job_requests() {
    assert_wire(&Request::Jobs, r#"{"type":"jobs"}"#);
    assert_wire(
        &Request::RunJob {
            job: "ha".into(),
            apply: false,
        },
        r#"{"type":"run_job","job":"ha","apply":false}"#,
    );
    assert_wire(
        &Request::JobHistory {
            job: "ha".into(),
            limit: 10,
        },
        r#"{"type":"job_history","job":"ha","limit":10}"#,
    );
}

fn sample_job_run() -> JobRunInfo {
    JobRunInfo {
        job: "ha".into(),
        started_at: 1,
        finished_at: 2,
        outcome: JobOutcomeInfo::Succeeded {
            uploaded: 1,
            unchanged: 2,
            expired: 3,
        },
    }
}

#[test]
fn job_responses() {
    assert_wire(
        &Response::Jobs {
            jobs: vec![JobInfo {
                id: "ha".into(),
                source: "/mnt/ha".into(),
                pattern: "*.tar".into(),
                target: "Backups/HA".into(),
                schedule: "0 3 * * *".into(),
                keep_last: Some(14),
                max_age_days: None,
                verify: JobVerification::Hash,
                running: false,
                last_run: None,
                next_run: Some(100),
            }],
        },
        concat!(
            r#"{"type":"jobs","jobs":[{"id":"ha","source":"/mnt/ha","pattern":"*.tar","#,
            r#""target":"Backups/HA","schedule":"0 3 * * *","keep_last":14,"max_age_days":null,"#,
            r#""verify":"hash","running":false,"last_run":null,"next_run":100}]}"#
        ),
    );
    assert_wire(
        &Response::JobRun {
            run: sample_job_run(),
            details: JobDetails {
                uploads: vec!["b.tar".into()],
                waiting: vec!["c.tar".into()],
                expired: vec!["a.tar".into()],
                failures: vec![FailureInfo {
                    path: "d.tar".into(),
                    message: "m".into(),
                }],
            },
        },
        concat!(
            r#"{"type":"job_run","run":{"job":"ha","started_at":1,"finished_at":2,"#,
            r#""outcome":{"type":"succeeded","uploaded":1,"unchanged":2,"expired":3}},"#,
            r#""details":{"uploads":["b.tar"],"waiting":["c.tar"],"expired":["a.tar"],"#,
            r#""failures":[{"path":"d.tar","message":"m"}]}}"#
        ),
    );
    assert_wire(
        &Response::JobHistory { runs: vec![] },
        r#"{"type":"job_history","runs":[]}"#,
    );
    assert_wire(
        &JobOutcomeInfo::Planned {
            uploads: 1,
            expired: 0,
        },
        r#"{"type":"planned","uploads":1,"expired":0}"#,
    );
    assert_wire(
        &JobOutcomeInfo::Failed {
            message: "m".into(),
            attempt: 2,
        },
        r#"{"type":"failed","message":"m","attempt":2}"#,
    );
}

#[test]
fn job_events() {
    assert_wire(
        &Event::JobStarted { job: "ha".into() },
        r#"{"type":"job_started","job":"ha"}"#,
    );
    assert_wire(
        &Event::JobFinished(sample_job_run()),
        concat!(
            r#"{"type":"job_finished","job":"ha","started_at":1,"finished_at":2,"#,
            r#""outcome":{"type":"succeeded","uploaded":1,"unchanged":2,"expired":3}}"#
        ),
    );
    assert_wire(
        &Event::JobGaveUp {
            job: "ha".into(),
            message: "m".into(),
            attempts: 5,
        },
        r#"{"type":"job_gave_up","job":"ha","message":"m","attempts":5}"#,
    );
}

#[test]
fn mount_requests() {
    assert_wire(&Request::Mounts, r#"{"type":"mounts"}"#);
    assert_wire(
        &Request::Pin {
            path: "/home/k/ProtonDrive/Photos".into(),
            state: PinChoice::Pinned,
        },
        r#"{"type":"pin","path":"/home/k/ProtonDrive/Photos","state":"pinned"}"#,
    );
    assert_wire(
        &Request::Free {
            path: None,
            apply: false,
        },
        r#"{"type":"free","path":null,"apply":false}"#,
    );
}

#[test]
fn mount_responses() {
    assert_wire(
        &Response::Mounts {
            mounts: vec![MountInfo {
                id: "home".into(),
                mountpoint: "/home/k/ProtonDrive".into(),
                remote_path: String::new(),
                mounted: true,
                keep_local: false,
                cache: Some(CacheInfo {
                    files: 2,
                    bytes: 4096,
                    pending_uploads: 1,
                }),
                pins: vec![PinInfo {
                    path: "Photos".into(),
                    state: PinChoice::Unpinned,
                }],
            }],
        },
        concat!(
            r#"{"type":"mounts","mounts":[{"id":"home","mountpoint":"/home/k/ProtonDrive","#,
            r#""remote_path":"","mounted":true,"keep_local":false,"#,
            r#""cache":{"files":2,"bytes":4096,"pending_uploads":1},"#,
            r#""pins":[{"path":"Photos","state":"unpinned"}]}]}"#
        ),
    );
    assert_wire(
        &Response::Freed {
            results: vec![FreeResult {
                mount: "home".into(),
                dry_run: true,
                files: vec!["Videos/a.mkv".into()],
                bytes: 300,
                skipped: 1,
            }],
        },
        concat!(
            r#"{"type":"freed","results":[{"mount":"home","dry_run":true,"#,
            r#""files":["Videos/a.mkv"],"bytes":300,"skipped":1}]}"#
        ),
    );
}

#[test]
fn photo_requests() {
    assert_wire(&Request::Photos, r#"{"type":"photos"}"#);
    assert_wire(
        &Request::SyncPhotos { apply: true },
        r#"{"type":"sync_photos","apply":true}"#,
    );
    assert_wire(&Request::PhotoDuplicates, r#"{"type":"photo_duplicates"}"#);
    assert_wire(
        &Request::TrashPhotoDuplicates {
            ids: vec!["v~n".into()],
            apply: false,
        },
        r#"{"type":"trash_photo_duplicates","ids":["v~n"],"apply":false}"#,
    );
}

fn sample_photo_run() -> PhotoRunInfo {
    PhotoRunInfo {
        finished_at: 1_700_000_100,
        outcome: PhotoOutcomeInfo::Applied(PhotoCounts {
            downloaded: 2,
            linked: 1,
            failures: vec![FailureInfo {
                path: "2026/10/a.jpg".into(),
                message: "m".into(),
            }],
            ..PhotoCounts::default()
        }),
    }
}

const SAMPLE_PHOTO_RUN_JSON: &str = concat!(
    r#"{"finished_at":1700000100,"outcome":{"type":"applied","downloaded":2,"uploaded":0,"#,
    r#""linked":1,"trashed_local":0,"trashed_remote":0,"ignored":0,"skipped":[],"#,
    r#""failures":[{"path":"2026/10/a.jpg","message":"m"}]}}"#
);

#[test]
fn photo_responses() {
    assert_wire(
        &Response::Photos(PhotoStatus {
            folder: "/home/k/Pictures".into(),
            direction: Direction::DownloadOnly,
            syncing: false,
            photos: Some(3),
            last_run: Some(sample_photo_run()),
            trash_entries: 1,
        }),
        &format!(
            "{}{}{}",
            r#"{"type":"photos","folder":"/home/k/Pictures","direction":"download_only","#,
            r#""syncing":false,"photos":3,"last_run":"#,
            format_args!("{SAMPLE_PHOTO_RUN_JSON},\"trash_entries\":1}}"),
        ),
    );
    assert_wire(
        &Response::PhotoRun(PhotoRunInfo {
            finished_at: 5,
            outcome: PhotoOutcomeInfo::Failed {
                message: "m".into(),
            },
        }),
        r#"{"type":"photo_run","finished_at":5,"outcome":{"type":"failed","message":"m"}}"#,
    );
    let photo = |id: &str| PhotoInfo {
        id: id.into(),
        name: "a.jpg".into(),
        size: 10,
        uploaded: 7,
        favorite: id == "k",
        albums: 0,
    };
    assert_wire(
        &Response::PhotoDuplicates {
            groups: vec![DuplicateGroupInfo {
                reason: DuplicateReasonInfo::SameContent,
                captured: 3,
                keep: photo("k"),
                others: vec![photo("o")],
            }],
        },
        concat!(
            r#"{"type":"photo_duplicates","groups":[{"reason":"same_content","captured":3,"#,
            r#""keep":{"id":"k","name":"a.jpg","size":10,"uploaded":7,"favorite":true,"albums":0},"#,
            r#""others":[{"id":"o","name":"a.jpg","size":10,"uploaded":7,"favorite":false,"albums":0}]}]}"#
        ),
    );
    assert_wire(
        &Response::PhotosTrashed {
            dry_run: true,
            photos: vec![TrashedPhotoInfo {
                id: "o".into(),
                path: "2026/10/a.jpg".into(),
            }],
        },
        r#"{"type":"photos_trashed","dry_run":true,"photos":[{"id":"o","path":"2026/10/a.jpg"}]}"#,
    );
}

#[test]
fn photo_events() {
    assert_wire(&Event::PhotoSyncStarted, r#"{"type":"photo_sync_started"}"#);
    assert_wire(
        &Event::PhotoSyncFinished(sample_photo_run()),
        &format!(
            "{{\"type\":\"photo_sync_finished\",{}",
            &SAMPLE_PHOTO_RUN_JSON[1..]
        ),
    );
}

#[test]
fn path_status() {
    assert_wire(
        &Request::PathStatus {
            paths: vec!["/home/k/ProtonDrive/a.txt".into()],
        },
        r#"{"type":"path_status","paths":["/home/k/ProtonDrive/a.txt"]}"#,
    );
    assert_wire(
        &Response::PathStatus {
            entries: vec![
                PathStatusInfo {
                    path: "/home/k/ProtonDrive/a.txt".into(),
                    state: PathState::OnlineOnly,
                    in_drive: true,
                    pinned: true,
                },
                PathStatusInfo {
                    path: "/tmp/x".into(),
                    state: PathState::Outside,
                    in_drive: false,
                    pinned: false,
                },
            ],
        },
        concat!(
            r#"{"type":"path_status","entries":[{"path":"/home/k/ProtonDrive/a.txt","#,
            r#""state":"online_only","in_drive":true,"pinned":true},"#,
            r#"{"path":"/tmp/x","state":"outside","in_drive":false,"pinned":false}]}"#
        ),
    );
}
