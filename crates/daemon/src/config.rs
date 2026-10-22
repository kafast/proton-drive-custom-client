//! Configuration of the daemon (TOML file).
//!
//! Example:
//!
//! ```toml
//! [general]
//! dry_run = true              # only plan automatic syncs (default on first use)
//! trash_retention_days = 30
//! secrets = "keyring"         # or "file" (server)
//!
//! [[pair]]
//! id = "main"
//! local = "/home/krischan/Proton"
//! remote = "protonsync"       # folder in "My files"
//! direction = "two_way"       # or "download_only", "upload_only"
//! protect_remote = false      # true: never delete or replace anything in Proton
//! ```
//!
//! Upload jobs (`[[job]]`) are described in [`crate::job_config`], the web interface
//! (`[web]`) in [`crate::web_config`], FUSE drives (`[[mount]]`, `[cache]`) in
//! [`crate::mount_config`], photos (`[photos]`) in [`crate::photo_config`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use protonsync_domain::job::UploadJob;
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncDirection;
use protonsync_domain::trash::RetentionPeriod;
use serde::Deserialize;
use thiserror::Error;

use crate::job_config::{RawJob, ensure_unique_job_ids};
use crate::mount_config::{
    CacheSettings, MountConfig, RawCache, RawMount, ensure_unique_mount_ids,
};
use crate::photo_config::{PhotoConfig, RawPhotos};
use crate::web_config::{RawWeb, WebSettings};

/// Environment variable with the path of the configuration file.
pub const CONFIG_ENV: &str = "PROTONSYNC_CONFIG";
/// Environment variable that overrides the data directory.
pub const DATA_DIR_ENV: &str = "PROTONSYNC_DATA_DIR";

const DEFAULT_RETENTION_DAYS: u32 = 30;
const DEFAULT_EVENT_POLL_SECONDS: u64 = 60;
const DEFAULT_FULL_SYNC_MINUTES: u64 = 60;
const DEFAULT_DEBOUNCE_SECONDS: u64 = 3;
const SECONDS_PER_MINUTE: u64 = 60;
/// Proton asks not to poll for events more often than about every half minute.
const MIN_EVENT_POLL_SECONDS: u64 = 30;

/// Error while reading the configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// File not readable.
    #[error("cannot read configuration {path}: {message}")]
    Unreadable {
        /// Path.
        path: PathBuf,
        /// Message.
        message: String,
    },
    /// Invalid TOML or unknown keys.
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

/// Where secrets are kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretsBackend {
    /// Secret Service of the desktop.
    Keyring,
    /// Encrypted file in the data directory (server).
    File,
}

/// The whole configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Data directory (state, encrypted secrets).
    pub data_dir: PathBuf,
    /// Secret store.
    pub secrets: SecretsBackend,
    /// Only plan automatic syncs and trash purges.
    pub dry_run: bool,
    /// Retention in the trash.
    pub retention: RetentionPeriod,
    /// Intervals of the schedule.
    pub timing: Timing,
    /// Sync pairs.
    pub pairs: Vec<PairConfig>,
    /// Upload jobs.
    pub jobs: Vec<UploadJob>,
    /// Web interface, if enabled.
    pub web: Option<WebSettings>,
    /// FUSE drives (placeholder mode).
    pub mounts: Vec<MountConfig>,
    /// Cache of the drives.
    pub cache: CacheSettings,
    /// Photo sync (`[photos]`); `None` if not set up.
    pub photos: Option<PhotoConfig>,
}

/// Intervals of the schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// How often Proton events are polled.
    pub event_poll: Duration,
    /// Full sync as a safety net.
    pub full_sync: Duration,
    /// Waiting time after local changes before syncing.
    pub local_debounce: Duration,
}

