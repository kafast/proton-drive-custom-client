//! Web interface of protonsync: a lean status board for the server.
//!
//! - Off by default; the daemon only starts it with `[web] enabled = true`.
//! - Reachable only from allowed networks (default: the local machine and private networks).
//! - Login with a password whose Argon2 hash is stored in the configuration.
//! - Forms are protected against CSRF, session cookies are `HttpOnly` and
//!   `SameSite=Strict`, plus `Secure` with TLS.
//! - No business logic of its own: every action is a request to the daemon API
//!   ([`Backend`]), just like the CLI and GUI.
//! - Server-side generated HTML without JavaScript.

mod auth;
mod html;
pub mod net;
mod pages;
mod server;
mod visitor;

use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::routing::{get, post};
use jiff::tz::TimeZone;
use protonsync_api::{Request, Response};

pub use auth::{AuthError, PasswordCheck, hash_password};
pub use net::IpNetwork;
pub use server::{WebError, serve};
pub use visitor::ClientAddr;

/// Access to the daemon API, through which the interface does everything.
pub trait Backend: Send + Sync + 'static {
    /// Answers a request.
    fn handle(&self, request: Request) -> impl Future<Output = Response> + Send;
}

/// Settings of the interface.
#[derive(Debug, Clone)]
pub struct WebConfig {
    /// Address and port the server binds to.
    pub bind: SocketAddr,
    /// Password check.
    pub password: PasswordCheck,
    /// Networks from which access is allowed.
    pub allowed: Vec<IpNetwork>,
    /// Certificate and key for HTTPS; without them plain HTTP (e.g. behind a reverse proxy).
    pub tls: Option<TlsFiles>,
    /// How long a login stays valid.
    pub session_lifetime: Duration,
    /// Time zone for displaying times.
    pub zone: TimeZone,
}

/// Files for HTTPS (PEM).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsFiles {
    /// Certificate chain.
    pub cert: PathBuf,
    /// Private key.
    pub key: PathBuf,
}

/// State shared by all requests.
pub(crate) struct AppState<B> {
    pub(crate) backend: Arc<B>,
    pub(crate) password: PasswordCheck,
    pub(crate) sessions: auth::Sessions,
    pub(crate) guard: auth::LoginGuard,
    pub(crate) allowed: Vec<IpNetwork>,
    pub(crate) secure_cookies: bool,
    pub(crate) zone: TimeZone,
}

/// Builds the routes of the interface. Every request needs the extension
/// [`ClientAddr`] (set by [`serve`]; by hand in tests).
pub fn router<B: Backend>(backend: Arc<B>, config: &WebConfig) -> Router {
    let state = Arc::new(AppState {
        backend,
        password: config.password.clone(),
        sessions: auth::Sessions::new(config.session_lifetime),
        guard: auth::LoginGuard::default(),
        allowed: config.allowed.clone(),
        secure_cookies: config.tls.is_some(),
        zone: config.zone.clone(),
    });
    Router::new()
        .route("/", get(pages::status::<B>))
        .route(
            "/login",
            get(pages::login_form::<B>).post(pages::login::<B>),
        )
        .route("/logout", post(pages::logout::<B>))
        .route("/sync", post(pages::sync::<B>))
        .route("/conflicts", get(pages::conflicts::<B>))
        .route("/conflicts/resolve", post(pages::resolve::<B>))
        .route("/trash", get(pages::trash::<B>))
        .route("/trash/restore", post(pages::restore::<B>))
        .route("/trash/purge", post(pages::purge::<B>))
        .route("/jobs", get(pages::jobs::<B>))
        .route("/jobs/{id}", get(pages::job_history::<B>))
        .route("/jobs/{id}/run", post(pages::run_job::<B>))
        .route("/photos", get(pages::photos::photos::<B>))
        .route("/photos/sync", post(pages::photos::sync::<B>))
        .route("/photos/duplicates", get(pages::photos::duplicates::<B>))
        .route("/photos/duplicates/trash", post(pages::photos::trash::<B>))
        .route("/settings", get(pages::settings::<B>))
        .route("/static/style.css", get(pages::style))
        .fallback(pages::not_found)
        .layer(axum::middleware::map_response(pages::security_headers))
        .with_state(state)
}
