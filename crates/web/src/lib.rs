//! Web-Oberfläche von protonsync: ein schlankes Statusboard für den Server.
//!
//! - Standardmäßig aus; der Daemon startet sie nur mit `[web] enabled = true`.
//! - Nur aus erlaubten Netzen erreichbar (Standard: eigener Rechner und private Netze).
//! - Anmeldung mit einem Passwort, dessen Argon2-Hash in der Konfiguration steht.
//! - Formulare sind gegen CSRF geschützt, Sitzungs-Cookies sind `HttpOnly` und
//!   `SameSite=Strict`, bei TLS zusätzlich `Secure`.
//! - Keine eigene Geschäftslogik: Jede Aktion ist eine Anfrage an die Daemon-API
//!   ([`Backend`]), genau wie bei CLI und GUI.
//! - Server-seitig erzeugtes HTML ohne JavaScript.

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

/// Zugang zur Daemon-API, über den die Oberfläche alles erledigt.
pub trait Backend: Send + Sync + 'static {
    /// Beantwortet eine Anfrage.
    fn handle(&self, request: Request) -> impl Future<Output = Response> + Send;
}

/// Einstellungen der Oberfläche.
#[derive(Debug, Clone)]
pub struct WebConfig {
    /// Adresse und Port, an die der Server sich bindet.
    pub bind: SocketAddr,
    /// Prüfung des Passworts.
    pub password: PasswordCheck,
    /// Netze, aus denen Zugriffe erlaubt sind.
    pub allowed: Vec<IpNetwork>,
    /// Zertifikat und Schlüssel für HTTPS; ohne nur HTTP (z. B. hinter einem Reverse Proxy).
    pub tls: Option<TlsFiles>,
    /// Wie lange eine Anmeldung gilt.
    pub session_lifetime: Duration,
    /// Zeitzone für die Anzeige von Zeiten.
    pub zone: TimeZone,
}

/// Dateien für HTTPS (PEM).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsFiles {
    /// Zertifikatskette.
    pub cert: PathBuf,
    /// Privater Schlüssel.
    pub key: PathBuf,
}

/// Gemeinsamer Zustand aller Anfragen.
pub(crate) struct AppState<B> {
    pub(crate) backend: Arc<B>,
    pub(crate) password: PasswordCheck,
    pub(crate) sessions: auth::Sessions,
    pub(crate) guard: auth::LoginGuard,
    pub(crate) allowed: Vec<IpNetwork>,
    pub(crate) secure_cookies: bool,
    pub(crate) zone: TimeZone,
}

/// Baut die Routen der Oberfläche. Jede Anfrage braucht die Erweiterung
/// [`ClientAddr`] (setzt [`serve`]; in Tests von Hand).
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
