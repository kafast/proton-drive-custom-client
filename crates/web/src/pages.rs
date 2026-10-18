//! Pages and actions of the interface. Every action is a request to the daemon API.

// The error path of the helper functions is a finished HTTP response that is
// returned right away; boxing it would only add hassle.
#![allow(clippy::result_large_err)]

use std::fmt::Write;
use std::sync::Arc;

use axum::extract::{Form, Path, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, LOCATION, REFERRER_POLICY, SET_COOKIE,
    X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use protonsync_api::{
    ApiError, ConflictInfo, ConflictKindInfo, Direction, ErrorCode, JobDetails, JobInfo,
    JobOutcomeInfo, JobRunInfo, PairSyncResult, Request, ResolutionChoice, Response as ApiResponse,
    StatusInfo, StepAction, SyncOutcomeInfo, TrashInfo, TrashSide,
};
use serde::Deserialize;

use crate::auth::constant_time_eq;
use crate::html::{Lang, Nav, button_form, escape, format_time, page};
use crate::visitor::{SESSION_COOKIE, Session, Visitor};
use crate::{AppState, Backend};

pub(crate) mod photos;

/// The result page shows at most this many planned steps.
const MAX_LISTED_STEPS: usize = 200;
/// The history of a job shows this many entries.
const HISTORY_LIMIT: u32 = 50;
/// The overview reloads itself at this interval (seconds).
const STATUS_REFRESH_SECONDS: u32 = 30;

const STYLE: &str = include_str!("../static/style.css");

type Shared<B> = State<Arc<AppState<B>>>;

// ---------------------------------------------------------------------------
// Frame, security, helpers

/// Security headers for every response.
pub(crate) async fn security_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; style-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'",
        ),
    );
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers
        .entry(CACHE_CONTROL)
        .or_insert(HeaderValue::from_static("no-store"));
    response
}

pub(crate) async fn style() -> Response {
    (
        [
            (CONTENT_TYPE, "text/css; charset=utf-8"),
            (CACHE_CONTROL, "max-age=3600"),
        ],
        STYLE,
    )
        .into_response()
}

pub(crate) async fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "404 Not Found\n").into_response()
}

fn see_other(location: &str) -> Response {
    let mut response = StatusCode::SEE_OTHER.into_response();
    if let Ok(value) = HeaderValue::from_str(location) {
        response.headers_mut().insert(LOCATION, value);
    }
    response
}

/// Logged in? Otherwise to the login page.
fn require_session(visitor: &Visitor) -> Result<&Session, Response> {
    visitor.session.as_ref().ok_or_else(|| see_other("/login"))
}

/// Logged in and with a matching CSRF token?
fn require_form_session<'v>(visitor: &'v Visitor, csrf: &str) -> Result<&'v Session, Response> {
    let session = require_session(visitor)?;
    if constant_time_eq(&session.csrf, csrf) {
        Ok(session)
    } else {
        Err((StatusCode::FORBIDDEN, "403 Forbidden (CSRF)\n").into_response())
    }
}

fn html_page(status: StatusCode, content: String) -> Response {
    (status, Html(content)).into_response()
}

fn logged_in_page(
    visitor: &Visitor,
    session: &Session,
    nav: Nav,
    title: &str,
    body: &str,
) -> Response {
    html_page(
        StatusCode::OK,
        page(visitor.lang, nav, title, Some(&session.csrf), body),
    )
}

fn error_page(visitor: &Visitor, session: &Session, error: &ApiError) -> Response {
    let lang = visitor.lang;
    let status = match error.code {
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        ErrorCode::InvalidRequest => StatusCode::BAD_REQUEST,
        ErrorCode::NotLoggedIn => StatusCode::CONFLICT,
        _ => StatusCode::BAD_GATEWAY,
    };
    let hint = if error.code == ErrorCode::NotLoggedIn {
        lang.pick(
            "<p>Der Daemon ist nicht bei Proton angemeldet. Auf dem Server <code>protonsync login</code> ausführen.</p>",
            "<p>The daemon is not logged in to Proton. Run <code>protonsync login</code> on the server.</p>",
        )
    } else {
        ""
    };
    let body = format!(
        "<p class=\"error\">{}</p>{hint}<p><a href=\"/\">{}</a></p>",
        escape(&error.message),
        lang.pick("Zur Übersicht", "Back to the overview")
    );
    html_page(
        status,
        page(
            lang,
            Nav::None,
            lang.pick("Fehler", "Error"),
            Some(&session.csrf),
            &body,
        ),
    )
}

/// Sends a request to the daemon; error responses become the error page.
async fn ask<B: Backend>(
    state: &AppState<B>,
    visitor: &Visitor,
    session: &Session,
    request: Request,
) -> Result<ApiResponse, Response> {
    match state.backend.handle(request).await {
        ApiResponse::Error(error) => Err(error_page(visitor, session, &error)),
        response => Ok(response),
    }
}

fn unexpected(visitor: &Visitor, session: &Session, response: &ApiResponse) -> Response {
    error_page(
        visitor,
        session,
        &ApiError::new(
            ErrorCode::Internal,
            format!("Unerwartete Antwort / unexpected response: {response:?}"),
        ),
    )
}

