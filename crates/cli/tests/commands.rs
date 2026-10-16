//! Befehle gegen einen geskripteten Daemon: richtige Anfragen, Ausgaben, Exit-Codes.

#![allow(clippy::unwrap_used)]

use std::collections::VecDeque;
use std::io;

use clap::Parser;
use protonsync_api::{
    ApiError, ErrorCode, LoginStep, PairSyncResult, Request, Response, StatusInfo, SyncOutcomeInfo,
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

#[test]
fn login_asks_for_everything_the_account_needs() {
    let answers = vec![
        step(true, true),
        step(false, true),
        Response::LoggedIn {
            username: "k".into(),
        },
    ];
    let result = run(&["login"], answers, &["k", "pw", "123456", "daten"]);
    assert_eq!(result.code, exit::OK);
    assert_eq!(result.out, "Angemeldet als k\n");
    assert!(
        matches!(&result.requests[0], Request::LoginStart { username, password } if username == "k" && password.expose() == "pw")
    );
    assert_eq!(
        result.requests[1],
        Request::LoginSecondFactor {
            code: "123456".into()
        }
    );
    assert!(
        matches!(&result.requests[2], Request::LoginFinish { data_password: Some(p) } if p.expose() == "daten")
    );
}

#[test]
fn login_without_second_factor_skips_questions() {
    let answers = vec![
        step(false, false),
        Response::LoggedIn {
            username: "k".into(),
        },
    ];
    let result = run(&["login", "--username", "k"], answers, &["pw"]);
    assert_eq!(result.code, exit::OK);
    assert_eq!(result.requests.len(), 2);
    assert_eq!(
        result.requests[1],
        Request::LoginFinish {
            data_password: None
        }
    );
}

#[test]
fn rejected_login_has_its_own_exit_code() {
    let answers = vec![Response::Error(ApiError::new(
        ErrorCode::LoginRejected,
        "falsch",
    ))];
    let result = run(&["login", "--username", "k"], answers, &["pw"]);
    assert_eq!(result.code, exit::LOGIN_REJECTED);
    assert_eq!(result.err, "falsch\n");
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
            message: "nicht da".into(),
        })
    };
    let code = commands::run(
        &cli,
        unreachable,
        &mut prompter,
        &mut out,
        &mut err,
        Lang::De,
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
    );
    assert_eq!(code, exit::OK);
    assert!(String::from_utf8(out).unwrap().contains("[[pair]]"));
}
