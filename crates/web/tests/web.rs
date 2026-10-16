//! Web-Oberfläche gegen einen geskripteten Daemon: Zugriffsschutz, Anmeldung,
//! CSRF, Darstellung und die Anfragen, die Aktionen auslösen.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::body::Body;
use axum::http::header::{CONTENT_SECURITY_POLICY, COOKIE, LOCATION, SET_COOKIE};
use axum::http::{Request as HttpRequest, StatusCode};
use protonsync_api::{
    ApiError, ConflictInfo, ConflictKindInfo, Direction, ErrorCode, JobDetails, JobInfo,
    JobOutcomeInfo, JobRunInfo, JobVerification, PairStatus, Request, ResolutionChoice, Response,
    StatusInfo, TrashInfo, TrashSide,
};
use protonsync_web::{Backend, ClientAddr, IpNetwork, PasswordCheck, WebConfig, hash_password};
use tower::ServiceExt;

const PASSWORD: &str = "richtig-geheim";
const HOME: &str = "192.168.1.20:50000";

fn password_hash() -> &'static str {
    // Argon2 ist absichtlich langsam; einmal je Testlauf reicht.
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| hash_password(PASSWORD).unwrap())
}

#[derive(Default)]
struct ScriptedDaemon {
    requests: Mutex<Vec<Request>>,
    fail_with: Mutex<Option<ApiError>>,
}

impl ScriptedDaemon {
    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

fn job_run(outcome: JobOutcomeInfo) -> JobRunInfo {
    JobRunInfo {
        job: "ha".into(),
        started_at: 1_760_000_000,
        finished_at: 1_760_000_060,
        outcome,
    }
}

impl Backend for ScriptedDaemon {
    fn handle(&self, request: Request) -> impl Future<Output = Response> + Send {
        std::future::ready(self.respond(&request))
    }
}

impl ScriptedDaemon {
    fn respond(&self, request: &Request) -> Response {
        self.requests.lock().unwrap().push(request.clone());
        if let Some(error) = self.fail_with.lock().unwrap().clone() {
            return Response::Error(error);
        }
        match request {
            Request::Status => Response::Status(StatusInfo {
                daemon_version: "0.1.0".into(),
                account: Some("<b>krischan</b>".into()),
                login_pending: false,
                dry_run: true,
                pairs: vec![PairStatus {
                    id: "main".into(),
                    local_root: "/backup".into(),
                    remote_path: "Backups".into(),
                    direction: Direction::TwoWay,
                    syncing: false,
                    last_sync: None,
                    open_conflicts: 1,
                    trash_entries: 1,
                }],
            }),
            Request::Jobs => Response::Jobs {
                jobs: vec![JobInfo {
                    id: "ha".into(),
                    source: "/mnt/ha".into(),
                    pattern: "*.tar".into(),
                    target: "Backups/HA".into(),
                    schedule: "0 4 * * *".into(),
                    keep_last: Some(14),
                    max_age_days: None,
                    verify: JobVerification::Hash,
                    running: false,
                    last_run: Some(job_run(JobOutcomeInfo::Failed {
                        message: "Netz weg".into(),
                        attempt: 2,
                    })),
                    next_run: Some(1_760_065_200),
                }],
            },
            Request::Conflicts => Response::Conflicts {
                conflicts: vec![ConflictInfo {
                    pair: "main".into(),
                    id: 7,
                    path: "Bericht.pdf".into(),
                    kind: ConflictKindInfo::BothModified,
                    conflict_copy: Some("Bericht (Konflikt 1).pdf".into()),
                    detected_at: 1_760_000_000,
                }],
            },
            Request::Trash | Request::PurgeTrash { .. } => Response::Trash {
                entries: vec![TrashInfo {
                    pair: "job:ha".into(),
                    id: 3,
                    path: "alt.tar".into(),
                    side: TrashSide::Remote,
                    trashed_at: 1_760_000_000,
                    expires_at: 1_762_592_000,
                }],
            },
            Request::RunJob { apply, .. } => Response::JobRun {
                run: job_run(if *apply {
                    JobOutcomeInfo::Succeeded {
                        uploaded: 1,
                        unchanged: 0,
                        expired: 1,
                    }
                } else {
                    JobOutcomeInfo::Planned {
                        uploads: 1,
                        expired: 1,
                    }
                }),
                details: JobDetails {
                    uploads: vec!["neu.tar".into()],
                    expired: vec!["alt.tar".into()],
                    ..JobDetails::default()
                },
            },
            Request::JobHistory { .. } => Response::JobHistory {
                runs: vec![job_run(JobOutcomeInfo::Planned {
                    uploads: 0,
                    expired: 0,
                })],
            },
            Request::Sync { .. } => Response::Sync { results: vec![] },
            _ => Response::Done,
        }
    }
}

fn config(tls: bool) -> WebConfig {
    WebConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        password: PasswordCheck::new(password_hash()).unwrap(),
        allowed: IpNetwork::private(),
        tls: tls.then(|| protonsync_web::TlsFiles {
            cert: "/x.pem".into(),
            key: "/x.key".into(),
        }),
        session_lifetime: Duration::from_hours(12),
        zone: jiff::tz::TimeZone::UTC,
    }
}