/// A sync pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairConfig {
    /// Identifier (lowercase letters, digits, `-`, `_`).
    pub id: String,
    /// Local folder (absolute).
    pub local: PathBuf,
    /// Folder in "My files"; empty for "My files" itself.
    pub remote: RelativePath,
    /// Direction.
    pub direction: SyncDirection,
    /// Local deletions and replacements are not passed on to Proton.
    pub protect_remote: bool,
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
    #[serde(default)]
    web: RawWeb,
    #[serde(default, rename = "mount")]
    mounts: Vec<RawMount>,
    #[serde(default)]
    cache: RawCache,
    #[serde(default)]
    photos: RawPhotos,
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
    #[serde(default)]
    protect_remote: bool,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RawDirection {
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

/// Environment from which default paths are derived (replaceable in tests).
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
    /// Reads the variables of the running process.
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

    /// Path of the configuration file.
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
    /// Loads the configuration; if the file is missing, the defaults apply (without sync pairs).
    ///
    /// # Errors
    /// [`ConfigError`] if the file is unreadable or invalid.
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

    /// Reads the configuration from TOML text.
    ///
    /// # Errors
    /// [`ConfigError::Invalid`] if the content is invalid.
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
                "event_poll_seconds must be at least {MIN_EVENT_POLL_SECONDS}"
            )));
        }
        let full_sync = general
            .full_sync_minutes
            .unwrap_or(DEFAULT_FULL_SYNC_MINUTES);
        if full_sync == 0 {
            return Err(ConfigError::Invalid(
                "full_sync_minutes must be greater than 0".into(),
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
        let web = raw.web.into_settings()?;
        let mounts = raw
            .mounts
            .into_iter()
            .map(MountConfig::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        ensure_unique_mount_ids(&mounts)?;
        ensure_separate_folders(&pairs, &mounts)?;
        let cache = raw.cache.into_settings()?;
        let photos = raw.photos.into_config()?;
        if let Some(photos) = &photos {
            ensure_separate_photo_folder(&photos.folder, &pairs, &mounts)?;
        }
        Ok(Self {
            data_dir: general.data_dir.unwrap_or_else(|| env.default_data_dir()),
            secrets: general.secrets.unwrap_or(default_secrets()),
            // Safety net: without an explicit setting, syncs are only planned.
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
            web,
            mounts,
            cache,
            photos,
        })
    }

    /// Path of the SQLite state database.
    #[must_use]
    pub fn state_path(&self) -> PathBuf {
        self.data_dir.join("state.sqlite")
    }

    /// Folder with the cache of a drive.
    #[must_use]
    pub fn cache_dir(&self, mount: &MountConfig) -> PathBuf {
        self.data_dir.join("cache").join(&mount.id)
    }

    /// Directory of the encrypted secrets (only with [`SecretsBackend::File`]).
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
        if !is_valid_id(&raw.id) {
            return Err(ConfigError::Invalid(format!(
                "invalid pair identifier {:?} (allowed: a-z, 0-9, -, _)",
                raw.id
            )));
        }
        if !raw.local.is_absolute() {
            return Err(ConfigError::Invalid(format!(
                "pair {}: local folder must be an absolute path",
                raw.id
            )));
        }
        let remote = parse_remote(&raw.remote).map_err(|message| {
            ConfigError::Invalid(format!("pair {}: remote folder: {message}", raw.id))
        })?;
        Ok(Self {
            id: raw.id,
            local: raw.local,
            remote,
            direction: raw.direction.into(),
            protect_remote: raw.protect_remote,
        })
    }
}

/// Identifiers consist of lowercase letters, digits, `-` and `_`.
pub(crate) fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
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
                "duplicate pair identifier {}",
                pair.id
            )));
        }
        if let Some(other) = pairs[..index]
            .iter()
            .find(|other| overlaps(&other.local, &pair.local))
        {
            return Err(ConfigError::Invalid(format!(
                "local folders of {} and {} overlap",
                other.id, pair.id
            )));
        }
    }
    Ok(())
}