// ---------------------------------------------------------------------------
// Login

#[derive(Deserialize)]
pub(crate) struct LoginForm {
    password: String,
}

#[derive(Deserialize)]
pub(crate) struct CsrfForm {
    csrf: String,
}

fn login_page(lang: Lang, status: StatusCode, message: Option<&str>) -> Response {
    let message = message.map_or_else(String::new, |m| {
        format!("<p class=\"error\">{}</p>", escape(m))
    });
    let body = format!(
        "{message}<form method=\"post\" action=\"/login\" class=\"login\">\
         <label>{} <input type=\"password\" name=\"password\" autocomplete=\"current-password\" required autofocus></label>\
         <button>{}</button></form>",
        lang.pick("Passwort der Web-Oberfläche", "Web interface password"),
        lang.pick("Anmelden", "Log in")
    );
    html_page(
        status,
        page(
            lang,
            Nav::None,
            lang.pick("Anmelden", "Log in"),
            None,
            &body,
        ),
    )
}

// `B` determines for which state the `Visitor` is read.
#[allow(clippy::extra_unused_type_parameters)]
pub(crate) async fn login_form<B: Backend>(visitor: Visitor) -> Response {
    if visitor.session.is_some() {
        return see_other("/");
    }
    login_page(visitor.lang, StatusCode::OK, None)
}

pub(crate) async fn login<B: Backend>(
    State(state): Shared<B>,
    visitor: Visitor,
    Form(form): Form<LoginForm>,
) -> Response {
    let lang = visitor.lang;
    if !state.guard.allowed(visitor.ip) {
        return login_page(
            lang,
            StatusCode::TOO_MANY_REQUESTS,
            Some(lang.pick(
                "Zu viele Fehlversuche. Bitte in einigen Minuten erneut versuchen.",
                "Too many failed attempts. Please try again in a few minutes.",
            )),
        );
    }
    if !state.password.verify(&form.password) {
        state.guard.failed(visitor.ip);
        tracing::warn!(ip = %visitor.ip, "failed login to the web interface");
        return login_page(
            lang,
            StatusCode::UNAUTHORIZED,
            Some(lang.pick("Falsches Passwort.", "Wrong password.")),
        );
    }
    state.guard.succeeded(visitor.ip);
    let Ok(token) = state.sessions.create() else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let secure = if state.secure_cookies { "; Secure" } else { "" };
    let cookie = format!(
        "{SESSION_COOKIE}={token}; Path=/; Max-Age={}; HttpOnly; SameSite=Strict{secure}",
        state.sessions.lifetime().as_secs()
    );
    let mut response = see_other("/");
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(SET_COOKIE, value);
    }
    response
}

pub(crate) async fn logout<B: Backend>(
    State(state): Shared<B>,
    visitor: Visitor,
    Form(form): Form<CsrfForm>,
) -> Response {
    let session = match require_form_session(&visitor, &form.csrf) {
        Ok(session) => session,
        Err(response) => return response,
    };
    state.sessions.remove(&session.token);
    let mut response = see_other("/login");
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_static(
            "protonsync_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict",
        ),
    );
    response
}

// ---------------------------------------------------------------------------
// Overview and sync

fn direction(lang: Lang, direction: Direction) -> &'static str {
    match direction {
        Direction::TwoWay => lang.pick("zwei Wege", "two-way"),
        Direction::DownloadOnly => lang.pick("nur herunterladen", "download only"),
        Direction::UploadOnly => lang.pick("nur hochladen", "upload only"),
    }
}

fn sync_outcome(lang: Lang, outcome: &SyncOutcomeInfo) -> String {
    match outcome {
        SyncOutcomeInfo::Planned { steps, skipped } => format!(
            "{}: {} {}, {} {}",
            lang.pick("Probelauf", "Dry run"),
            steps.len(),
            lang.pick("Schritte", "steps"),
            skipped.len(),
            lang.pick("übersprungen", "skipped")
        ),
        SyncOutcomeInfo::Applied(summary) => {
            let mut text = format!(
                "{} {}, {} {}, {} {}, {} {}",
                summary.uploaded,
                lang.pick("hoch", "up"),
                summary.downloaded,
                lang.pick("herunter", "down"),
                summary.trashed_remote + summary.trashed_local,
                lang.pick("Papierkorb", "trashed"),
                summary.conflicts,
                lang.pick("Konflikte", "conflicts")
            );
            if !summary.failures.is_empty() {
                let _ = write!(
                    text,
                    ", {} {}",
                    summary.failures.len(),
                    lang.pick("Fehler", "errors")
                );
            }
            text
        }
        SyncOutcomeInfo::Failed { message } => {
            format!("{}: {message}", lang.pick("FEHLER", "FAILED"))
        }
    }
}

