//! Konfiguration des Daemons (TOML-Datei).
//!
//! Beispiel:
//!
//! ```toml
//! [general]
//! dry_run = true              # automatische Abgleiche nur planen (Standard beim ersten Einsatz)
//! trash_retention_days = 30
//! secrets = "keyring"         # oder "file" (Server)
//!
//! [[pair]]
//! id = "main"
//! local = "/home/krischan/Proton"
//! remote = "protonsync"       # Ordner in „Meine Dateien“
//! direction = "two_way"       # oder "download_only", "upload_only"
//! ```
//!
//! Upload-Jobs (`[[job]]`) beschreibt [`crate::job_config`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use protonsync_domain::job::UploadJob;
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncDirection;
use protonsync_domain::trash::RetentionPeriod;
use serde::Deserialize;
use thiserror::Error;

use crate::job_config::{RawJob, ensure_unique_job_ids};

/// Umgebungsvariable mit dem Pfad der Konfigurationsdatei.
pub const CONFIG_ENV: &str = "PROTONSYNC_CONFIG";
/// Umgebungsvariable, die das Datenverzeichnis überschreibt.
pub const DATA_DIR_ENV: &str = "PROTONSYNC_DATA_DIR";

const DEFAULT_RETENTION_DAYS: u32 = 30;
const DEFAULT_EVENT_POLL_SECONDS: u64 = 60;
const DEFAULT_FULL_SYNC_MINUTES: u64 = 60;
const DEFAULT_DEBOUNCE_SECONDS: u64 = 3;
const SECONDS_PER_MINUTE: u64 = 60;
/// Proton bittet, nicht häufiger als etwa alle halbe Minute nach Ereignissen zu fragen.
const MIN_EVENT_POLL_SECONDS: u64 = 30;

/// Fehler beim Lesen der Konfiguration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Datei nicht lesbar.
    #[error("Konfiguration {path} nicht lesbar: {message}")]
    Unreadable {
        /// Pfad.
        path: PathBuf,
        /// Meldung.
        message: String,
    },
    /// Ungültiges TOML oder unbekannte Schlüssel.
    #[error("Konfiguration ungültig: {0}")]
    Invalid(String),
}

/// Wo Geheimnisse liegen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretsBackend {
    /// Secret Service des Desktops.
    Keyring,
    /// Verschlüsselte Datei im Datenverzeichnis (Server).
    File,
}

/// Gesamte Konfiguration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Datenverzeichnis (Zustand, verschlüsselte Geheimnisse).
    pub data_dir: PathBuf,
    /// Geheimnisspeicher.
    pub secrets: SecretsBackend,
    /// Automatische Abgleiche und Papierkorb-Leerung nur planen.
    pub dry_run: bool,
    /// Aufbewahrung im Papierkorb.
    pub retention: RetentionPeriod,
    /// Zeitabstände des Zeitplans.
    pub timing: Timing,
    /// Sync-Paare.
    pub pairs: Vec<PairConfig>,
    /// Upload-Jobs.
    pub jobs: Vec<UploadJob>,
}

/// Zeitabstände des Zeitplans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// Wie oft nach Proton-Ereignissen gefragt wird.
    pub event_poll: Duration,
    /// Vollständiger Abgleich als Sicherheitsnetz.
    pub full_sync: Duration,
    /// Wartezeit nach lokalen Änderungen, bevor abgeglichen wird.
    pub local_debounce: Duration,
}

