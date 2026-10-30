//! Writes changed settings back into the configuration file (ADR 0021).
//!
//! Only the changed keys are touched: comments, order and every other
//! setting stay as the user wrote them (`toml_edit` keeps the layout).
//! Before writing, the changed text is checked with the normal parser, so
//! the file never ends up in a state the daemon would refuse at startup.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use protonsync_api::{Direction, PairSettingsChange, SettingsChange};
use toml_edit::{DocumentMut, Item, Table, TableLike, Value, value};

use crate::config::{Config, Environment};
use crate::core::SettingsFile;

/// The configuration file of the daemon.
#[derive(Debug, Clone)]
pub struct ConfigFile {
    env: Environment,
}

impl ConfigFile {
    /// The file that `env` points to (see [`Environment::config_path`]).
    #[must_use]
    pub fn new(env: Environment) -> Self {
        Self { env }
    }

    fn file_path(&self) -> PathBuf {
        self.env.config_path()
    }

    fn read(&self) -> Result<String, String> {
        match fs::read_to_string(self.file_path()) {
            Ok(text) => Ok(text),
            // Without a file the defaults apply; the change creates it.
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e.to_string()),
        }
    }
}

impl SettingsFile for ConfigFile {
    fn path(&self) -> String {
        self.file_path().display().to_string()
    }

    fn writable(&self) -> bool {
        let path = self.file_path();
        match fs::OpenOptions::new().append(true).open(&path) {
            Ok(_) => true,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                path.parent().is_some_and(Path::is_dir)
            }
            Err(_) => false,
        }
    }

    fn save(&self, change: &SettingsChange) -> Result<(), String> {
        let text = self.read()?;
        let changed = apply_change(&text, change)?;
        Config::parse(&changed, &self.env).map_err(|e| e.to_string())?;
        write_file(&self.file_path(), &changed).map_err(|e| e.to_string())
    }
}

/// The TOML text after `change`.
///
/// # Errors
/// A message if the text is not valid TOML or a sync pair is missing.
pub fn apply_change(text: &str, change: &SettingsChange) -> Result<String, String> {
    let mut doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| e.to_string())?;
    if change.dry_run.is_some() || change.trash_retention_days.is_some() {
        let general = general_table(&mut doc)?;
        if let Some(dry_run) = change.dry_run {
            set_value(general, "dry_run", dry_run);
        }
        if let Some(days) = change.trash_retention_days {
            set_value(general, "trash_retention_days", i64::from(days));
        }
    }
    for pair in &change.pairs {
        apply_pair(pair_table(&mut doc, &pair.id)?, pair);
    }
    Ok(doc.to_string())
}

/// `[general]`; created at the top if missing.
fn general_table(doc: &mut DocumentMut) -> Result<&mut dyn TableLike, String> {
    if !doc.contains_key("general") {
        let mut table = Table::new();
        // Position 0: before all other tables, where readers expect it.
        table.set_position(Some(0));
        doc.insert("general", Item::Table(table));
    }
    doc.get_mut("general")
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| "`general` is not a table".to_owned())
}

/// The `[[pair]]` with the identifier `id`.
fn pair_table<'a>(doc: &'a mut DocumentMut, id: &str) -> Result<&'a mut Table, String> {
    doc.get_mut("pair")
        .and_then(Item::as_array_of_tables_mut)
        .and_then(|pairs| {
            pairs
                .iter_mut()
                .find(|table| table.get("id").and_then(Item::as_str) == Some(id))
        })
        .ok_or_else(|| format!("no [[pair]] with id {id:?} in the file"))
}

fn apply_pair(table: &mut dyn TableLike, change: &PairSettingsChange) {
    if let Some(direction) = change.direction {
        set_value(table, "direction", direction_key(direction));
    }
    if let Some(protect) = change.protect_remote {
        set_value(table, "protect_remote", protect);
    }
    if let Some(archive) = change.archive_versions {
        set_value(table, "archive_versions", archive);
    }
}

/// Sets `key`; an existing value keeps its spacing and trailing comment.
fn set_value(table: &mut dyn TableLike, key: &str, new: impl Into<Value>) {
    let mut new = new.into();
    match table.get_mut(key).and_then(Item::as_value_mut) {
        Some(old) => {
            *new.decor_mut() = old.decor().clone();
            *old = new;
        }
        None => {
            table.insert(key, value(new));
        }
    }
}

/// The value of `direction` in the file.
fn direction_key(direction: Direction) -> &'static str {
    match direction {
        Direction::TwoWay => "two_way",
        Direction::DownloadOnly => "download_only",
        Direction::UploadOnly => "upload_only",
        Direction::BackupOnly => "backup_only",
    }
}

/// Writes `text` so that a crash leaves either the old or the new file:
/// first a temporary file next to it, then a rename. If the folder is not
/// writable (only the file is), the file is overwritten in place.
fn write_file(path: &Path, text: &str) -> io::Result<()> {
    // A symlinked configuration (e.g. from a dotfiles repository) keeps its
    // link; the file it points to is changed.
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let temp = temp_path(&path);
    match write_new(&temp, &path, text) {
        Ok(()) => fs::rename(&temp, &path).inspect_err(|_| {
            let _ = fs::remove_file(&temp);
        }),
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied && path.exists() => {
            fs::write(&path, text)
        }
        Err(e) => Err(e),
    }
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".protonsync-tmp");
    path.with_file_name(name)
}