fn job_outcome(lang: Lang, outcome: &JobOutcomeInfo) -> String {
    match outcome {
        JobOutcomeInfo::Planned { uploads, expired } => format!(
            "{}: {uploads} {}, {expired} {}",
            lang.pick("Probelauf", "Dry run"),
            lang.pick("hochzuladen", "to upload"),
            lang.pick("abgelaufen", "expired")
        ),
        JobOutcomeInfo::Succeeded {
            uploaded,
            unchanged,
            expired,
        } => format!(
            "{}: {uploaded} {}, {unchanged} {}, {expired} {}",
            lang.pick("OK", "OK"),
            lang.pick("hochgeladen", "uploaded"),
            lang.pick("unverändert", "unchanged"),
            lang.pick("in den Papierkorb", "to trash")
        ),
        JobOutcomeInfo::Failed { message, attempt } => match lang {
            Lang::De => format!("FEHLER ({attempt}. Versuch): {message}"),
            Lang::En => format!("FAILED (attempt {attempt}): {message}"),
        },
    }
}

fn status_class(failed: bool) -> &'static str {
    if failed { " class=\"bad\"" } else { "" }
}

pub(crate) async fn status<B: Backend>(State(state): Shared<B>, visitor: Visitor) -> Response {
    let session = match require_session(&visitor) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let lang = visitor.lang;
    let status = match ask(&state, &visitor, session, Request::Status).await {
        Ok(ApiResponse::Status(status)) => status,
        Ok(other) => return unexpected(&visitor, session, &other),
        Err(response) => return response,
    };
    let jobs = match ask(&state, &visitor, session, Request::Jobs).await {
        Ok(ApiResponse::Jobs { jobs }) => jobs,
        Ok(other) => return unexpected(&visitor, session, &other),
        Err(response) => return response,
    };
    let mut body = account_section(lang, &status);
    body.push_str(&pairs_section(&state, lang, &session.csrf, &status));
    if !jobs.is_empty() {
        let _ = write!(
            body,
            "<h2>{}</h2><table><tr><th>Job</th><th>{}</th><th>{}</th></tr>",
            lang.pick("Upload-Jobs", "Upload jobs"),
            lang.pick("Letzter Lauf", "Last run"),
            lang.pick("Nächster Lauf", "Next run")
        );
        for job in &jobs {
            let _ = write!(
                body,
                "<tr><td><a href=\"/jobs/{0}\">{0}</a></td>{1}<td>{2}</td></tr>",
                escape(&job.id),
                last_job_run_cell(&state, lang, job),
                job.next_run
                    .map_or_else(String::new, |at| format_time(at, &state.zone)),
            );
        }
        body.push_str("</table>");
    }
    let content = page(lang, Nav::Status, lang.pick("Übersicht", "Overview"), Some(&session.csrf), &body)
        .replacen(
            "<meta charset=\"utf-8\">",
            &format!("<meta charset=\"utf-8\"><meta http-equiv=\"refresh\" content=\"{STATUS_REFRESH_SECONDS}\">"),
            1,
        );
    html_page(StatusCode::OK, content)
}

fn account_section(lang: Lang, status: &StatusInfo) -> String {
    let mut body = String::from("<section class=\"summary\">");
    match &status.account {
        Some(account) => {
            let _ = write!(
                body,
                "<p>{}: <strong>{}</strong></p>",
                lang.pick("Konto", "Account"),
                escape(account)
            );
        }
        None => {
            let _ = write!(
                body,
                "<p class=\"bad\">{}</p>",
                lang.pick(
                    "Nicht bei Proton angemeldet. Auf dem Server <code>protonsync login</code> ausführen.",
                    "Not logged in to Proton. Run <code>protonsync login</code> on the server."
                )
            );
        }
    }
    if status.dry_run {
        let _ = write!(
            body,
            "<p class=\"note\">{}</p>",
            lang.pick(
                "Probelauf aktiv: Automatische Abgleiche und Jobs planen nur (<code>dry_run = false</code> schaltet ab).",
                "Dry run active: automatic syncs and jobs only plan (set <code>dry_run = false</code> to change)."
            )
        );
    }
    let _ = write!(
        body,
        "<p class=\"muted\">protonsyncd {}</p></section>",
        escape(&status.daemon_version)
    );
    body
}

fn pairs_section<B>(state: &AppState<B>, lang: Lang, csrf: &str, status: &StatusInfo) -> String {
    let mut body = format!("<h2>{}</h2>", lang.pick("Sync-Paare", "Sync pairs"));
    if status.pairs.is_empty() {
        let _ = write!(
            body,
            "<p>{}</p>",
            lang.pick(
                "Keine Sync-Paare konfiguriert.",
                "No sync pairs configured."
            )
        );
        return body;
    }
    let _ = write!(
        body,
        "<table><tr><th>{}</th><th>{}</th><th>Proton</th><th>{}</th><th>{}</th><th>{}</th><th></th></tr>",
        lang.pick("Paar", "Pair"),
        lang.pick("Lokal", "Local"),
        lang.pick("Letzter Abgleich", "Last sync"),
        lang.pick("Konflikte", "Conflicts"),
        lang.pick("Papierkorb", "Trash"),
    );
    for pair in &status.pairs {
        let last = pair.last_sync.as_ref().map_or_else(
            || (false, lang.pick("noch keiner", "none yet").to_owned()),
            |result| last_sync_text(state, lang, result),
        );
        let remote = if pair.remote_path.is_empty() {
            "/"
        } else {
            &pair.remote_path
        };
        let running = if pair.syncing {
            lang.pick(" (läuft)", " (running)")
        } else {
            ""
        };
        let _ = write!(
            body,
            "<tr><td>{}{running}</td><td>{}</td><td>{} ({})</td><td{}>{}</td><td>{}</td><td>{}</td><td>{}{}</td></tr>",
            escape(&pair.id),
            escape(&pair.local_root),
            escape(remote),
            direction(lang, pair.direction),
            status_class(last.0),
            escape(&last.1),
            pair.open_conflicts,
            pair.trash_entries,
            button_form(
                "/sync",
                csrf,
                &[("pair", &pair.id)],
                lang.pick("Planen", "Plan")
            ),
            button_form(
                "/sync",
                csrf,
                &[("pair", &pair.id), ("apply", "1")],
                lang.pick("Abgleichen", "Sync now")
            ),
        );
    }
    let _ = write!(
        body,
        "</table><p>{}{}</p>",
        button_form("/sync", csrf, &[], lang.pick("Alle planen", "Plan all")),
        button_form(
            "/sync",
            csrf,
            &[("apply", "1")],
            lang.pick("Alle abgleichen", "Sync all")
        ),
    );
    body
}

