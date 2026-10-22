//! Section `[notify]` of the configuration: notifications by email and webhook
//! and the weekly report (ADR 0017). Without the section nothing is sent.
//!
//! ```toml
//! [notify]
//! language = "de"                        # "de" (default) or "en"
//! weekly_report_schedule = "0 8 * * 1"   # cron in local time, default Monday 8:00
//!
//! [notify.mail]
//! host = "smtp.example.org"
//! port = 587                             # default: 587 (starttls), 465 (tls), 25 (none)
//! security = "starttls"                  # or "tls", "none"
//! username = "me@example.org"            # optional
//! password_file = "/etc/protonsync/smtp-password"  # optional, mode 0600
//! from = "protonsync <me@example.org>"
//! to = ["me@example.org"]
//! conflicts = true                       # every topic can be switched off on its own
//! sync_failures = true
//! job_failures = true
//! login_required = true
//! weekly_report = false
//!
//! [notify.webhook]
//! url = "https://ntfy.sh/my-topic"
//! format = "text"                        # or "json"
//! token_file = "/etc/protonsync/webhook-token"     # optional, mode 0600
//! ```
//!
//! Passwords and tokens never stand in the file itself. They come from a systemd
//! credential (`protonsync-smtp-password`, `protonsync-webhook-token`), the
//! file named here or an environment variable (`PROTONSYNC_SMTP_PASSWORD`,
//! `PROTONSYNC_WEBHOOK_TOKEN`), in this order.

use std::path::{Path, PathBuf};

use protonsync_adapter_notify::{MailSecurity, WebhookFormat};
use protonsync_client::Lang;
use protonsync_domain::job::CronSchedule;
use serde::Deserialize;
use zeroize::Zeroizing;

use crate::config::ConfigError;

/// Default time of the weekly report: Monday 8:00 local time.
pub const DEFAULT_REPORT_SCHEDULE: &str = "0 8 * * 1";
/// Name of the systemd credential with the SMTP password.
pub const SMTP_PASSWORD_CREDENTIAL: &str = "protonsync-smtp-password";
/// Environment variable with the SMTP password.
pub const SMTP_PASSWORD_ENV: &str = "PROTONSYNC_SMTP_PASSWORD";
/// Name of the systemd credential with the webhook token.
pub const WEBHOOK_TOKEN_CREDENTIAL: &str = "protonsync-webhook-token";
/// Environment variable with the webhook token.
pub const WEBHOOK_TOKEN_ENV: &str = "PROTONSYNC_WEBHOOK_TOKEN";

const PORT_STARTTLS: u16 = 587;
const PORT_TLS: u16 = 465;
const PORT_PLAIN: u16 = 25;
#[cfg(unix)]
const GROUP_AND_OTHER_BITS: u32 = 0o077;

/// What a channel reports; each topic can be switched on or off on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // One switch per topic is the point here.
pub struct Topics {
    /// New conflicts.
    pub conflicts: bool,
    /// A sync pair or the photo sync fails.
    pub sync_failures: bool,
    /// An upload job gave up after all attempts.
    pub job_failures: bool,
    /// The login expired; someone has to log in again.
    pub login_required: bool,
    /// Weekly summary.
    pub weekly_report: bool,
}

/// A secret that is read only when the daemon starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretSource {
    /// systemd credential name.
    pub credential: &'static str,
    /// File from the configuration.
    pub file: Option<PathBuf>,
    /// Environment variable.
    pub env: &'static str,
}

/// Email channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailConfig {
    /// SMTP server.
    pub host: String,
    /// Port.
    pub port: u16,
    /// Encryption.
    pub security: MailSecurity,
    /// Login name; the password comes from `password`.
    pub username: Option<String>,
    /// Where the password comes from (only with `username`).
    pub password: SecretSource,
    /// Sender.
    pub from: String,
    /// Recipients.
    pub to: Vec<String>,
    /// What is sent.
    pub topics: Topics,
}

/// Webhook channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookConfig {
    /// Target address.
    pub url: String,
    /// Body format.
    pub format: WebhookFormat,
    /// Where the optional token comes from.
    pub token: SecretSource,
    /// What is sent.
    pub topics: Topics,
}