/// Ein Sync-Paar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairConfig {
    /// Kennung (Kleinbuchstaben, Ziffern, `-`, `_`).
    pub id: String,
    /// Lokaler Ordner (absolut).
    pub local: PathBuf,
    /// Ordner in „Meine Dateien“; leer für „Meine Dateien“ selbst.
    pub remote: RelativePath,
    /// Richtung.
    pub direction: SyncDirection,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    general: RawGeneral,
    #[serde(default, rename = "pair")]
    pairs: Vec<RawPair>,
    #[serde(default, rename = "job")]
    jobs: Vec<RawJob>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGeneral {
    data_dir: Option<PathBuf>,
    secrets: Option<SecretsBackend>,
    dry_run: Option<bool>,
    trash_retention_days: Option<u32>,
    event_poll_seconds: Option<u64>,
    full_sync_minutes: Option<u64>,
    local_debounce_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPair {
    id: String,
    local: PathBuf,
    #[serde(default)]
    remote: String,
    #[serde(default)]
    direction: RawDirection,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawDirection {
    #[default]
    TwoWay,
    DownloadOnly,
    UploadOnly,
}

impl From<RawDirection> for SyncDirection {
    fn from(raw: RawDirection) -> Self {
        match raw {
            RawDirection::TwoWay => Self::TwoWay,
            RawDirection::DownloadOnly => Self::DownloadOnly,
            RawDirection::UploadOnly => Self::UploadOnly,
        }
    }
}

/// Umgebung, aus der Standardpfade abgeleitet werden (für Tests austauschbar).
#[derive(Debug, Clone, Default)]
pub struct Environment {
    /// `PROTONSYNC_CONFIG`.
    pub config_file: Option<PathBuf>,
    /// `PROTONSYNC_DATA_DIR`.
    pub data_dir: Option<PathBuf>,
    /// `XDG_CONFIG_HOME`.
    pub xdg_config_home: Option<PathBuf>,
    /// `XDG_DATA_HOME`.
    pub xdg_data_home: Option<PathBuf>,
    /// `HOME`.
    pub home: Option<PathBuf>,
}

impl Environment {
    /// Liest die Variablen des laufenden Prozesses.
    #[must_use]
    pub fn from_process() -> Self {
        let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
        Self {
            config_file: var(CONFIG_ENV),
            data_dir: var(DATA_DIR_ENV),
            xdg_config_home: var("XDG_CONFIG_HOME"),
            xdg_data_home: var("XDG_DATA_HOME"),
            home: var("HOME"),
        }
    }

    /// Pfad der Konfigurationsdatei.
    #[must_use]
    pub fn config_path(&self) -> PathBuf {
        self.config_file.clone().unwrap_or_else(|| {
            self.xdg_dir(self.xdg_config_home.as_ref(), ".config")
                .join("protonsync")
                .join("config.toml")
        })
    }

    fn default_data_dir(&self) -> PathBuf {
        self.data_dir.clone().unwrap_or_else(|| {
            self.xdg_dir(self.xdg_data_home.as_ref(), ".local/share")
                .join("protonsync")
        })
    }

    fn xdg_dir(&self, explicit: Option<&PathBuf>, below_home: &str) -> PathBuf {
        explicit.cloned().unwrap_or_else(|| {
            self.home
                .clone()
                .unwrap_or_else(|| PathBuf::from("/"))
                .join(below_home)
        })
    }
}

impl Config {
    /// Lädt die Konfiguration; fehlt die Datei, gelten die Standardwerte (ohne Sync-Paare).
    ///
    /// # Errors
    /// [`ConfigError`] bei unlesbarer oder ungültiger Datei.
    pub fn load(env: &Environment) -> Result<Self, ConfigError> {
        let path = env.config_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text, env),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::parse("", env),
            Err(e) => Err(ConfigError::Unreadable {
                path,
                message: e.to_string(),
            }),
        }
    }

    /// Liest die Konfiguration aus TOML-Text.
    ///
    /// # Errors
    /// [`ConfigError::Invalid`] bei ungültigem Inhalt.
    pub fn parse(text: &str, env: &Environment) -> Result<Self, ConfigError> {
        let raw: RawConfig =
            toml::from_str(text).map_err(|e| ConfigError::Invalid(e.to_string()))?;
        let general = raw.general;
        let retention = RetentionPeriod::days(
            general
                .trash_retention_days
                .unwrap_or(DEFAULT_RETENTION_DAYS),
        )
        .map_err(|e| ConfigError::Invalid(e.to_string()))?;
        let event_poll = general
            .event_poll_seconds
            .unwrap_or(DEFAULT_EVENT_POLL_SECONDS);
        if event_poll < MIN_EVENT_POLL_SECONDS {
            return Err(ConfigError::Invalid(format!(
                "event_poll_seconds muss mindestens {MIN_EVENT_POLL_SECONDS} sein"
            )));
        }
        let full_sync = general
            .full_sync_minutes
            .unwrap_or(DEFAULT_FULL_SYNC_MINUTES);
        if full_sync == 0 {
            return Err(ConfigError::Invalid(
                "full_sync_minutes muss größer als 0 sein".into(),
            ));
        }
        let pairs = raw
            .pairs
            .into_iter()
            .map(PairConfig::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        ensure_unique_ids(&pairs)?;
        let jobs = raw
            .jobs
            .into_iter()
            .map(UploadJob::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        ensure_unique_job_ids(&jobs)?;
        Ok(Self {
            data_dir: general.data_dir.unwrap_or_else(|| env.default_data_dir()),
            secrets: general.secrets.unwrap_or(default_secrets()),
            // Sicherheitsnetz: Ohne ausdrückliche Angabe wird nur geplant.
            dry_run: general.dry_run.unwrap_or(true),
            retention,
            timing: Timing {
                event_poll: Duration::from_secs(event_poll),
                full_sync: Duration::from_secs(full_sync.saturating_mul(SECONDS_PER_MINUTE)),
                local_debounce: Duration::from_secs(
                    general
                        .local_debounce_seconds
                        .unwrap_or(DEFAULT_DEBOUNCE_SECONDS),
                ),
            },
            pairs,
            jobs,
        })
    }

    /// Pfad der SQLite-Zustandsdatenbank.
    #[must_use]
    pub fn state_path(&self) -> PathBuf {
        self.data_dir.join("state.sqlite")
    }

    /// Verzeichnis der verschlüsselten Geheimnisse (nur bei [`SecretsBackend::File`]).
    #[must_use]
    pub fn secrets_dir(&self) -> PathBuf {
        self.data_dir.join("secrets")
    }
}

fn default_secrets() -> SecretsBackend {
    if cfg!(feature = "keyring") {
        SecretsBackend::Keyring
    } else {
        SecretsBackend::File
    }
}

impl TryFrom<RawPair> for PairConfig {
    type Error = ConfigError;

    fn try_from(raw: RawPair) -> Result<Self, Self::Error> {
        let valid_id = !raw.id.is_empty()
            && raw
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
        if !valid_id {
            return Err(ConfigError::Invalid(format!(
                "Ungültige Paar-Kennung {:?} (erlaubt: a-z, 0-9, -, _)",
                raw.id
            )));
        }
        if !raw.local.is_absolute() {
            return Err(ConfigError::Invalid(format!(
                "Paar {}: lokaler Ordner muss ein absoluter Pfad sein",
                raw.id
            )));
        }
        let remote = parse_remote(&raw.remote).map_err(|message| {
            ConfigError::Invalid(format!("Paar {}: entfernter Ordner: {message}", raw.id))
        })?;
        Ok(Self {
            id: raw.id,
            local: raw.local,
            remote,
            direction: raw.direction.into(),
        })
    }
}

pub(crate) fn parse_remote(text: &str) -> Result<RelativePath, String> {
    let trimmed = text.trim_matches('/');
    if trimmed.is_empty() {
        return Ok(RelativePath::root());
    }
    RelativePath::parse(trimmed).map_err(|e| e.to_string())
}

fn ensure_unique_ids(pairs: &[PairConfig]) -> Result<(), ConfigError> {
    for (index, pair) in pairs.iter().enumerate() {
        if pairs[..index].iter().any(|other| other.id == pair.id) {
            return Err(ConfigError::Invalid(format!(
                "Paar-Kennung {} doppelt",
                pair.id
            )));
        }
        if let Some(other) = pairs[..index]
            .iter()
            .find(|other| overlaps(&other.local, &pair.local))
        {
            return Err(ConfigError::Invalid(format!(
                "Lokale Ordner von {} und {} überschneiden sich",
                other.id, pair.id
            )));
        }
    }
    Ok(())
}

fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use protonsync_domain::job::{JobTrigger, Verification};

    fn env() -> Environment {
        Environment {
            home: Some("/home/k".into()),
            ..Environment::default()
        }
    }

    #[test]
    fn defaults_are_safe() {
        let config = Config::parse("", &env()).unwrap();
        assert!(config.dry_run);
        assert_eq!(config.retention, RetentionPeriod::default());
        assert_eq!(
            config.data_dir,
            PathBuf::from("/home/k/.local/share/protonsync")
        );
        assert_eq!(config.timing.event_poll, Duration::from_secs(60));
        assert_eq!(config.pairs, Vec::new());
    }

    #[test]
    fn parses_pairs_and_general_settings() {
        let text = r#"
            [general]
            dry_run = false
            trash_retention_days = 7
            secrets = "file"
            data_dir = "/var/lib/protonsync"

            [[pair]]
            id = "main"
            local = "/home/k/Proton"
            remote = "/Backups/Server/"
            direction = "download_only"

            [[pair]]
            id = "docs"
            local = "/home/k/Docs"
        "#;
        let config = Config::parse(text, &env()).unwrap();
        assert!(!config.dry_run);
        assert_eq!(config.secrets, SecretsBackend::File);
        assert_eq!(
            config.state_path(),
            PathBuf::from("/var/lib/protonsync/state.sqlite")
        );
        assert_eq!(config.pairs[0].remote.as_str(), "Backups/Server");
        assert_eq!(config.pairs[0].direction, SyncDirection::DownloadOnly);
        assert!(config.pairs[1].remote.is_root());
        assert_eq!(config.pairs[1].direction, SyncDirection::TwoWay);
    }

    #[test]
    fn rejects_invalid_settings() {
        let cases = [
            "[general]\nunknown = 1",
            "[general]\ntrash_retention_days = 0",
            "[general]\nevent_poll_seconds = 5",
            "[general]\nfull_sync_minutes = 0",
            "[[pair]]\nid = \"A\"\nlocal = \"/x\"",
            "[[pair]]\nid = \"a\"\nlocal = \"relativ\"",
            "[[pair]]\nid = \"a\"\nlocal = \"/x\"\nremote = \"a//b\"",
            "[[pair]]\nid = \"a\"\nlocal = \"/x\"\n[[pair]]\nid = \"a\"\nlocal = \"/y\"",
            "[[pair]]\nid = \"a\"\nlocal = \"/x\"\n[[pair]]\nid = \"b\"\nlocal = \"/x/y\"",
        ];
        for text in cases {
            assert!(
                matches!(Config::parse(text, &env()), Err(ConfigError::Invalid(_))),
                "{text}"
            );
        }
    }

    #[test]
    fn paths_follow_xdg_and_overrides() {
        let mut env = env();
        assert_eq!(
            env.config_path(),
            PathBuf::from("/home/k/.config/protonsync/config.toml")
        );
        env.xdg_config_home = Some("/cfg".into());
        env.xdg_data_home = Some("/data".into());
        assert_eq!(
            env.config_path(),
            PathBuf::from("/cfg/protonsync/config.toml")
        );
        assert_eq!(env.default_data_dir(), PathBuf::from("/data/protonsync"));
        env.config_file = Some("/etc/protonsync.toml".into());
        env.data_dir = Some("/srv/ps".into());
        assert_eq!(env.config_path(), PathBuf::from("/etc/protonsync.toml"));
        assert_eq!(env.default_data_dir(), PathBuf::from("/srv/ps"));
    }

    #[test]
    fn parses_upload_jobs_with_defaults() {
        let text = r#"
            [[job]]
            id = "ha"
            source = "/mnt/ha"
            pattern = "*.tar"
            target = "/Backups/HA/"
            schedule = "0 3 * * *"
            keep_last = 14
            verify = "size"

            [[job]]
            id = "fotos"
            source = "/srv/fotos"
            target = "Fotos"
            schedule = "on_new_file"
        "#;
        let jobs = Config::parse(text, &env()).unwrap().jobs;
        let ha = &jobs[0];
        assert_eq!(ha.target.as_str(), "Backups/HA");
        assert!(ha.pattern.matches("x.tar") && !ha.pattern.matches("x.zip"));
        assert!(matches!(&ha.trigger, JobTrigger::Schedule(s) if s.expression() == "0 3 * * *"));
        assert_eq!(ha.retention.keep_last(), Some(14));
        assert_eq!(ha.verification, Verification::Size);
        let fotos = &jobs[1];
        assert_eq!(fotos.trigger, JobTrigger::OnNewFile);
        assert!(fotos.pattern.matches("alles"));
        assert!(fotos.retention.keeps_everything());
        assert_eq!(fotos.verification, Verification::Hash);
        assert_eq!((fotos.max_attempts, fotos.settle_seconds), (5, 60));
    }

    #[test]
    fn rejects_invalid_jobs() {
        let job = |extra: &str| {
            format!(
                "[[job]]\nid = \"j\"\nsource = \"/q\"\ntarget = \"Z\"\nschedule = \"@daily\"\n{extra}"
            )
        };
        let cases = [
            "[[job]]\nid = \"J\"\nsource = \"/q\"\ntarget = \"Z\"\nschedule = \"@daily\""
                .to_owned(),
            "[[job]]\nid = \"j\"\nsource = \"q\"\ntarget = \"Z\"\nschedule = \"@daily\"".to_owned(),
            "[[job]]\nid = \"j\"\nsource = \"/q\"\ntarget = \"/\"\nschedule = \"@daily\""
                .to_owned(),
            "[[job]]\nid = \"j\"\nsource = \"/q\"\ntarget = \"Z\"\nschedule = \"61 * * * *\""
                .to_owned(),
            job("pattern = \"a/b\""),
            job("keep_last = 0"),
            job("max_attempts = 0"),
            job("verify = \"md5\""),
            job("unbekannt = 1"),
            format!("{}\n{}", job(""), job("")),
        ];
        for text in &cases {
            assert!(
                matches!(Config::parse(text, &env()), Err(ConfigError::Invalid(_))),
                "{text}"
            );
        }
    }

    #[test]
    fn missing_file_means_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let env = Environment {
            config_file: Some(dir.path().join("fehlt.toml")),
            ..env()
        };
        assert_eq!(Config::load(&env).unwrap().pairs, Vec::new());
    }
}