fn last_sync_text<B>(state: &AppState<B>, lang: Lang, result: &PairSyncResult) -> (bool, String) {
    let failed = match &result.outcome {
        SyncOutcomeInfo::Failed { .. } => true,
        SyncOutcomeInfo::Applied(summary) => !summary.failures.is_empty(),
        SyncOutcomeInfo::Planned { .. } => false,
    };
    (
        failed,
        format!(
            "{} {}",
            format_time(result.finished_at, &state.zone),
            sync_outcome(lang, &result.outcome)
        ),
    )
}

fn last_job_run_cell<B>(state: &AppState<B>, lang: Lang, job: &JobInfo) -> String {
    match &job.last_run {
        None => format!("<td>{}</td>", lang.pick("noch keiner", "none yet")),
        Some(run) => format!(
            "<td{}>{} {}</td>",
            status_class(matches!(run.outcome, JobOutcomeInfo::Failed { .. })),
            format_time(run.finished_at, &state.zone),
            escape(&job_outcome(lang, &run.outcome))
        ),
    }
}

#[derive(Deserialize)]
pub(crate) struct SyncForm {
    csrf: String,
    #[serde(default)]
    pair: Option<String>,
    #[serde(default)]
    apply: Option<String>,
}

fn step_action(lang: Lang, action: StepAction) -> &'static str {
    match action {
        StepAction::Upload => lang.pick("hochladen", "upload"),
        StepAction::Download => lang.pick("herunterladen", "download"),
        StepAction::TrashRemote => lang.pick("in Proton-Papierkorb", "to Proton trash"),
        StepAction::TrashLocal => lang.pick("in lokalen Papierkorb", "to local trash"),
        StepAction::Conflict => lang.pick("Konflikt (beide behalten)", "conflict (keep both)"),
        StepAction::IgnoreLocalChange => {
            lang.pick("lokale Änderung bleibt lokal", "local change stays local")
        }
        StepAction::IgnoreRemoteChange => lang.pick(
            "entfernte Änderung bleibt entfernt",
            "remote change stays remote",
        ),
        StepAction::RecordConverged => lang.pick("bereits gleich", "already identical"),
    }
}

pub(crate) async fn sync<B: Backend>(
    State(state): Shared<B>,
    visitor: Visitor,
    Form(form): Form<SyncForm>,
) -> Response {
    let session = match require_form_session(&visitor, &form.csrf) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let lang = visitor.lang;
    let request = Request::Sync {
        pair: form.pair.filter(|p| !p.is_empty()),
        apply: form.apply.is_some(),
    };
    let results = match ask(&state, &visitor, session, request).await {
        Ok(ApiResponse::Sync { results }) => results,
        Ok(other) => return unexpected(&visitor, session, &other),
        Err(response) => return response,
    };
    let mut body = String::new();
    for result in &results {
        let (failed, text) = last_sync_text(&state, lang, result);
        let _ = write!(
            body,
            "<h2>{}</h2><p{}>{}</p>",
            escape(&result.pair),
            status_class(failed),
            escape(&text)
        );
        match &result.outcome {
            SyncOutcomeInfo::Planned { steps, skipped } => {
                body.push_str("<ul>");
                for step in steps.iter().take(MAX_LISTED_STEPS) {
                    let _ = write!(
                        body,
                        "<li>{}: {}</li>",
                        step_action(lang, step.action),
                        escape(&step.path)
                    );
                }
                if steps.len() > MAX_LISTED_STEPS {
                    let _ = write!(
                        body,
                        "<li>… {} {}</li>",
                        steps.len() - MAX_LISTED_STEPS,
                        lang.pick("weitere", "more")
                    );
                }
                for skip in skipped {
                    let _ = write!(
                        body,
                        "<li>{}: {} ({})</li>",
                        lang.pick("übersprungen", "skipped"),
                        escape(&skip.path),
                        escape(&skip.reason)
                    );
                }
                body.push_str("</ul>");
            }
            SyncOutcomeInfo::Applied(summary) => {
                body.push_str("<ul>");
                for failure in &summary.failures {
                    let _ = write!(
                        body,
                        "<li class=\"bad\">{}: {}</li>",
                        escape(&failure.path),
                        escape(&failure.message)
                    );
                }
                body.push_str("</ul>");
            }
            SyncOutcomeInfo::Failed { .. } => {}
        }
    }
    let _ = write!(
        body,
        "<p><a href=\"/\">{}</a></p>",
        lang.pick("Zur Übersicht", "Back to the overview")
    );
    logged_in_page(
        &visitor,
        session,
        Nav::Status,
        lang.pick("Abgleich", "Sync"),
        &body,
    )
}

