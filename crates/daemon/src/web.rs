//! Connects the web interface to the core: the interface makes the same
//! requests as the CLI and GUI, just without the detour through the socket.

use std::future::Future;

use protonsync_api::{Request, Response};
use protonsync_application::ports::{Account, Clock, LocalStorage};
use protonsync_web::WebConfig;

use crate::core::{Core, DaemonState};
use crate::local_time::LocalZone;
use crate::web_config::WebSettings;

impl<A, L, S, C> protonsync_web::Backend for Core<A, L, S, C>
where
    A: Account + 'static,
    A::Remote: 'static,
    L: LocalStorage + 'static,
    S: DaemonState,
    C: Clock + 'static,
{
    fn handle(&self, request: Request) -> impl Future<Output = Response> + Send {
        Core::handle(self, request)
    }
}

/// Builds the web server settings from the `[web]` section.
#[must_use]
pub fn web_config(settings: &WebSettings, zone: &LocalZone) -> WebConfig {
    WebConfig {
        bind: settings.bind,
        password: settings.password.clone(),
        allowed: settings.allowed.clone(),
        tls: settings.tls.clone(),
        session_lifetime: settings.session_lifetime,
        zone: zone.time_zone().clone(),
    }
}