struct Client {
    app: axum::Router,
    daemon: Arc<ScriptedDaemon>,
    from: SocketAddr,
    cookies: Vec<String>,
}

struct Reply {
    status: StatusCode,
    location: Option<String>,
    set_cookie: Vec<String>,
    csp: bool,
    body: String,
}

impl Client {
    fn new() -> Self {
        let daemon = Arc::new(ScriptedDaemon::default());
        Self {
            app: protonsync_web::router(Arc::clone(&daemon), &config(false)),
            daemon,
            from: HOME.parse().unwrap(),
            cookies: Vec::new(),
        }
    }

    async fn send(&mut self, method: &str, uri: &str, form: Option<&str>) -> Reply {
        let mut request = HttpRequest::builder().method(method).uri(uri);
        if !self.cookies.is_empty() {
            request = request.header(COOKIE, self.cookies.join("; "));
        }
        let body = match form {
            Some(form) => {
                request = request.header("content-type", "application/x-www-form-urlencoded");
                Body::from(form.to_owned())
            }
            None => Body::empty(),
        };
        let mut request = request.body(body).unwrap();
        request.extensions_mut().insert(ClientAddr(self.from));
        let response = self.app.clone().oneshot(request).await.unwrap();
        let set_cookie: Vec<String> = response
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_owned())
            .collect();
        for cookie in &set_cookie {
            let pair = cookie.split(';').next().unwrap().to_owned();
            let name = pair.split('=').next().unwrap().to_owned();
            self.cookies.retain(|c| !c.starts_with(&format!("{name}=")));
            if !pair.ends_with('=') {
                self.cookies.push(pair);
            }
        }
        Reply {
            status: response.status(),
            location: response
                .headers()
                .get(LOCATION)
                .map(|v| v.to_str().unwrap().to_owned()),
            set_cookie,
            csp: response.headers().contains_key(CONTENT_SECURITY_POLICY),
            body: String::from_utf8(
                axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap()
                    .to_vec(),
            )
            .unwrap(),
        }
    }

    async fn get(&mut self, uri: &str) -> Reply {
        self.send("GET", uri, None).await
    }

    async fn post(&mut self, uri: &str, form: &str) -> Reply {
        self.send("POST", uri, Some(form)).await
    }

    async fn login(&mut self) -> String {
        let reply = self.post("/login", &format!("password={PASSWORD}")).await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER);
        let page = self.get("/").await;
        let start = page.body.find("name=\"csrf\" value=\"").unwrap() + 19;
        page.body[start..start + 64].to_owned()
    }
}

#[tokio::test]
async fn only_allowed_networks_get_in() {
    let mut client = Client::new();
    client.from = "8.8.8.8:4000".parse().unwrap();
    let reply = client.get("/login").await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    client.from = "[::1]:4000".parse().unwrap();
    assert_eq!(client.get("/login").await.status, StatusCode::OK);
}