// ---------------------------------------------------------------------------
// Conflicts

fn conflict_kind(lang: Lang, kind: ConflictKindInfo) -> &'static str {
    match kind {
        ConflictKindInfo::BothModified => {
            lang.pick("beide Seiten geändert", "changed on both sides")
        }
        ConflictKindInfo::LocalModifiedRemoteDeleted => lang.pick(
            "lokal geändert, in Proton gelöscht",
            "changed locally, deleted in Proton",
        ),
        ConflictKindInfo::LocalDeletedRemoteModified => lang.pick(
            "lokal gelöscht, in Proton geändert",
            "deleted locally, changed in Proton",
        ),
    }
}

pub(crate) async fn conflicts<B: Backend>(State(state): Shared<B>, visitor: Visitor) -> Response {
    let session = match require_session(&visitor) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let lang = visitor.lang;
    let conflicts: Vec<ConflictInfo> =
        match ask(&state, &visitor, session, Request::Conflicts).await {
            Ok(ApiResponse::Conflicts { conflicts }) => conflicts,
            Ok(other) => return unexpected(&visitor, session, &other),
            Err(response) => return response,
        };
    let mut body = String::new();
    if conflicts.is_empty() {
        let _ = write!(
            body,
            "<p>{}</p>",
            lang.pick("Keine offenen Konflikte.", "No open conflicts.")
        );
    } else {
        let _ = write!(
            body,
            "<p>{}</p><table><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th></th></tr>",
            lang.pick(
                "Bei „beide behalten“ bleibt die Konfliktkopie als eigene Datei; sonst kommt die verworfene Version in den Papierkorb.",
                "With \"keep both\" the conflict copy stays as its own file; otherwise the discarded version goes to the trash."
            ),
            lang.pick("Datei", "File"),
            lang.pick("Art", "Kind"),
            lang.pick("Konfliktkopie", "Conflict copy"),
            lang.pick("Erkannt", "Detected"),
        );
        for conflict in &conflicts {
            let id = conflict.id.to_string();
            let fields = |choice: &'static str| {
                [
                    ("pair", conflict.pair.as_str()),
                    ("id", id.as_str()),
                    ("resolution", choice),
                ]
            };
            let _ = write!(
                body,
                "<tr><td>[{}] {}</td><td>{}</td><td>{}</td><td>{}</td><td>{}{}{}</td></tr>",
                escape(&conflict.pair),
                escape(&conflict.path),
                conflict_kind(lang, conflict.kind),
                escape(conflict.conflict_copy.as_deref().unwrap_or("–")),
                format_time(conflict.detected_at, &state.zone),
                button_form(
                    "/conflicts/resolve",
                    &session.csrf,
                    &fields("keep_local"),
                    lang.pick("Lokal behalten", "Keep local")
                ),
                button_form(
                    "/conflicts/resolve",
                    &session.csrf,
                    &fields("keep_remote"),
                    lang.pick("Proton behalten", "Keep Proton")
                ),
                button_form(
                    "/conflicts/resolve",
                    &session.csrf,
                    &fields("keep_both"),
                    lang.pick("Beide behalten", "Keep both")
                ),
            );
        }
        body.push_str("</table>");
    }
    logged_in_page(
        &visitor,
        session,
        Nav::Conflicts,
        lang.pick("Konflikte", "Conflicts"),
        &body,
    )
}

#[derive(Deserialize)]
pub(crate) struct ResolveForm {
    csrf: String,
    pair: String,
    id: i64,
    resolution: ResolutionChoice,
}

pub(crate) async fn resolve<B: Backend>(
    State(state): Shared<B>,
    visitor: Visitor,
    Form(form): Form<ResolveForm>,
) -> Response {
    let session = match require_form_session(&visitor, &form.csrf) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let request = Request::ResolveConflict {
        pair: form.pair,
        id: form.id,
        resolution: form.resolution,
    };
    match ask(&state, &visitor, session, request).await {
        Ok(_) => see_other("/conflicts"),
        Err(response) => response,
    }
}

// ---------------------------------------------------------------------------
// Trash