/// Mount points must not lie inside each other, nor inside or above sync folders.
fn ensure_separate_folders(
    pairs: &[PairConfig],
    mounts: &[MountConfig],
) -> Result<(), ConfigError> {
    for (index, mount) in mounts.iter().enumerate() {
        if let Some(pair) = pairs
            .iter()
            .find(|pair| overlaps(&pair.local, &mount.mountpoint))
        {
            return Err(ConfigError::Invalid(format!(
                "mount point of {} overlaps with the sync folder of {}",
                mount.id, pair.id
            )));
        }
        if let Some(other) = mounts[..index]
            .iter()
            .find(|other| overlaps(&other.mountpoint, &mount.mountpoint))
        {
            return Err(ConfigError::Invalid(format!(
                "mount points of {} and {} overlap",
                other.id, mount.id
            )));
        }
    }
    Ok(())
}

/// The photo folder must not lie inside or above sync folders or mount points.
fn ensure_separate_photo_folder(
    folder: &Path,
    pairs: &[PairConfig],
    mounts: &[MountConfig],
) -> Result<(), ConfigError> {
    let clash = pairs
        .iter()
        .find(|pair| overlaps(&pair.local, folder))
        .map(|pair| format!("the sync folder of {}", pair.id))
        .or_else(|| {
            mounts
                .iter()
                .find(|mount| overlaps(&mount.mountpoint, folder))
                .map(|mount| format!("the mount point of {}", mount.id))
        });
    match clash {
        Some(other) => Err(ConfigError::Invalid(format!(
            "[photos]: photo folder overlaps with {other}"
        ))),
        None => Ok(()),
    }
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
            protect_remote = true
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
        assert!(!config.pairs[0].protect_remote);
        assert!(config.pairs[1].protect_remote);
    }

    #[test]
    fn rejects_invalid_settings() {
        let cases = [
            "[general]\nunknown = 1",
            "[general]\ntrash_retention_days = 0",
            "[general]\nevent_poll_seconds = 5",
            "[general]\nfull_sync_minutes = 0",
            "[[pair]]\nid = \"A\"\nlocal = \"/x\"",
            "[[pair]]\nid = \"a\"\nlocal = \"relative\"",
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
    fn parses_mounts_and_cache_settings() {
        let text = r#"
            [general]
            data_dir = "/data"

            [[mount]]
            id = "home"
            mountpoint = "/home/k/ProtonDrive"

            [[mount]]
            id = "photos"
            mountpoint = "/home/k/Photos"
            remote = "/Pictures/"
            keep_local = true

            [cache]
            evict_after_days = 7
            max_size_gb = 2
        "#;
        let config = Config::parse(text, &env()).unwrap();
        assert_eq!(config.mounts.len(), 2);
        assert!(config.mounts[0].remote.is_root());
        assert!(!config.mounts[0].keep_local);
        assert_eq!(config.mounts[1].remote.as_str(), "Pictures");
        assert!(config.mounts[1].keep_local);
        assert_eq!(config.cache.policy.unused_days(), 7);
        assert_eq!(config.cache.policy.max_bytes(), 2_000_000_000);
        assert_eq!(
            config.cache_dir(&config.mounts[0]),
            PathBuf::from("/data/cache/home")
        );

        let defaults = Config::parse("", &env()).unwrap();
        assert_eq!(defaults.mounts, Vec::new());
        assert_eq!(defaults.cache.policy.unused_days(), 30);
        assert_eq!(defaults.cache.policy.max_bytes(), 10_000_000_000);
    }

    #[test]
    fn rejects_invalid_mounts() {
        let mount =
            |id: &str, at: &str| format!("[[mount]]\nid = \"{id}\"\nmountpoint = \"{at}\"\n");
        let cases = [
            mount("A", "/x"),
            mount("a", "relative"),
            format!("{}remote = \"a//b\"", mount("a", "/x")),
            format!("{}{}", mount("a", "/x"), mount("a", "/y")),
            format!("{}{}", mount("a", "/x"), mount("b", "/x/y")),
            format!(
                "[[pair]]\nid = \"p\"\nlocal = \"/x\"\n{}",
                mount("a", "/x/drive")
            ),
            "[cache]\nevict_after_days = 0".to_owned(),
            "[cache]\nunknown = 1".to_owned(),
        ];
        for text in cases {
            assert!(
                matches!(Config::parse(&text, &env()), Err(ConfigError::Invalid(_))),
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
            id = "photos"
            source = "/srv/photos"
            target = "Photos"
            schedule = "on_new_file"
        "#;
        let jobs = Config::parse(text, &env()).unwrap().jobs;
        let ha = &jobs[0];
        assert_eq!(ha.target.as_str(), "Backups/HA");
        assert!(ha.pattern.matches("x.tar") && !ha.pattern.matches("x.zip"));
        assert!(matches!(&ha.trigger, JobTrigger::Schedule(s) if s.expression() == "0 3 * * *"));
        assert_eq!(ha.retention.keep_last(), Some(14));
        assert_eq!(ha.verification, Verification::Size);
        let photos = &jobs[1];
        assert_eq!(photos.trigger, JobTrigger::OnNewFile);
        assert!(photos.pattern.matches("anything"));
        assert!(photos.retention.keeps_everything());
        assert_eq!(photos.verification, Verification::Hash);
        assert_eq!((photos.max_attempts, photos.settle_seconds), (5, 60));
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
            job("unknown = 1"),
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
    fn web_interface_is_off_by_default_and_validated() {
        assert_eq!(Config::parse("", &env()).unwrap().web, None);
        let hash = protonsync_web::hash_password("long-enough-123").unwrap();
        let text = format!("[web]\nenabled = true\npassword_hash = \"{hash}\"");
        let web = Config::parse(&text, &env()).unwrap().web.unwrap();
        assert_eq!(web.bind.to_string(), "127.0.0.1:8470");
        assert!(web.password.verify("long-enough-123"));
        assert_eq!(web.allowed, protonsync_web::IpNetwork::private());
        assert_eq!((web.tls, web.session_lifetime.as_secs()), (None, 12 * 3600));

        let full = format!(
            "[web]\nenabled = true\npassword_hash = \"{hash}\"\nbind = \"0.0.0.0:9000\"\n\
             allowed_networks = [\"192.168.178.0/24\"]\ntls_cert = \"/c.pem\"\ntls_key = \"/k.pem\"\n\
             session_hours = 1"
        );
        let web = Config::parse(&full, &env()).unwrap().web.unwrap();
        assert_eq!(web.allowed.len(), 1);
        assert!(web.tls.is_some());

        let cases = [
            "[web]\nenabled = true".to_owned(),
            "[web]\nenabled = true\npassword_hash = \"plaintext\"".to_owned(),
            format!("[web]\nenabled = true\npassword_hash = \"{hash}\"\nbind = \"localhost\""),
            format!("[web]\nenabled = true\npassword_hash = \"{hash}\"\nallowed_networks = []"),
            format!(
                "[web]\nenabled = true\npassword_hash = \"{hash}\"\nallowed_networks = [\"x\"]"
            ),
            format!("[web]\nenabled = true\npassword_hash = \"{hash}\"\ntls_cert = \"/c.pem\""),
            format!("[web]\nenabled = true\npassword_hash = \"{hash}\"\nsession_hours = 0"),
            "[web]\nunknown = 1".to_owned(),
        ];
        for text in &cases {
            assert!(
                matches!(Config::parse(text, &env()), Err(ConfigError::Invalid(_))),
                "{text}"
            );
        }
        // When disabled, the rest is not checked.
        assert_eq!(
            Config::parse("[web]\nenabled = false\nbind = \"x\"", &env())
                .unwrap()
                .web,
            None
        );
    }

    #[test]
    fn missing_file_means_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let env = Environment {
            config_file: Some(dir.path().join("missing.toml")),
            ..env()
        };
        assert_eq!(Config::load(&env).unwrap().pairs, Vec::new());
    }
}