#[tokio::test]
async fn pages_and_actions_need_a_login() {
    let mut client = Client::new();
    for uri in [
        "/",
        "/conflicts",
        "/trash",
        "/jobs",
        "/jobs/ha",
        "/settings",
    ] {
        let reply = client.get(uri).await;
        assert_eq!(
            (reply.status, reply.location.as_deref()),
            (StatusCode::SEE_OTHER, Some("/login")),
            "{uri}"
        );
    }
    let reply = client.post("/jobs/ha/run", "csrf=x&apply=1").await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(client.daemon.requests(), Vec::new(), "nichts an den Daemon");
    assert!(client.get("/login").await.csp, "Sicherheits-Header");
}

#[tokio::test]
async fn wrong_passwords_are_rejected_and_then_locked_out() {
    let mut client = Client::new();
    for _ in 0..5 {
        let reply = client.post("/login", "password=falsch").await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
        assert!(reply.body.contains("Wrong password"));
    }
    let reply = client.post("/login", &format!("password={PASSWORD}")).await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    // Andere Adressen sind nicht betroffen.
    client.from = "192.168.1.21:5000".parse().unwrap();
    assert_eq!(
        client
            .post("/login", &format!("password={PASSWORD}"))
            .await
            .status,
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn login_sets_a_protected_session_cookie_and_logout_ends_it() {
    let mut client = Client::new();
    let reply = client.post("/login", &format!("password={PASSWORD}")).await;
    let cookie = &reply.set_cookie[0];
    assert!(cookie.starts_with("protonsync_session="));
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
    assert!(!cookie.contains("Secure"), "ohne TLS kein Secure");
    assert_eq!(client.get("/login").await.location.as_deref(), Some("/"));

    let csrf = client.login().await;
    let reply = client.post("/logout", "csrf=falsch").await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let reply = client.post("/logout", &format!("csrf={csrf}")).await;
    assert_eq!(reply.location.as_deref(), Some("/login"));
    assert_eq!(client.get("/").await.location.as_deref(), Some("/login"));
}

#[tokio::test]
async fn tls_sessions_use_secure_cookies() {
    let daemon = Arc::new(ScriptedDaemon::default());
    let mut client = Client {
        app: protonsync_web::router(Arc::clone(&daemon), &config(true)),
        daemon,
        from: HOME.parse().unwrap(),
        cookies: Vec::new(),
    };
    let reply = client.post("/login", &format!("password={PASSWORD}")).await;
    assert!(reply.set_cookie[0].contains("; Secure"));
}

#[tokio::test]
async fn overview_shows_escaped_daemon_data() {
    let mut client = Client::new();
    client.login().await;
    let page = client.get("/").await.body;
    assert!(page.contains("&lt;b&gt;krischan&lt;/b&gt;"));
    assert!(!page.contains("<b>krischan</b>"));
    assert!(page.contains("Dry run active"));
    assert!(page.contains("FAILED (attempt 2): Netz weg"));
    assert!(page.contains("2025-10-10 03:00"), "nächster Lauf");
    assert!(page.contains("http-equiv=\"refresh\""));
}

#[tokio::test]
async fn forms_without_the_right_csrf_token_do_nothing() {
    let mut client = Client::new();
    client.login().await;
    let before = client.daemon.requests().len();
    let reply = client.post("/sync", "csrf=falsch&apply=1").await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(client.daemon.requests().len(), before);
}

#[tokio::test]
async fn actions_send_the_matching_requests() {
    let mut client = Client::new();
    let csrf = client.login().await;

    let reply = client
        .post(
            "/conflicts/resolve",
            &format!("csrf={csrf}&pair=main&id=7&resolution=keep_both"),
        )
        .await;
    assert_eq!(reply.location.as_deref(), Some("/conflicts"));
    let reply = client
        .post("/trash/restore", &format!("csrf={csrf}&pair=job%3Aha&id=3"))
        .await;
    assert_eq!(reply.location.as_deref(), Some("/trash"));
    let planned = client.post("/trash/purge", &format!("csrf={csrf}")).await;
    assert!(planned.body.contains("would be deleted"));
    let run = client
        .post("/jobs/ha/run", &format!("csrf={csrf}&apply=1"))
        .await;
    assert!(run.body.contains("uploaded: neu.tar"));
    client
        .post("/sync", &format!("csrf={csrf}&pair=main"))
        .await;

    let requests = client.daemon.requests();
    let actions: Vec<&Request> = requests
        .iter()
        .filter(|r| !matches!(r, Request::Status | Request::Jobs))
        .collect();
    assert_eq!(
        actions,
        [
            &Request::ResolveConflict {
                pair: "main".into(),
                id: 7,
                resolution: ResolutionChoice::KeepBoth
            },
            &Request::RestoreTrash {
                pair: "job:ha".into(),
                id: 3
            },
            &Request::PurgeTrash { apply: false },
            &Request::RunJob {
                job: "ha".into(),
                apply: true
            },
            &Request::Sync {
                pair: Some("main".into()),
                apply: false
            },
        ]
    );
}

#[tokio::test]
async fn list_pages_render() {
    let mut client = Client::new();
    client.login().await;
    let conflicts = client.get("/conflicts").await.body;
    assert!(conflicts.contains("Bericht (Konflikt 1).pdf") && conflicts.contains("keep_remote"));
    let trash = client.get("/trash").await.body;
    assert!(trash.contains("job:ha") && trash.contains("alt.tar"));
    let jobs = client.get("/jobs").await.body;
    assert!(jobs.contains("/mnt/ha/*.tar → Backups/HA") && jobs.contains("newest 14"));
    let history = client.get("/jobs/ha").await.body;
    assert!(history.contains("Dry run: 0 to upload"));
    let settings = client.get("/settings").await.body;
    assert!(settings.contains("config.toml") && settings.contains("0 4 * * *"));
    assert!(client.daemon.requests().contains(&Request::JobHistory {
        job: "ha".into(),
        limit: 50
    }));
}

#[tokio::test]
async fn daemon_errors_become_error_pages() {
    let mut client = Client::new();
    let csrf = client.login().await;
    *client.daemon.fail_with.lock().unwrap() =
        Some(ApiError::new(ErrorCode::NotLoggedIn, "Nicht angemeldet"));
    let reply = client.post("/jobs/ha/run", &format!("csrf={csrf}")).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert!(reply.body.contains("protonsync login"));
    *client.daemon.fail_with.lock().unwrap() =
        Some(ApiError::new(ErrorCode::NotFound, "Unbekannter Job x"));
    assert_eq!(client.get("/jobs/x").await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn language_can_be_switched_and_is_remembered() {
    let mut client = Client::new();
    let reply = client.get("/login?lang=de").await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    assert_eq!(reply.location.as_deref(), Some("/login"));
    let page = client.get("/login").await.body;
    assert!(page.contains("<html lang=\"de\">") && page.contains("Anmelden"));
    assert_eq!(client.get("/nicht-da").await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn server_answers_over_a_real_socket_and_reports_busy_ports() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut config = config(false);
    config.bind = busy.local_addr().unwrap();
    let (_stop, shutdown) = tokio::sync::watch::channel(false);
    let daemon = Arc::new(ScriptedDaemon::default());
    let error = protonsync_web::serve(config.clone(), Arc::clone(&daemon), shutdown.clone())
        .await
        .unwrap_err();
    assert!(error.to_string().contains(&config.bind.to_string()));
    drop(busy);

    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let server = tokio::spawn(protonsync_web::serve(config.clone(), daemon, shutdown));
    let mut stream = loop {
        if let Ok(stream) = tokio::net::TcpStream::connect(config.bind).await {
            break stream;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    stream
        .write_all(b"GET /login HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).await.unwrap();
    assert!(answer.starts_with("HTTP/1.1 200 OK"), "{answer}");
    stop.send(true).unwrap();
    server.await.unwrap().unwrap();
}