fn trash_table<B>(
    state: &AppState<B>,
    lang: Lang,
    csrf: &str,
    entries: &[TrashInfo],
    restore: bool,
) -> String {
    let mut body = format!(
        "<table><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th>{}</tr>",
        lang.pick("Besitzer", "Owner"),
        lang.pick("Pfad", "Path"),
        lang.pick("Wo", "Where"),
        lang.pick("Gelöscht", "Deleted"),
        lang.pick("Endgültig ab", "Purged from"),
        if restore { "<th></th>" } else { "" },
    );
    for entry in entries {
        let id = entry.id.to_string();
        let action = if restore {
            format!(
                "<td>{}</td>",
                button_form(
                    "/trash/restore",
                    csrf,
                    &[("pair", &entry.pair), ("id", &id)],
                    lang.pick("Wiederherstellen", "Restore")
                )
            )
        } else {
            String::new()
        };
        let _ = write!(
            body,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td>{action}</tr>",
            escape(&entry.pair),
            escape(&entry.path),
            match entry.side {
                TrashSide::Local => lang.pick("lokal", "local"),
                TrashSide::Remote => "Proton",
            },
            format_time(entry.trashed_at, &state.zone),
            format_time(entry.expires_at, &state.zone),
        );
    }
    body.push_str("</table>");
    body
}

pub(crate) async fn trash<B: Backend>(State(state): Shared<B>, visitor: Visitor) -> Response {
    let session = match require_session(&visitor) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let lang = visitor.lang;
    let entries = match ask(&state, &visitor, session, Request::Trash).await {
        Ok(ApiResponse::Trash { entries }) => entries,
        Ok(other) => return unexpected(&visitor, session, &other),
        Err(response) => return response,
    };
    let body = if entries.is_empty() {
        format!(
            "<p>{}</p>",
            lang.pick("Der Papierkorb ist leer.", "The trash is empty.")
        )
    } else {
        format!(
            "{}<p>{}</p>",
            trash_table(&state, lang, &session.csrf, &entries, true),
            button_form(
                "/trash/purge",
                &session.csrf,
                &[],
                lang.pick("Abgelaufene anzeigen", "Show expired")
            ),
        )
    };
    logged_in_page(
        &visitor,
        session,
        Nav::Trash,
        lang.pick("Papierkorb", "Trash"),
        &body,
    )
}

#[derive(Deserialize)]
pub(crate) struct RestoreForm {
    csrf: String,
    pair: String,
    id: i64,
}

pub(crate) async fn restore<B: Backend>(
    State(state): Shared<B>,
    visitor: Visitor,
    Form(form): Form<RestoreForm>,
) -> Response {
    let session = match require_form_session(&visitor, &form.csrf) {
        Ok(session) => session,
        Err(response) => return response,
    };
    match ask(
        &state,
        &visitor,
        session,
        Request::RestoreTrash {
            pair: form.pair,
            id: form.id,
        },
    )
    .await
    {
        Ok(_) => see_other("/trash"),
        Err(response) => response,
    }
}

#[derive(Deserialize)]
pub(crate) struct ApplyForm {
    csrf: String,
    #[serde(default)]
    apply: Option<String>,
}

/// Without `apply` only show what has expired; with `apply` delete it permanently.
pub(crate) async fn purge<B: Backend>(
    State(state): Shared<B>,
    visitor: Visitor,
    Form(form): Form<ApplyForm>,
) -> Response {
    let session = match require_form_session(&visitor, &form.csrf) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let lang = visitor.lang;
    let apply = form.apply.is_some();
    let entries = match ask(&state, &visitor, session, Request::PurgeTrash { apply }).await {
        Ok(ApiResponse::Trash { entries }) => entries,
        Ok(other) => return unexpected(&visitor, session, &other),
        Err(response) => return response,
    };
    let mut body = String::new();
    if entries.is_empty() {
        let _ = write!(
            body,
            "<p>{}</p>",
            lang.pick("Nichts abgelaufen.", "Nothing expired.")
        );
    } else {
        let _ = write!(
            body,
            "<p>{}</p>{}",
            if apply {
                lang.pick("Endgültig gelöscht:", "Deleted for good:")
            } else {
                lang.pick(
                    "Diese Einträge sind abgelaufen und würden endgültig gelöscht:",
                    "These entries have expired and would be deleted for good:",
                )
            },
            trash_table(&state, lang, &session.csrf, &entries, false)
        );
        if !apply {
            let _ = write!(
                body,
                "<p>{}</p>",
                button_form(
                    "/trash/purge",
                    &session.csrf,
                    &[("apply", "1")],
                    lang.pick("Jetzt endgültig löschen", "Delete for good now")
                )
            );
        }
    }
    let _ = write!(
        body,
        "<p><a href=\"/trash\">{}</a></p>",
        lang.pick("Zum Papierkorb", "Back to the trash")
    );
    logged_in_page(
        &visitor,
        session,
        Nav::Trash,
        lang.pick("Abgelaufene Einträge", "Expired entries"),
        &body,
    )
}

// ---------------------------------------------------------------------------
// Upload jobs

fn retention(lang: Lang, job: &JobInfo) -> String {
    let mut parts = Vec::new();
    if let Some(keep) = job.keep_last {
        parts.push(format!("{} {keep}", lang.pick("die neuesten", "newest")));
    }
    if let Some(days) = job.max_age_days {
        parts.push(format!(
            "{} {days} {}",
            lang.pick("höchstens", "at most"),
            lang.pick("Tage", "days")
        ));
    }
    if parts.is_empty() {
        lang.pick("alles behalten", "keep everything").to_owned()
    } else {
        parts.join(lang.pick(" und ", " and "))
    }
}

