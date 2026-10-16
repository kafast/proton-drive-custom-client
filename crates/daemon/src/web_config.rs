//! Abschnitt `[web]` der Konfiguration: die optionale Web-Oberfläche.
//!
//! ```toml
//! [web]
//! enabled = true
//! bind = "0.0.0.0:8470"                 # Standard 127.0.0.1:8470 (nur dieser Rechner)
//! password_hash = "$argon2id$…"         # mit `protonsyncd hash-password` erzeugen
//! allowed_networks = ["192.168.178.0/24"]  # Standard: private Netze
//! tls_cert = "/etc/protonsync/cert.pem" # optional, zusammen mit tls_key
//! tls_key = "/etc/protonsync/key.pem"
//! session_hours = 12
//! ```

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use protonsync_web::{IpNetwork, PasswordCheck, TlsFiles};
use serde::Deserialize;

use crate::config::ConfigError;

/// Standardadresse: nur vom eigenen Rechner erreichbar.
pub const DEFAULT_BIND: &str = "127.0.0.1:8470";
const DEFAULT_SESSION_HOURS: u64 = 12;
const SECONDS_PER_HOUR: u64 = 3600;

/// Eingeschaltete Web-Oberfläche.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSettings {
    /// Adresse und Port.
    pub bind: SocketAddr,
    /// Prüfung des Passworts.
    pub password: PasswordCheck,
    /// Erlaubte Netze.
    pub allowed: Vec<IpNetwork>,
    /// HTTPS-Dateien.
    pub tls: Option<TlsFiles>,
    /// Gültigkeit einer Anmeldung.
    pub session_lifetime: Duration,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawWeb {
    #[serde(default)]
    enabled: bool,
    bind: Option<String>,
    password_hash: Option<String>,
    allowed_networks: Option<Vec<String>>,
    tls_cert: Option<PathBuf>,
    tls_key: Option<PathBuf>,
    session_hours: Option<u64>,
}

fn invalid(message: impl std::fmt::Display) -> ConfigError {
    ConfigError::Invalid(format!("[web]: {message}"))
}

impl RawWeb {
    /// `None`, wenn die Oberfläche aus ist (Standard).
    pub(crate) fn into_settings(self) -> Result<Option<WebSettings>, ConfigError> {
        if !self.enabled {
            return Ok(None);
        }
        let bind_text = self.bind.as_deref().unwrap_or(DEFAULT_BIND);
        let bind: SocketAddr = bind_text
            .parse()
            .map_err(|_| invalid(format!("bind {bind_text:?} ist keine Adresse mit Port")))?;
        let hash = self.password_hash.ok_or_else(|| {
            invalid("password_hash fehlt (mit `protonsyncd hash-password` erzeugen)")
        })?;
        let password = PasswordCheck::new(&hash).map_err(invalid)?;
        let allowed = match self.allowed_networks {
            None => IpNetwork::private(),
            Some(networks) => networks
                .iter()
                .map(|text| text.parse::<IpNetwork>().map_err(invalid))
                .collect::<Result<Vec<_>, _>>()?,
        };
        if allowed.is_empty() {
            return Err(invalid("allowed_networks ist leer; niemand käme hinein"));
        }
        let tls = match (self.tls_cert, self.tls_key) {
            (Some(cert), Some(key)) => Some(TlsFiles { cert, key }),
            (None, None) => None,
            _ => return Err(invalid("tls_cert und tls_key nur zusammen angeben")),
        };
        let hours = self.session_hours.unwrap_or(DEFAULT_SESSION_HOURS);
        if hours == 0 {
            return Err(invalid("session_hours muss größer als 0 sein"));
        }
        Ok(Some(WebSettings {
            bind,
            password,
            allowed,
            tls,
            session_lifetime: Duration::from_secs(hours.saturating_mul(SECONDS_PER_HOUR)),
        }))
    }
}