/// Creates `temp` with the permissions of `original` (new files: only the owner).
fn write_new(temp: &Path, original: &Path, text: &str) -> io::Result<()> {
    let mut file = fs::File::create(temp)?;
    match fs::metadata(original) {
        Ok(metadata) => file.set_permissions(metadata.permissions())?,
        Err(_) => restrict_to_owner(&file)?,
    }
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

#[cfg(unix)]
fn restrict_to_owner(file: &fs::File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    /// Read and write for the owner only.
    const OWNER_ONLY: u32 = 0o600;
    file.set_permissions(fs::Permissions::from_mode(OWNER_ONLY))
}

#[cfg(not(unix))]
fn restrict_to_owner(_file: &fs::File) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = r#"# My configuration
[general]
dry_run = true   # first try

[[pair]]
id = "docs"
local = "/home/k/Docs"
# two_way is the default

[[pair]]
id = "photos"
local = "/home/k/Pictures"
direction = "download_only"
"#;

    fn pair_change(id: &str) -> PairSettingsChange {
        PairSettingsChange::new(id)
    }

    #[test]
    fn only_the_changed_keys_change_and_comments_stay() {
        let change = SettingsChange {
            dry_run: Some(false),
            trash_retention_days: Some(14),
            pairs: vec![PairSettingsChange {
                direction: Some(Direction::BackupOnly),
                archive_versions: Some(true),
                ..pair_change("photos")
            }],
        };
        let changed = apply_change(TEXT, &change).unwrap();
        assert!(changed.contains("# My configuration"), "{changed}");
        assert!(changed.contains("# first try"), "{changed}");
        assert!(changed.contains("# two_way is the default"), "{changed}");
        assert!(changed.contains("dry_run = false"), "{changed}");
        assert!(changed.contains("trash_retention_days = 14"), "{changed}");
        assert!(changed.contains("direction = \"backup_only\""), "{changed}");
        assert!(changed.contains("archive_versions = true"), "{changed}");
        let config = Config::parse(&changed, &Environment::default()).unwrap();
        assert!(!config.dry_run);
        assert_eq!(config.retention.in_days(), 14);
        assert_eq!(
            config.pairs[0].direction,
            protonsync_domain::sync_pair::SyncDirection::TwoWay
        );
        assert_eq!(
            config.pairs[1].direction,
            protonsync_domain::sync_pair::SyncDirection::BackupOnly
        );
        assert!(config.pairs[1].archive_versions);
    }

    #[test]
    fn a_missing_general_table_is_created_at_the_top() {
        let text = "[[pair]]\nid = \"docs\"\nlocal = \"/d\"\n";
        let change = SettingsChange {
            dry_run: Some(false),
            ..SettingsChange::default()
        };
        let changed = apply_change(text, &change).unwrap();
        assert!(changed.starts_with("[general]\ndry_run = false"), "{changed}");
        assert!(!Config::parse(&changed, &Environment::default()).unwrap().dry_run);
    }

    #[test]
    fn an_unknown_pair_is_refused() {
        let change = SettingsChange {
            pairs: vec![PairSettingsChange {
                protect_remote: Some(true),
                ..pair_change("nope")
            }],
            ..SettingsChange::default()
        };
        let error = apply_change(TEXT, &change).unwrap_err();
        assert!(error.contains("nope"), "{error}");
    }

    fn file_in(dir: &Path, text: &str) -> ConfigFile {
        let path = dir.join("config.toml");
        if !text.is_empty() {
            fs::write(&path, text).unwrap();
        }
        ConfigFile::new(Environment {
            config_file: Some(path),
            ..Environment::default()
        })
    }

    #[test]
    fn save_writes_the_file_and_keeps_its_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let file = file_in(dir.path(), TEXT);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir.path().join("config.toml"), fs::Permissions::from_mode(0o640))
                .unwrap();
        }
        assert!(file.writable());
        file.save(&SettingsChange {
            dry_run: Some(false),
            ..SettingsChange::default()
        })
        .unwrap();
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("dry_run = false   # first try"), "{text}");
        assert!(!dir.path().join("config.toml.protonsync-tmp").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.path().join("config.toml"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o640);
        }
    }

    #[test]
    fn save_creates_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = file_in(dir.path(), "");
        assert!(file.writable());
        file.save(&SettingsChange {
            trash_retention_days: Some(7),
            ..SettingsChange::default()
        })
        .unwrap();
        let text = fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("trash_retention_days = 7"), "{text}");
    }

    #[test]
    fn an_invalid_result_is_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let file = file_in(dir.path(), TEXT);
        let error = file
            .save(&SettingsChange {
                trash_retention_days: Some(0),
                ..SettingsChange::default()
            })
            .unwrap_err();
        assert!(!error.is_empty());
        assert_eq!(
            fs::read_to_string(dir.path().join("config.toml")).unwrap(),
            TEXT
        );
    }

    #[test]
    fn a_missing_folder_is_not_writable() {
        let file = ConfigFile::new(Environment {
            config_file: Some("/does/not/exist/config.toml".into()),
            ..Environment::default()
        });
        assert!(!file.writable());
    }
}