fn schedule(lang: Lang, job: &JobInfo) -> String {
    if job.schedule == "on_new_file" {
        lang.pick("bei neuer Datei", "on new file").to_owned()
    } else {
        format!("cron {}", job.schedule)
    }
}

async fn load_jobs<B: Backend>(
    state: &AppState<B>,
    visitor: &Visitor,
    session: &Session,
) -> Result<Vec<JobInfo>, Response> {
    match ask(state, visitor, session, Request::Jobs).await? {
        ApiResponse::Jobs { jobs } => Ok(jobs),
        other => Err(unexpected(visitor, session, &other)),
    }
}

pub(crate) async fn jobs<B: Backend>(State(state): Shared<B>, visitor: Visitor) -> Response {
    let session = match require_session(&visitor) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let lang = visitor.lang;
    let jobs = match load_jobs(&state, &visitor, session).await {
        Ok(jobs) => jobs,
        Err(response) => return response,
    };
    let mut body = String::new();
    if jobs.is_empty() {
        let _ = write!(body, "<p>{}</p>", lang.pick(
            "Keine Upload-Jobs konfiguriert (Abschnitt <code>[[job]]</code> in der Konfiguration).",
            "No upload jobs configured (section <code>[[job]]</code> in the configuration)."));
    } else {
        let _ = write!(
            body,
            "<table><tr><th>Job</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th></th></tr>",
            lang.pick("Quelle → Ziel", "Source → target"),
            lang.pick("Zeitplan", "Schedule"),
            lang.pick("Aufbewahrung", "Retention"),
            lang.pick("Letzter Lauf", "Last run"),
            lang.pick("Nächster Lauf", "Next run"),
        );
        for job in &jobs {
            let action = format!("/jobs/{}/run", job.id);
            let running = if job.running {
                lang.pick(" (läuft)", " (running)")
            } else {
                ""
            };
            let _ = write!(
                body,
                "<tr><td><a href=\"/jobs/{0}\">{0}</a>{running}</td><td>{1}/{2} → {3}</td><td>{4}</td><td>{5}</td>{6}<td>{7}</td><td>{8}{9}</td></tr>",
                escape(&job.id),
                escape(&job.source),
                escape(&job.pattern),
                escape(&job.target),
                escape(&schedule(lang, job)),
                escape(&retention(lang, job)),
                last_job_run_cell(&state, lang, job),
                job.next_run
                    .map_or_else(String::new, |at| format_time(at, &state.zone)),
                button_form(&action, &session.csrf, &[], lang.pick("Planen", "Plan")),
                button_form(
                    &action,
                    &session.csrf,
                    &[("apply", "1")],
                    lang.pick("Jetzt ausführen", "Run now")
                ),
            );
        }
        body.push_str("</table>");
    }
    logged_in_page(
        &visitor,
        session,
        Nav::Jobs,
        lang.pick("Upload-Jobs", "Upload jobs"),
        &body,
    )
}

fn runs_table<B>(state: &AppState<B>, lang: Lang, runs: &[JobRunInfo]) -> String {
    if runs.is_empty() {
        return format!("<p>{}</p>", lang.pick("Noch keine Läufe.", "No runs yet."));
    }
    let mut body = format!(
        "<table><tr><th>{}</th><th>{}</th><th>{}</th></tr>",
        lang.pick("Beginn", "Started"),
        lang.pick("Ende", "Finished"),
        lang.pick("Ergebnis", "Result"),
    );
    for run in runs {
        let _ = write!(
            body,
            "<tr><td>{}</td><td>{}</td><td{}>{}</td></tr>",
            format_time(run.started_at, &state.zone),
            format_time(run.finished_at, &state.zone),
            status_class(matches!(run.outcome, JobOutcomeInfo::Failed { .. })),
            escape(&job_outcome(lang, &run.outcome)),
        );
    }
    body.push_str("</table>");
    body
}

pub(crate) async fn job_history<B: Backend>(
    State(state): Shared<B>,
    visitor: Visitor,
    Path(id): Path<String>,
) -> Response {
    let session = match require_session(&visitor) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let lang = visitor.lang;
    let runs = match ask(
        &state,
        &visitor,
        session,
        Request::JobHistory {
            job: id.clone(),
            limit: HISTORY_LIMIT,
        },
    )
    .await
    {
        Ok(ApiResponse::JobHistory { runs }) => runs,
        Ok(other) => return unexpected(&visitor, session, &other),
        Err(response) => return response,
    };
    let action = format!("/jobs/{id}/run");
    let body = format!(
        "<p>{}{}</p>{}<p><a href=\"/jobs\">{}</a></p>",
        button_form(&action, &session.csrf, &[], lang.pick("Planen", "Plan")),
        button_form(
            &action,
            &session.csrf,
            &[("apply", "1")],
            lang.pick("Jetzt ausführen", "Run now")
        ),
        runs_table(&state, lang, &runs),
        lang.pick("Alle Upload-Jobs", "All upload jobs"),
    );
    let title = format!("{}: {id}", lang.pick("Verlauf", "History"));
    logged_in_page(&visitor, session, Nav::Jobs, &title, &body)
}

