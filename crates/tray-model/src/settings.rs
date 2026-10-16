//! Eigene Einstellungen der Tray-App, Autostart und wo die Dateien liegen.
//!
//! Die Einstellungen des Daemons gehören nicht hierher: Die Tray-App zeigt
//! sie nur an und öffnet auf Wunsch dessen Konfigurationsdatei (ADR 0011).

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use protonsync_client::Lang;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Name der Autostart-Datei.
const AUTOSTART_FILE: &str = "protonsync-tray.desktop";
/// Name der Einstellungsdatei im Konfigurationsordner von protonsync.
const SETTINGS_FILE: &str = "tray.toml";

/// Fehler beim Lesen oder Schreiben der Einstellungen.
#[derive(Debug, Error)]
pub enum SettingsError {
    /// Datei nicht lesbar oder nicht schreibbar.
    #[error("{path}: {message}")]
    Io {
        /// Betroffene Datei.
        path: PathBuf,
        /// Meldung des Betriebssystems.
        message: String,
    },
    /// Inhalt ist kein gültiges TOML für diese Einstellungen.
    #[error("{path}: ungültige Einstellungen: {message}")]
    Invalid {
        /// Betroffene Datei.
        path: PathBuf,
        /// Meldung des Parsers.
        message: String,
    },
}

fn io_error(path: &Path) -> impl Fn(io::Error) -> SettingsError + '_ {
    move |e| SettingsError::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    }
}

/// Gewählte Sprache der Oberfläche.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageChoice {
    /// Wie die Umgebung (`LC_ALL`, `LC_MESSAGES`, `LANG`).
    #[default]
    Auto,
    /// Deutsch.
    De,
    /// Englisch.
    En,
}

impl LanguageChoice {
    /// Die tatsächlich verwendete Sprache; `system` gilt bei [`Self::Auto`].
    #[must_use]
    pub fn resolve(self, system: Lang) -> Lang {
        match self {
            Self::Auto => system,
            Self::De => Lang::De,
            Self::En => Lang::En,
        }
    }

    /// Aus dem Text der Oberfläche (`auto`, `de`, `en`).
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

/// Einstellungen der Tray-App (`tray.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TraySettings {
    /// Sprache.
    pub language: LanguageChoice,
    /// Desktop-Benachrichtigungen anzeigen.
    pub notifications: bool,
}

impl Default for TraySettings {
    fn default() -> Self {
        Self {
            language: LanguageChoice::Auto,
            notifications: true,
        }
    }
}

impl TraySettings {
    /// Lädt die Einstellungen; fehlt die Datei, gelten die Standardwerte.
    ///
    /// # Errors
    /// [`SettingsError`] bei unlesbarer oder ungültiger Datei.
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

    /// Speichert die Einstellungen (legt den Ordner bei Bedarf an).
    ///
    /// # Errors
    /// [`SettingsError::Io`], wenn die Datei nicht geschrieben werden kann.
    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        let text = toml::to_string(self).map_err(|e| SettingsError::Invalid {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        write_file(path, &text)
    }
}

/// Schreibt erst eine Nachbardatei und benennt sie dann um, damit ein
/// Absturz nie eine halbe Datei hinterlässt.
fn write_file(path: &Path, text: &str) -> Result<(), SettingsError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io_error(dir))?;
    }
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, text).map_err(io_error(&temporary))?;
    std::fs::rename(&temporary, path).map_err(io_error(path))
}

/// Wo die Dateien der Tray-App liegen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayPaths {
    /// `tray.toml`.
    pub settings: PathBuf,
    /// Ordner für Autostart-Einträge (`~/.config/autostart`).
    pub autostart_dir: PathBuf,
    /// Konfigurationsdatei des Daemons (zum Öffnen im Editor).
    pub daemon_config: PathBuf,
}

impl TrayPaths {
    /// Aus den Umgebungsvariablen des Prozesses, nach denselben Regeln wie
    /// der Daemon (`PROTONSYNC_CONFIG`, `XDG_CONFIG_HOME`, `HOME`).
    #[must_use]
    pub fn from_process() -> Self {
        let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
        Self::from_env(
            var("PROTONSYNC_CONFIG"),
            var("XDG_CONFIG_HOME"),
            var("HOME"),
        )
    }

    /// Aus gegebenen Werten (für Tests).
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

    /// Pfad des Autostart-Eintrags.
    #[must_use]
    pub fn autostart_file(&self) -> PathBuf {
        self.autostart_dir.join(AUTOSTART_FILE)
    }
}

/// Ob die Tray-App beim Anmelden startet (der Eintrag existiert).
#[must_use]
pub fn autostart_enabled(paths: &TrayPaths) -> bool {
    paths.autostart_file().is_file()
}

/// Legt den Autostart-Eintrag an (startet `program`) oder entfernt ihn.
///
/// # Errors
/// [`SettingsError::Io`], wenn der Eintrag nicht geschrieben oder gelöscht
/// werden kann.
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

/// Autostart-Eintrag nach der freedesktop-Spezifikation.
fn desktop_entry(program: &Path) -> String {
    let mut entry = String::from("[Desktop Entry]\nType=Application\nName=protonsync\n");
    entry.push_str("Comment=Inoffizieller Proton-Drive-Client (Tray)\n");
    entry.push_str("Comment[en]=Unofficial Proton Drive client (tray)\n");
    let _ = writeln!(entry, "Exec={}", quote_exec(&program.to_string_lossy()));
    entry.push_str("Terminal=false\nX-GNOME-Autostart-enabled=true\n");
    entry
}

/// Setzt den Programmpfad für `Exec=` in Anführungszeichen; `"`, `` ` ``,
/// `$` und `\` werden dabei maskiert (freedesktop „Exec key“).
fn quote_exec(program: &str) -> String {
    let mut quoted = String::from("\"");
    for c in program.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    // In .desktop-Dateien ist `\` selbst noch einmal zu maskieren.
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
        };
        settings.save(&path).unwrap();
        assert_eq!(TraySettings::load(&path).unwrap(), settings);
        assert!(!path.with_extension("tmp").exists());
    }

    #[test]
    fn invalid_files_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tray.toml");
        std::fs::write(&path, "sprache = 'klingonisch'\n").unwrap();
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
        set_autostart(&paths, false, Path::new("egal")).unwrap();
        assert!(!autostart_enabled(&paths));
        // Zweimal ausschalten ist kein Fehler.
        set_autostart(&paths, false, Path::new("egal")).unwrap();
    }

    #[test]
    fn exec_paths_are_quoted() {
        assert_eq!(quote_exec("/opt/a b/x"), "\"/opt/a b/x\"");
        assert_eq!(quote_exec("/opt/$x\"y"), "\"/opt/\\\\$x\\\\\"y\"");
    }
}
