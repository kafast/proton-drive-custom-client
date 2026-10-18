//! Who makes a request: address (allowed network?), language, session.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::header::{ACCEPT_LANGUAGE, COOKIE, LOCATION, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::AppState;
use crate::html::Lang;

/// Name of the session cookie.
pub(crate) const SESSION_COOKIE: &str = "protonsync_session";
/// Name of the language cookie.
const LANG_COOKIE: &str = "protonsync_lang";
/// The language cookie is valid for one year.
const LANG_COOKIE_SECONDS: u32 = 365 * 24 * 3600;

/// Address of the client; set by the server per connection as a request extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientAddr(pub SocketAddr);

/// A valid session.
pub(crate) struct Session {
    pub(crate) token: String,
    pub(crate) csrf: String,
}

/// Who is asking. Access from outside the allowed networks is rejected with 403
/// before any handler runs.
pub(crate) struct Visitor {
    pub(crate) ip: IpAddr,
    pub(crate) lang: Lang,
    pub(crate) session: Option<Session>,
}

impl<B: Send + Sync + 'static> FromRequestParts<Arc<AppState<B>>> for Visitor {
    type Rejection = Response;

    fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState<B>>,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        // Everything here is synchronous; a ready future is enough.
        std::future::ready(Self::from_parts(parts, state))
    }
}

impl Visitor {
    // The error path is the finished rejection, which axum sends directly.
    #[allow(clippy::result_large_err)]
    fn from_parts<B>(parts: &Parts, state: &AppState<B>) -> Result<Self, Response> {
        let Some(ClientAddr(addr)) = parts.extensions.get::<ClientAddr>().copied() else {
            return Err(StatusCode::FORBIDDEN.into_response());
        };
        let ip = addr.ip();
        if !state.allowed.iter().any(|network| network.contains(ip)) {
            tracing::warn!(%ip, "access to the web interface from outside the allowed networks");
            return Err((StatusCode::FORBIDDEN, "403 Forbidden\n").into_response());
        }
        if let Some(switch) = language_switch(parts) {
            return Err(switch);
        }
        let lang = cookie(&parts.headers, LANG_COOKIE)
            .and_then(Lang::from_code)
            .unwrap_or_else(|| {
                Lang::from_accept_language(
                    parts
                        .headers
                        .get(ACCEPT_LANGUAGE)
                        .and_then(|v| v.to_str().ok()),
                )
            });
        let session = cookie(&parts.headers, SESSION_COOKIE).and_then(|token| {
            state.sessions.csrf(token).map(|csrf| Session {
                token: token.to_owned(),
                csrf,
            })
        });
        Ok(Self { ip, lang, session })
    }
}

/// `?lang=de` or `?lang=en`: remember the language and reload without the parameter.
fn language_switch(parts: &Parts) -> Option<Response> {
    let query = parts.uri.query()?;
    let lang = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("lang="))
        .and_then(Lang::from_code)?;
    let cookie = format!(
        "{LANG_COOKIE}={}; Path=/; Max-Age={LANG_COOKIE_SECONDS}; SameSite=Strict",
        lang.code()
    );
    let mut response = StatusCode::SEE_OTHER.into_response();
    let headers = response.headers_mut();
    headers.insert(LOCATION, HeaderValue::from_str(parts.uri.path()).ok()?);
    headers.insert(SET_COOKIE, HeaderValue::from_str(&cookie).ok()?);
    Some(response)
}

/// Value of a cookie from the `Cookie` header.
pub(crate) fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|pair| {
            let (key, value) = pair.trim().split_once('=')?;
            (key == name).then_some(value)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cookies_from_one_or_more_headers() {
        let mut headers = HeaderMap::new();
        headers.append(COOKIE, HeaderValue::from_static("a=1; protonsync_lang=de"));
        headers.append(COOKIE, HeaderValue::from_static("b=2"));
        assert_eq!(cookie(&headers, "protonsync_lang"), Some("de"));
        assert_eq!(cookie(&headers, "b"), Some("2"));
        assert_eq!(cookie(&headers, "c"), None);
    }
}
