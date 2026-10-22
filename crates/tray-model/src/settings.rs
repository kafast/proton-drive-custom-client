//! The tray app's own settings, autostart and where the files live.
//!
//! The daemon's settings do not belong here: the tray app only shows them
//! and opens the daemon's configuration file on request (ADR 0011).

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use protonsync_client::Lang;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::notify::NotificationTopics;

/// Name of the autostart file.
const AUTOSTART_FILE: &str = "protonsync-tray.desktop";
/// Name of the settings file in the protonsync configuration folder.
const SETTINGS_FILE: &str = "tray.toml";

/// Error while reading or writing the settings.
#[derive(Debug, Error)]
pub enum SettingsError {
    /// File not readable or not writable.
    #[error("{path}: {message}")]
    Io {
        /// Affected file.
        path: PathBuf,
        /// Message from the operating system.
        message: String,
    },
    /// Content is not valid TOML for these settings.
    #[error("{path}: invalid settings: {message}")]
    Invalid {
        /// Affected file.
        path: PathBuf,
        /// Message from the parser.
        message: String,
    },
}

fn io_error(path: &Path) -> impl Fn(io::Error) -> SettingsError + '_ {
    move |e| SettingsError::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    }
}

/// Chosen UI language.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageChoice {
    /// Same as the environment (`LC_ALL`, `LC_MESSAGES`, `LANG`).
    #[default]
    Auto,
    /// German.
    De,
    /// English.
    En,
}

impl LanguageChoice {
    /// The language actually used; `system` applies for [`Self::Auto`].
    #[must_use]
    pub fn resolve(self, system: Lang) -> Lang {
        match self {
            Self::Auto => system,
            Self::De => Lang::De,
            Self::En => Lang::En,
        }
    }

    /// From the UI text (`auto`, `de`, `en`).
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "de" => Some(Self::De),
            "en" => Some(Self::En),
            _ => None,
        }
    }
}

/// Settings of the tray app (`tray.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TraySettings {
    /// Language.
    pub language: LanguageChoice,
    /// Show desktop notifications.
    pub notifications: bool,
    /// Which topics notify (all by default).
    pub notification_topics: NotificationTopics,
}

impl Default for TraySettings {
    fn default() -> Self {
        Self {
            language: LanguageChoice::Auto,
            notifications: true,
            notification_topics: NotificationTopics::default(),
        }
    }
}

impl TraySettings {
    /// Loads the settings; if the file is missing, the defaults apply.
    ///
    /// # Errors
    /// [`SettingsError`] for an unreadable or invalid file.
    pub fn load(path: &Path) -> Result<Self, SettingsError> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| SettingsError::Invalid {
                path: path.to_path_buf(),
                message: e.to_string(),
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(io_error(path)(e)),
        }
    }

    /// Saves the settings (creates the folder if needed).
    ///
    /// # Errors
    /// [`SettingsError::Io`] if the file cannot be written.
    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        let text = toml::to_string(self).map_err(|e| SettingsError::Invalid {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        write_file(path, &text)
    }
}

/// Writes a sibling file first and then renames it, so that a crash
/// never leaves a half-written file behind.
fn write_file(path: &Path, text: &str) -> Result<(), SettingsError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io_error(dir))?;
    }
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, text).map_err(io_error(&temporary))?;
    std::fs::rename(&temporary, path).map_err(io_error(path))
}

/// Where the tray app's files live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayPaths {
    /// `tray.toml`.
    pub settings: PathBuf,
    /// Folder for autostart entries (`~/.config/autostart`).
    pub autostart_dir: PathBuf,
    /// The daemon's configuration file (to open in an editor).
    pub daemon_config: PathBuf,
}

impl TrayPaths {
    /// From the process environment variables, by the same rules as the
    /// daemon (`PROTONSYNC_CONFIG`, `XDG_CONFIG_HOME`, `HOME`).
    #[must_use]
    pub fn from_process() -> Self {
        let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
        Self::from_env(
            var("PROTONSYNC_CONFIG"),
            var("XDG_CONFIG_HOME"),
            var("HOME"),
        )
    }

    /// From given values (for tests).
    #[must_use]
    pub fn from_env(
        daemon_config: Option<PathBuf>,
        xdg_config_home: Option<PathBuf>,
        home: Option<PathBuf>,
    ) -> Self {
        let config_home = xdg_config_home
            .unwrap_or_else(|| home.unwrap_or_else(|| PathBuf::from("/")).join(".config"));
        let protonsync = config_home.join("protonsync");
        Self {
            settings: protonsync.join(SETTINGS_FILE),
            autostart_dir: config_home.join("autostart"),
            daemon_config: daemon_config.unwrap_or_else(|| protonsync.join("config.toml")),
        }
    }

    /// Path of the autostart entry.
    #[must_use]
    pub fn autostart_file(&self) -> PathBuf {
        self.autostart_dir.join(AUTOSTART_FILE)
    }
}