/// Notifications, if `[notify]` contains at least one channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyConfig {
    /// Language of the messages.
    pub lang: Lang,
    /// When the weekly report goes out.
    pub report_schedule: CronSchedule,
    /// Email.
    pub mail: Option<MailConfig>,
    /// Webhook.
    pub webhook: Option<WebhookConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawNotify {
    language: Option<RawLanguage>,
    weekly_report_schedule: Option<String>,
    mail: Option<RawMail>,
    webhook: Option<RawWebhook>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawLanguage {
    De,
    En,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawSecurity {
    Starttls,
    Tls,
    None,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawFormat {
    #[default]
    Json,
    Text,
}

/// The switches shared by both channels; a missing switch means: everything that
/// needs attention is on, the weekly report must be chosen.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
struct RawTopics {
    conflicts: Option<bool>,
    sync_failures: Option<bool>,
    job_failures: Option<bool>,
    login_required: Option<bool>,
    weekly_report: Option<bool>,
}

impl From<RawTopics> for Topics {
    fn from(raw: RawTopics) -> Self {
        Self {
            conflicts: raw.conflicts.unwrap_or(true),
            sync_failures: raw.sync_failures.unwrap_or(true),
            job_failures: raw.job_failures.unwrap_or(true),
            login_required: raw.login_required.unwrap_or(true),
            weekly_report: raw.weekly_report.unwrap_or(false),
        }
    }
}

// The switches are listed once per channel instead of `#[serde(flatten)]`,
// because `flatten` would silently accept unknown (mistyped) keys.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawMail {
    #[serde(default = "enabled_by_default")]
    enabled: bool,
    host: String,
    port: Option<u16>,
    security: Option<RawSecurity>,
    username: Option<String>,
    password_file: Option<PathBuf>,
    from: String,
    to: Vec<String>,
    conflicts: Option<bool>,
    sync_failures: Option<bool>,
    job_failures: Option<bool>,
    login_required: Option<bool>,
    weekly_report: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawWebhook {
    #[serde(default = "enabled_by_default")]
    enabled: bool,
    url: String,
    #[serde(default)]
    format: RawFormat,
    token_file: Option<PathBuf>,
    conflicts: Option<bool>,
    sync_failures: Option<bool>,
    job_failures: Option<bool>,
    login_required: Option<bool>,
    weekly_report: Option<bool>,
}

/// Picks the switches out of a channel section.
macro_rules! topics_of {
    ($raw:expr) => {
        Topics::from(RawTopics {
            conflicts: $raw.conflicts,
            sync_failures: $raw.sync_failures,
            job_failures: $raw.job_failures,
            login_required: $raw.login_required,
            weekly_report: $raw.weekly_report,
        })
    };
}

const fn enabled_by_default() -> bool {
    true
}

fn invalid(message: impl std::fmt::Display) -> ConfigError {
    ConfigError::Invalid(format!("[notify]: {message}"))
}

impl RawNotify {
    /// `None` if no channel is set up (or all are switched off).
    pub(crate) fn into_config(self) -> Result<Option<NotifyConfig>, ConfigError> {
        let report_schedule = CronSchedule::parse(
            self.weekly_report_schedule
                .as_deref()
                .unwrap_or(DEFAULT_REPORT_SCHEDULE),
        )
        .map_err(invalid)?;
        let mail = self
            .mail
            .filter(|mail| mail.enabled)
            .map(mail_config)
            .transpose()?;
        let webhook = self
            .webhook
            .filter(|webhook| webhook.enabled)
            .map(webhook_config)
            .transpose()?;
        if mail.is_none() && webhook.is_none() {
            return Ok(None);
        }
        Ok(Some(NotifyConfig {
            lang: match self.language {
                Some(RawLanguage::En) => Lang::En,
                Some(RawLanguage::De) | None => Lang::De,
            },
            report_schedule,
            mail,
            webhook,
        }))
    }
}

fn mail_config(raw: RawMail) -> Result<MailConfig, ConfigError> {
    if raw.host.trim().is_empty() {
        return Err(invalid("mail: host is empty"));
    }
    if raw.to.is_empty() {
        return Err(invalid("mail: to needs at least one recipient"));
    }
    if raw.password_file.is_some() && raw.username.is_none() {
        return Err(invalid("mail: password_file needs a username"));
    }
    let security = match raw.security.unwrap_or(RawSecurity::Starttls) {
        RawSecurity::Starttls => MailSecurity::StartTls,
        RawSecurity::Tls => MailSecurity::Tls,
        RawSecurity::None => MailSecurity::None,
    };
    let port = raw.port.unwrap_or(match security {
        MailSecurity::StartTls => PORT_STARTTLS,
        MailSecurity::Tls => PORT_TLS,
        MailSecurity::None => PORT_PLAIN,
    });
    Ok(MailConfig {
        host: raw.host,
        port,
        security,
        username: raw.username,
        password: SecretSource {
            credential: SMTP_PASSWORD_CREDENTIAL,
            file: raw.password_file,
            env: SMTP_PASSWORD_ENV,
        },
        from: raw.from,
        to: raw.to,
        topics: topics_of!(raw),
    })
}

fn webhook_config(raw: RawWebhook) -> Result<WebhookConfig, ConfigError> {
    if !(raw.url.starts_with("https://") || raw.url.starts_with("http://")) {
        return Err(invalid("webhook: url must start with https:// or http://"));
    }
    Ok(WebhookConfig {
        url: raw.url,
        format: match raw.format {
            RawFormat::Json => WebhookFormat::Json,
            RawFormat::Text => WebhookFormat::Text,
        },
        token: SecretSource {
            credential: WEBHOOK_TOKEN_CREDENTIAL,
            file: raw.token_file,
            env: WEBHOOK_TOKEN_ENV,
        },
        topics: topics_of!(raw),
    })
}

/// Where the process finds secrets (replaceable in tests).
pub trait SecretEnvironment {
    /// `$CREDENTIALS_DIRECTORY` of systemd.
    fn credentials_dir(&self) -> Option<PathBuf>;
    /// Value of an environment variable.
    fn var(&self, name: &str) -> Option<String>;
}

/// The real process environment.
pub struct ProcessEnvironment;

impl SecretEnvironment for ProcessEnvironment {
    fn credentials_dir(&self) -> Option<PathBuf> {
        std::env::var_os("CREDENTIALS_DIRECTORY").map(PathBuf::from)
    }

    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|value| !value.is_empty())
    }
}

impl SecretSource {
    /// Reads the secret: systemd credential, then the file, then the environment
    /// variable. `None` if none of them is set.
    ///
    /// # Errors
    /// [`ConfigError`] if a file is unreadable or (with a configured file)
    /// readable by group or others.
    pub fn read(
        &self,
        environment: &dyn SecretEnvironment,
    ) -> Result<Option<Zeroizing<String>>, ConfigError> {
        if let Some(dir) = environment.credentials_dir() {
            let path = dir.join(self.credential);
            if path.exists() {
                // systemd creates credentials with safe permissions itself.
                return read_secret_file(&path).map(Some);
            }
        }
        if let Some(path) = &self.file {
            ensure_private(path)?;
            return read_secret_file(path).map(Some);
        }
        Ok(environment.var(self.env).map(Zeroizing::new))
    }
}

fn read_secret_file(path: &Path) -> Result<Zeroizing<String>, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Unreadable {
        path: path.to_owned(),
        message: e.to_string(),
    })?;
    // A trailing line break from `echo` is not part of the secret.
    Ok(Zeroizing::new(
        text.trim_end_matches(['\r', '\n']).to_owned(),
    ))
}

