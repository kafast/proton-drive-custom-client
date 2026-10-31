//! Commands against a scripted daemon: correct requests, output, exit codes.

#![allow(clippy::unwrap_used)]

use std::collections::VecDeque;
use std::io;

use clap::Parser;
use protonsync_api::{
    ApiError, ErrorCode, JobDetails, JobOutcomeInfo, JobRunInfo, LoginStep, PairSyncResult,
    PhotoOutcomeInfo, PhotoRunInfo, PinChoice, Request, Response, StatusInfo, SyncOutcomeInfo,
    SyncSummary,
};
use protonsync_cli::client::{ClientError, Transport};
use protonsync_cli::commands::{self, Cli, Prompter, exit};
use protonsync_cli::i18n::Lang;

#[derive(Default)]
struct ScriptedDaemon {
    answers: VecDeque<Response>,
    requests: Vec<Request>,
}

impl Transport for &mut ScriptedDaemon {
    fn request(&mut self, request: Request) -> Result<Response, ClientError> {
        self.requests.push(request);
        Ok(self.answers.pop_front().unwrap_or(Response::Done))
    }
}

struct ScriptedUser(VecDeque<&'static str>);

impl Prompter for ScriptedUser {
    fn ask(&mut self, _label: &str) -> io::Result<String> {
        Ok(self.0.pop_front().unwrap_or_default().to_owned())
    }

    fn ask_secret(&mut self, label: &str) -> io::Result<String> {
        self.ask(label)
    }

    fn tell(&mut self, _text: &str) -> io::Result<()> {
        Ok(())
    }
}

struct Run {
    code: u8,
    out: String,
    err: String,
    requests: Vec<Request>,
}

fn run(args: &[&str], answers: Vec<Response>, user: &[&'static str]) -> Run {
    let cli =
        Cli::try_parse_from(std::iter::once("protonsync").chain(args.iter().copied())).unwrap();
    let mut daemon = ScriptedDaemon {
        answers: answers.into(),
        requests: Vec::new(),
    };
    let mut prompter = ScriptedUser(user.iter().copied().collect());
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = commands::run(
        &cli,
        || Ok(&mut daemon),
        &mut prompter,
        &mut out,
        &mut err,
        Lang::De,
        &jiff::tz::TimeZone::UTC,
    );
    Run {
        code,
        out: String::from_utf8(out).unwrap(),
        err: String::from_utf8(err).unwrap(),
        requests: daemon.requests,
    }
}

fn step(needs_second_factor: bool, needs_data_password: bool) -> Response {
    Response::LoginStep(LoginStep {
        needs_second_factor,
        needs_data_password,
    })
}

fn password_login() -> Response {
    Response::LoginMethod { browser: false }
}

#[test]
fn browser_login_shows_the_link_and_waits() {
    let answers = vec![
        Response::LoginMethod { browser: true },
        Response::LoginBrowserUrl {
            url: "https://account.example.invalid/x".into(),
        },
        Response::LoginBrowserPending,
        Response::LoggedIn {
            username: "k@example.invalid".into(),
        },
    ];
    let result = run(&["login"], answers, &[]);
    assert_eq!(result.code, exit::OK);
    assert_eq!(result.out, "Angemeldet als k@example.invalid\n");
    assert_eq!(
        result.requests,
        vec![
            Request::LoginMethod,
            Request::LoginBrowserStart,
            Request::LoginBrowserWait,
            Request::LoginBrowserWait,
        ]
    );
}

#[test]
fn older_daemons_get_the_password_login() {
    let answers = vec![
        Response::Error(ApiError::new(ErrorCode::InvalidRequest, "unknown")),
        step(false, false),
        Response::LoggedIn {
            username: "k".into(),
        },
    ];
    let result = run(&["login", "--username", "k"], answers, &["pw"]);
    assert_eq!(result.code, exit::OK);
    assert!(matches!(result.requests[1], Request::LoginStart { .. }));
}

#[test]
fn login_asks_for_everything_the_account_needs() {
    let answers = vec![
        password_login(),
        step(true, true),
        step(false, true),
        Response::LoggedIn {
            username: "k".into(),
        },
    ];
    let result = run(&["login"], answers, &["k", "pw", "123456", "data"]);
    assert_eq!(result.code, exit::OK);
    assert_eq!(result.out, "Angemeldet als k\n");
    assert_eq!(result.requests[0], Request::LoginMethod);
    assert!(
        matches!(&result.requests[1], Request::LoginStart { username, password } if username == "k" && password.expose() == "pw")
    );
    assert_eq!(
        result.requests[2],
        Request::LoginSecondFactor {
            code: "123456".into()
        }
    );
    assert!(
        matches!(&result.requests[3], Request::LoginFinish { data_password: Some(p) } if p.expose() == "data")
    );
}

#[test]
fn login_without_second_factor_skips_questions() {
    let answers = vec![
        password_login(),
        step(false, false),
        Response::LoggedIn {
            username: "k".into(),
        },
    ];
    let result = run(&["login", "--username", "k"], answers, &["pw"]);
    assert_eq!(result.code, exit::OK);
    assert_eq!(result.requests.len(), 3);
    assert_eq!(
        result.requests[2],
        Request::LoginFinish {
            data_password: None
        }
    );
}

#[test]
fn rejected_login_has_its_own_exit_code() {
    let answers = vec![
        password_login(),
        Response::Error(ApiError::new(ErrorCode::LoginRejected, "wrong")),
    ];
    let result = run(&["login", "--username", "k"], answers, &["pw"]);
    assert_eq!(result.code, exit::LOGIN_REJECTED);
    assert_eq!(result.err, "wrong\n");
}

#[test]
fn sync_defaults_to_dry_run() {
    let result = run(
        &["sync"],
        vec![Response::Sync {
            results: Vec::new(),
        }],
        &[],
    );
    assert_eq!(
        result.requests,
        [Request::Sync {
            pair: None,
            apply: false
        }]
    );
    let result = run(&["sync", "--pair", "main", "--apply"], vec![], &[]);
    assert_eq!(
        result.requests,
        [Request::Sync {
            pair: Some("main".into()),
            apply: true
        }]
    );
}

#[test]
fn partial_sync_failures_set_exit_code() {
    let failed = PairSyncResult {
        pair: "main".into(),
        finished_at: 0,
        outcome: SyncOutcomeInfo::Failed {
            message: "x".into(),
        },
    };
    let result = run(
        &["sync", "--apply"],
        vec![Response::Sync {
            results: vec![failed],
        }],
        &[],
    );
    assert_eq!(result.code, exit::PARTIAL);

    let ok = PairSyncResult {
        pair: "main".into(),
        finished_at: 0,
        outcome: SyncOutcomeInfo::Applied(SyncSummary::default()),
    };
    let result = run(
        &["sync", "--apply"],
        vec![Response::Sync { results: vec![ok] }],
        &[],
    );
    assert_eq!(result.code, exit::OK);
}

#[test]
fn not_logged_in_has_its_own_exit_code_and_hint() {
    let answers = vec![Response::Error(ApiError::new(ErrorCode::NotLoggedIn, "x"))];
    let result = run(&["status"], answers, &[]);
    assert_eq!(result.code, exit::NOT_LOGGED_IN);
    assert!(result.err.contains("protonsync login"));
}

#[test]
fn json_output_is_the_daemon_response() {
    let status = StatusInfo {
        daemon_version: "0.1.0".into(),
        account: None,
        login_pending: false,
        dry_run: true,
        pairs: Vec::new(),
    };
    let result = run(
        &["--json", "status"],
        vec![Response::Status(status.clone())],
        &[],
    );
    let parsed: Response = serde_json::from_str(&result.out).unwrap();
    assert_eq!(parsed, Response::Status(status));
}

#[test]
fn subcommands_map_to_requests() {
    let cases: Vec<(&[&str], Request)> = vec![
        (&["conflicts"], Request::Conflicts),
        (&["conflicts", "list"], Request::Conflicts),
        (
            &["conflicts", "resolve", "main", "3", "keep-remote"],
            Request::ResolveConflict {
                pair: "main".into(),
                id: 3,
                resolution: protonsync_api::ResolutionChoice::KeepRemote,
            },
        ),
        (&["trash"], Request::Trash),
        (
            &["trash", "restore", "main", "5"],
            Request::RestoreTrash {
                pair: "main".into(),
                id: 5,
            },
        ),
        (&["trash", "purge"], Request::PurgeTrash { apply: false }),
        (&["logout"], Request::Logout),
    ];
    for (args, expected) in cases {
        assert_eq!(run(args, vec![], &[]).requests, [expected], "{args:?}");
    }
    assert_eq!(run(&["logout"], vec![], &[]).out, "Abgemeldet.\n");
}

#[test]
fn unreachable_daemon_and_example_config() {
    let cli = Cli::try_parse_from(["protonsync", "status"]).unwrap();
    let mut prompter = ScriptedUser(VecDeque::new());
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let unreachable = || -> Result<&mut ScriptedDaemon, ClientError> {
        Err(ClientError::Unreachable {
            path: "/run/x.sock".into(),
            message: "not there".into(),
        })
    };
    let code = commands::run(
        &cli,
        unreachable,
        &mut prompter,
        &mut out,
        &mut err,
        Lang::De,
        &jiff::tz::TimeZone::UTC,
    );
    assert_eq!(code, exit::DAEMON_UNREACHABLE);
    assert!(String::from_utf8(err).unwrap().contains("/run/x.sock"));

    let cli = Cli::try_parse_from(["protonsync", "config", "example"]).unwrap();
    let mut out = Vec::new();
    let code = commands::run(
        &cli,
        unreachable,
        &mut prompter,
        &mut out,
        &mut Vec::new(),
        Lang::De,
        &jiff::tz::TimeZone::UTC,
    );
    assert_eq!(code, exit::OK);
    let example = String::from_utf8(out).unwrap();
    assert!(example.contains("[[pair]]") && example.contains("# [[job]]"));
}

#[test]
fn example_config_is_bilingual_in_both_languages() {
    let cli = Cli::try_parse_from(["protonsync", "config", "example"]).unwrap();
    let unreachable =
        || -> Result<&mut ScriptedDaemon, ClientError> { Err(ClientError::Io("unused".into())) };
    for lang in [Lang::De, Lang::En] {
        let mut out = Vec::new();
        let code = commands::run(
            &cli,
            unreachable,
            &mut ScriptedUser(VecDeque::new()),
            &mut out,
            &mut Vec::new(),
            lang,
            &jiff::tz::TimeZone::UTC,
        );
        assert_eq!(code, exit::OK);
        let example = String::from_utf8(out).unwrap();
        assert_eq!(example, commands::EXAMPLE_CONFIG);
        assert!(example.contains("# DE: ") && example.contains("# EN: "));
    }
}

#[test]
fn job_commands_send_the_right_requests() {
    let r = run(&["jobs"], vec![Response::Jobs { jobs: vec![] }], &[]);
    assert_eq!((r.code, r.requests), (exit::OK, vec![Request::Jobs]));
    assert!(r.out.contains("Keine Upload-Jobs"));

    let r = run(&["jobs", "history", "ha"], vec![], &[]);
    assert_eq!(
        r.requests,
        [Request::JobHistory {
            job: "ha".into(),
            limit: commands::DEFAULT_HISTORY
        }]
    );

    let failed = Response::JobRun {
        run: JobRunInfo {
            job: "ha".into(),
            started_at: 1,
            finished_at: 2,
            outcome: JobOutcomeInfo::Failed {
                message: "network".into(),
                attempt: 1,
            },
        },
        details: JobDetails::default(),
    };
    let r = run(
        &["--json", "jobs", "run", "ha", "--apply"],
        vec![failed],
        &[],
    );
    assert_eq!(
        r.requests,
        [Request::RunJob {
            job: "ha".into(),
            apply: true
        }]
    );
    assert_eq!(r.code, exit::PARTIAL, "failed run");
    assert!(r.out.contains("\"type\": \"job_run\""), "{}", r.out);
}

#[test]
fn mount_commands_send_absolute_paths() {
    let cwd = std::env::current_dir().unwrap();
    let inside = |rest: &str| cwd.join(rest).to_str().unwrap().to_owned();

    let r = run(&["pin", "Fotos"], vec![], &[]);
    assert_eq!(
        r.requests,
        [Request::Pin {
            path: inside("Fotos"),
            state: PinChoice::Pinned
        }]
    );
    assert!(r.out.contains("immer lokal"), "{}", r.out);

    let r = run(
        &["unpin", "/home/k/Drive/Videos", "/home/k/Drive/Musik"],
        vec![],
        &[],
    );
    assert_eq!(
        r.requests,
        [
            Request::Pin {
                path: "/home/k/Drive/Videos".into(),
                state: PinChoice::Unpinned
            },
            Request::Pin {
                path: "/home/k/Drive/Musik".into(),
                state: PinChoice::Unpinned
            }
        ]
    );
    assert!(
        Cli::try_parse_from(["protonsync", "pin"]).is_err(),
        "without path"
    );

    let r = run(&["file-status", "Fotos", "/tmp/x"], vec![], &[]);
    assert_eq!(
        r.requests,
        [Request::PathStatus {
            paths: vec![inside("Fotos"), "/tmp/x".into()]
        }]
    );

    let r = run(&["free"], vec![Response::Freed { results: vec![] }], &[]);
    assert_eq!(
        r.requests,
        [Request::Free {
            path: None,
            apply: false
        }]
    );
    let r = run(&["free", "Videos", "--apply"], vec![], &[]);
    assert_eq!(
        r.requests,
        [Request::Free {
            path: Some(inside("Videos")),
            apply: true
        }]
    );

    let r = run(&["mounts"], vec![Response::Mounts { mounts: vec![] }], &[]);
    assert_eq!((r.code, r.requests), (exit::OK, vec![Request::Mounts]));
    assert!(r.out.contains("Keine Laufwerke"), "{}", r.out);
}

#[test]
fn photo_commands_send_the_right_requests() {
    for (args, expected) in [
        (&["photos"][..], Request::Photos),
        (&["photos", "status"][..], Request::Photos),
        (
            &["photos", "sync"][..],
            Request::SyncPhotos { apply: false },
        ),
        (&["photos", "duplicates"][..], Request::PhotoDuplicates),
        (
            &["photos", "duplicates", "trash", "a", "b"][..],
            Request::TrashPhotoDuplicates {
                ids: vec!["a".into(), "b".into()],
                apply: false,
            },
        ),
    ] {
        assert_eq!(run(args, vec![], &[]).requests, [expected], "{args:?}");
    }
    assert!(
        Cli::try_parse_from(["protonsync", "photos", "duplicates", "trash"]).is_err(),
        "without ID"
    );

    let failed = Response::PhotoRun(PhotoRunInfo {
        finished_at: 1,
        outcome: PhotoOutcomeInfo::Failed {
            message: "network".into(),
        },
    });
    let r = run(&["photos", "sync", "--apply"], vec![failed], &[]);
    assert_eq!(r.requests, [Request::SyncPhotos { apply: true }]);
    assert_eq!(r.code, exit::PARTIAL, "failed run");
    assert!(r.out.contains("network"), "{}", r.out);
}

fn settings_sample() -> protonsync_api::SettingsInfo {
    protonsync_api::SettingsInfo {
        config_path: "/home/k/.config/protonsync/config.toml".into(),
        editable: true,
        dry_run: false,
        trash_retention_days: 30,
        pairs: vec![protonsync_api::PairSettings {
            id: "main".into(),
            direction: protonsync_api::Direction::BackupOnly,
            protect_remote: false,
            archive_versions: true,
        }],
    }
}

#[test]
fn config_show_and_set_send_the_right_requests() {
    let shown = run(
        &["config", "show"],
        vec![Response::Settings(settings_sample())],
        &[],
    );
    assert_eq!(shown.requests, [Request::Settings]);
    assert!(
        shown.out.contains("dry_run: false (aktiv)"),
        "{}",
        shown.out
    );
    assert!(
        shown.out.contains(
            "Paar main: direction = backup-only (nur Sicherung), protect_remote = nein, archive_versions = ja"
        ),
        "{}",
        shown.out
    );

    let set = run(
        &[
            "config",
            "set",
            "--dry-run",
            "false",
            "--pair",
            "main",
            "--direction",
            "backup-only",
            "--archive-versions",
            "true",
        ],
        vec![Response::Settings(settings_sample())],
        &[],
    );
    assert_eq!(set.code, exit::OK);
    assert_eq!(
        set.requests,
        [Request::ChangeSettings {
            change: protonsync_api::SettingsChange {
                dry_run: Some(false),
                trash_retention_days: None,
                pairs: vec![protonsync_api::PairSettingsChange {
                    direction: Some(protonsync_api::Direction::BackupOnly),
                    archive_versions: Some(true),
                    ..protonsync_api::PairSettingsChange::new("main")
                }],
            },
        }]
    );
}

#[test]
fn config_set_needs_something_to_change_and_a_pair_for_pair_options() {
    let nothing = run(&["config", "set", "--pair", "main"], vec![], &[]);
    assert_eq!(nothing.code, exit::USAGE);
    assert_eq!(nothing.requests, Vec::<Request>::new());
    assert!(
        Cli::try_parse_from(["protonsync", "config", "set", "--protect-remote", "true"]).is_err()
    );
}

#[test]
fn changes_command_sends_the_right_request() {
    let answer = Response::Changes {
        changes: vec![],
        keep_days: 30,
    };
    let r = run(&["changes"], vec![answer.clone()], &[]);
    assert_eq!(
        r.requests,
        [Request::Changes {
            since: None,
            pair: None,
            limit: commands::DEFAULT_CHANGES
        }]
    );
    assert_eq!(r.code, exit::OK);
    assert!(r.out.contains("Keine Änderungen"), "{}", r.out);

    let before = jiff::Timestamp::now().as_second();
    let r = run(
        &["changes", "--pair", "main", "--days", "1", "--limit", "5"],
        vec![answer],
        &[],
    );
    let [Request::Changes { since, pair, limit }] = r.requests.as_slice() else {
        panic!("unexpected: {:?}", r.requests);
    };
    let since = since.unwrap();
    assert!(
        (before - 86_400 - 5..=before - 86_400 + 5).contains(&since),
        "{since}"
    );
    assert_eq!((pair.as_deref(), *limit), (Some("main"), 5));
}