/// Whether the tray app starts at login (the entry exists).
#[must_use]
pub fn autostart_enabled(paths: &TrayPaths) -> bool {
    paths.autostart_file().is_file()
}

/// Creates the autostart entry (starts `program`) or removes it.
///
/// # Errors
/// [`SettingsError::Io`] if the entry cannot be written or deleted.
pub fn set_autostart(
    paths: &TrayPaths,
    enabled: bool,
    program: &Path,
) -> Result<(), SettingsError> {
    let file = paths.autostart_file();
    if enabled {
        write_file(&file, &desktop_entry(program))
    } else {
        match std::fs::remove_file(&file) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_error(&file)(e)),
            _ => Ok(()),
        }
    }
}

/// Autostart entry according to the freedesktop specification.
fn desktop_entry(program: &Path) -> String {
    let mut entry = String::from("[Desktop Entry]\nType=Application\nName=protonsync\n");
    entry.push_str("Comment=Inoffizieller Proton-Drive-Client (Tray)\n");
    entry.push_str("Comment[en]=Unofficial Proton Drive client (tray)\n");
    let _ = writeln!(entry, "Exec={}", quote_exec(&program.to_string_lossy()));
    entry.push_str("Terminal=false\nX-GNOME-Autostart-enabled=true\n");
    entry
}

/// Puts the program path for `Exec=` in quotes; `"`, `` ` ``, `$` and
/// `\` are escaped (freedesktop "Exec key").
fn quote_exec(program: &str) -> String {
    let mut quoted = String::from("\"");
    for c in program.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    // In .desktop files, `\` itself has to be escaped once more.
    quoted.replace('\\', "\\\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults_and_saving_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("protonsync/tray.toml");
        assert_eq!(TraySettings::load(&path).unwrap(), TraySettings::default());

        let settings = TraySettings {
            language: LanguageChoice::En,
            notifications: false,
            notification_topics: NotificationTopics {
                sync_failures: false,
                ..NotificationTopics::default()
            },
        };
        settings.save(&path).unwrap();
        assert_eq!(TraySettings::load(&path).unwrap(), settings);
        assert!(!path.with_extension("tmp").exists());
    }

    #[test]
    fn invalid_files_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tray.toml");
        std::fs::write(&path, "lingo = 'klingon'\n").unwrap();
        assert!(matches!(
            TraySettings::load(&path),
            Err(SettingsError::Invalid { .. })
        ));
    }

    #[test]
    fn language_choice() {
        assert_eq!(LanguageChoice::Auto.resolve(Lang::De), Lang::De);
        assert_eq!(LanguageChoice::En.resolve(Lang::De), Lang::En);
        assert_eq!(LanguageChoice::De.resolve(Lang::En), Lang::De);
        assert_eq!(LanguageChoice::parse("de"), Some(LanguageChoice::De));
        assert_eq!(LanguageChoice::parse("fr"), None);
    }

    #[test]
    fn paths_follow_the_daemon_rules() {
        let home = TrayPaths::from_env(None, None, Some("/home/k".into()));
        assert_eq!(
            home.settings,
            PathBuf::from("/home/k/.config/protonsync/tray.toml")
        );
        assert_eq!(
            home.daemon_config,
            PathBuf::from("/home/k/.config/protonsync/config.toml")
        );
        assert_eq!(
            home.autostart_file(),
            PathBuf::from("/home/k/.config/autostart/protonsync-tray.desktop")
        );
        let explicit = TrayPaths::from_env(Some("/etc/p.toml".into()), Some("/x".into()), None);
        assert_eq!(explicit.daemon_config, PathBuf::from("/etc/p.toml"));
        assert_eq!(explicit.settings, PathBuf::from("/x/protonsync/tray.toml"));
    }

    #[test]
    fn autostart_entry_is_written_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let paths = TrayPaths::from_env(None, Some(dir.path().into()), None);
        assert!(!autostart_enabled(&paths));
        set_autostart(
            &paths,
            true,
            Path::new("/home/k/.cargo/bin/protonsync-tray"),
        )
        .unwrap();
        assert!(autostart_enabled(&paths));
        let entry = std::fs::read_to_string(paths.autostart_file()).unwrap();
        assert!(entry.starts_with("[Desktop Entry]\n"), "{entry}");
        assert!(
            entry.contains("Exec=\"/home/k/.cargo/bin/protonsync-tray\"\n"),
            "{entry}"
        );
        set_autostart(&paths, false, Path::new("unused")).unwrap();
        assert!(!autostart_enabled(&paths));
        // Turning it off twice is not an error.
        set_autostart(&paths, false, Path::new("unused")).unwrap();
    }

    #[test]
    fn exec_paths_are_quoted() {
        assert_eq!(quote_exec("/opt/a b/x"), "\"/opt/a b/x\"");
        assert_eq!(quote_exec("/opt/$x\"y"), "\"/opt/\\\\$x\\\\\"y\"");
    }
}