fn details_list(lang: Lang, planned: bool, details: &JobDetails) -> String {
    let mut body = String::from("<ul>");
    let mut list = |label: &str, names: &[String]| {
        for name in names {
            let _ = write!(body, "<li>{label}: {}</li>", escape(name));
        }
    };
    list(
        if planned {
            lang.pick("hochladen", "upload")
        } else {
            lang.pick("hochgeladen", "uploaded")
        },
        &details.uploads,
    );
    list(
        lang.pick("wartet (Ruhezeit)", "waiting (settle time)"),
        &details.waiting,
    );
    list(
        if planned {
            lang.pick("in den Papierkorb", "to trash")
        } else {
            lang.pick("im Papierkorb", "trashed")
        },
        &details.expired,
    );
    for failure in &details.failures {
        let _ = write!(
            body,
            "<li class=\"bad\">{}: {}</li>",
            escape(&failure.path),
            escape(&failure.message)
        );
    }
    body.push_str("</ul>");
    body
}

pub(crate) async fn run_job<B: Backend>(
    State(state): Shared<B>,
    visitor: Visitor,
    Path(id): Path<String>,
    Form(form): Form<ApplyForm>,
) -> Response {
    let session = match require_form_session(&visitor, &form.csrf) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let lang = visitor.lang;
    let request = Request::RunJob {
        job: id.clone(),
        apply: form.apply.is_some(),
    };
    let (run, details) = match ask(&state, &visitor, session, request).await {
        Ok(ApiResponse::JobRun { run, details }) => (run, details),
        Ok(other) => return unexpected(&visitor, session, &other),
        Err(response) => return response,
    };
    let planned = matches!(run.outcome, JobOutcomeInfo::Planned { .. });
    let mut body = format!(
        "<p{}>{}</p>{}",
        status_class(matches!(run.outcome, JobOutcomeInfo::Failed { .. })),
        escape(&job_outcome(lang, &run.outcome)),
        details_list(lang, planned, &details)
    );
    if planned {
        let _ = write!(
            body,
            "<p>{}</p>",
            button_form(
                &format!("/jobs/{id}/run"),
                &session.csrf,
                &[("apply", "1")],
                lang.pick("Jetzt ausführen", "Run now")
            )
        );
    }
    let _ = write!(
        body,
        "<p><a href=\"/jobs/{}\">{}</a></p>",
        escape(&id),
        lang.pick("Zum Verlauf", "Show history")
    );
    let title = format!("{}: {id}", lang.pick("Upload-Job", "Upload job"));
    logged_in_page(&visitor, session, Nav::Jobs, &title, &body)
}

// ---------------------------------------------------------------------------
// Settings (read-only)

pub(crate) async fn settings<B: Backend>(State(state): Shared<B>, visitor: Visitor) -> Response {
    let session = match require_session(&visitor) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let lang = visitor.lang;
    let status = match ask(&state, &visitor, session, Request::Status).await {
        Ok(ApiResponse::Status(status)) => status,
        Ok(other) => return unexpected(&visitor, session, &other),
        Err(response) => return response,
    };
    let jobs = match load_jobs(&state, &visitor, session).await {
        Ok(jobs) => jobs,
        Err(response) => return response,
    };
    let yes_no = |value: bool| {
        if value {
            lang.pick("ja", "yes")
        } else {
            lang.pick("nein", "no")
        }
    };
    let mut body = format!(
        "<p class=\"note\">{}</p><table><tr><th>dry_run</th><td>{}</td></tr><tr><th>{}</th><td>{}</td></tr></table>",
        lang.pick(
            "Einstellungen werden in der Konfigurationsdatei geändert (<code>config.toml</code>); danach den Daemon neu starten.",
            "Settings are changed in the configuration file (<code>config.toml</code>); restart the daemon afterwards."
        ),
        yes_no(status.dry_run),
        lang.pick("Version", "Version"),
        escape(&status.daemon_version),
    );
    let _ = write!(
        body,
        "<h2>{}</h2><table><tr><th>id</th><th>local</th><th>remote</th><th>direction</th></tr>",
        lang.pick("Sync-Paare", "Sync pairs")
    );
    for pair in &status.pairs {
        let _ = write!(
            body,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape(&pair.id),
            escape(&pair.local_root),
            escape(&pair.remote_path),
            direction(lang, pair.direction)
        );
    }
    let _ = write!(
        body,
        "</table><h2>{}</h2><table><tr><th>id</th><th>source</th><th>pattern</th><th>target</th><th>schedule</th><th>keep_last</th><th>max_age_days</th><th>verify</th></tr>",
        lang.pick("Upload-Jobs", "Upload jobs")
    );
    for job in &jobs {
        let optional = |value: Option<u32>| value.map_or_else(|| "–".to_owned(), |v| v.to_string());
        let _ = write!(
            body,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape(&job.id),
            escape(&job.source),
            escape(&job.pattern),
            escape(&job.target),
            escape(&job.schedule),
            optional(job.keep_last),
            optional(job.max_age_days),
            match job.verify {
                protonsync_api::JobVerification::Size => "size",
                protonsync_api::JobVerification::Hash => "hash",
            }
        );
    }
    body.push_str("</table>");
    logged_in_page(
        &visitor,
        session,
        Nav::Settings,
        lang.pick("Einstellungen", "Settings"),
        &body,
    )
}