#[cfg(unix)]
fn ensure_private(path: &Path) -> Result<(), ConfigError> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(path).map_err(|e| ConfigError::Unreadable {
        path: path.to_owned(),
        message: e.to_string(),
    })?;
    if metadata.permissions().mode() & GROUP_AND_OTHER_BITS != 0 {
        return Err(invalid(format!(
            "{} may only be readable by its owner (chmod 600)",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn ensure_private(_path: &Path) -> Result<(), ConfigError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Environment};

    fn parse(text: &str) -> Result<Option<NotifyConfig>, ConfigError> {
        Config::parse(text, &Environment::default()).map(|config| config.notify)
    }

    #[test]
    fn without_a_channel_nothing_is_sent() {
        assert_eq!(parse("").unwrap(), None);
        assert_eq!(parse("[notify]\nlanguage = \"en\"").unwrap(), None);
        let off = "[notify.webhook]\nenabled = false\nurl = \"https://x\"";
        assert_eq!(parse(off).unwrap(), None);
    }

    #[test]
    fn mail_defaults() {
        let config = parse(
            r#"
            [notify.mail]
            host = "smtp.example.org"
            from = "sync@example.org"
            to = ["me@example.org"]
            weekly_report = true
            sync_failures = false
            "#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(config.lang, Lang::De);
        assert_eq!(config.report_schedule.to_string(), DEFAULT_REPORT_SCHEDULE);
        let mail = config.mail.unwrap();
        assert_eq!((mail.security, mail.port), (MailSecurity::StartTls, 587));
        assert_eq!(
            mail.topics,
            Topics {
                conflicts: true,
                sync_failures: false,
                job_failures: true,
                login_required: true,
                weekly_report: true,
            }
        );
        assert!(config.webhook.is_none());
    }

    #[test]
    fn webhook_with_text_format() {
        let config = parse(
            r#"
            [notify]
            language = "en"
            weekly_report_schedule = "0 18 * * 5"

            [notify.webhook]
            url = "https://ntfy.sh/topic"
            format = "text"
            conflicts = false
            "#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(config.lang, Lang::En);
        let webhook = config.webhook.unwrap();
        assert_eq!(webhook.format, WebhookFormat::Text);
        assert!(!webhook.topics.conflicts);
        assert_eq!(webhook.token.env, WEBHOOK_TOKEN_ENV);
    }

    #[test]
    fn invalid_settings_are_rejected() {
        for text in [
            "[notify]\nweekly_report_schedule = \"every monday\"\n[notify.webhook]\nurl = \"https://x\"",
            "[notify.webhook]\nurl = \"ftp://x\"",
            "[notify.webhook]\nurl = \"https://x\"\nunknown = 1",
            "[notify.mail]\nhost = \"h\"\nfrom = \"a@b.c\"\nto = []",
            "[notify.mail]\nhost = \"h\"\nfrom = \"a@b.c\"\nto = [\"a@b.c\"]\npassword_file = \"/p\"",
            "[notify]\nlanguage = \"fr\"",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
    }

    struct TestEnvironment {
        credentials: Option<PathBuf>,
        value: Option<String>,
    }

    impl SecretEnvironment for TestEnvironment {
        fn credentials_dir(&self) -> Option<PathBuf> {
            self.credentials.clone()
        }
        fn var(&self, _name: &str) -> Option<String> {
            self.value.clone()
        }
    }

    fn source(file: Option<PathBuf>) -> SecretSource {
        SecretSource {
            credential: SMTP_PASSWORD_CREDENTIAL,
            file,
            env: SMTP_PASSWORD_ENV,
        }
    }

    #[test]
    fn secrets_come_from_credential_file_or_environment() {
        let dir = tempfile::tempdir().unwrap();
        let empty = TestEnvironment {
            credentials: None,
            value: None,
        };
        assert_eq!(source(None).read(&empty).unwrap(), None);

        let from_env = TestEnvironment {
            credentials: None,
            value: Some("env-secret".into()),
        };
        assert_eq!(
            source(None)
                .read(&from_env)
                .unwrap()
                .as_deref()
                .map(String::as_str),
            Some("env-secret")
        );

        let file = dir.path().join("password");
        std::fs::write(&file, "file-secret\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(source(Some(file.clone())).read(&from_env).is_err());
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert_eq!(
            source(Some(file.clone()))
                .read(&from_env)
                .unwrap()
                .as_deref()
                .map(String::as_str),
            Some("file-secret")
        );

        std::fs::write(dir.path().join(SMTP_PASSWORD_CREDENTIAL), "credential").unwrap();
        let systemd = TestEnvironment {
            credentials: Some(dir.path().to_owned()),
            value: Some("env-secret".into()),
        };
        assert_eq!(
            source(Some(file))
                .read(&systemd)
                .unwrap()
                .as_deref()
                .map(String::as_str),
            Some("credential")
        );
    }
}
