//! Lokales Dateisystem (Adapter für den Port [`LocalStorage`]).
//!
//! - Dateien werden zuerst als versteckte Hilfsdatei im Zielordner geschrieben
//!   und dann atomar umbenannt; es gibt nie halb geschriebene Dateien.
//! - Der lokale Papierkorb liegt in `.protonsync-trash` im Wurzelordner. So liegt
//!   er sicher auf demselben Dateisystem, und Verschieben ist ein schnelles Umbenennen.
//! - Einträge, deren Name mit [`INTERNAL_PREFIX`] beginnt, sind Hilfsdateien
//!   und tauchen im Scan nicht auf. Symbolische Links werden nicht verfolgt.

mod watch;

pub use watch::LocalWatcher;

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use protonsync_application::ports::{LocalEntry, LocalError, LocalKind, LocalStorage};
use protonsync_domain::node::{Fingerprint, NodeName};
use protonsync_domain::path::RelativePath;
use protonsync_domain::time::Timestamp;
use sha1::{Digest, Sha1};
use tempfile::NamedTempFile;

/// Namensanfang aller eigenen Hilfsdateien und -ordner.
pub const INTERNAL_PREFIX: &str = ".protonsync-";
const TRASH_FOLDER: &str = ".protonsync-trash";
const TEMP_PREFIX: &str = ".protonsync-tmp-";
/// Name des Inhalts innerhalb eines Papierkorbeintrags.
const TRASHED_CONTENT: &str = "content";
const HASH_BUFFER_BYTES: usize = 64 * 1024;

/// Lokaler Wurzelordner eines Sync-Paares.
#[derive(Debug, Clone)]
pub struct FsLocal {
    root: PathBuf,
}

impl FsLocal {
    /// Benutzt `root` als Wurzelordner; er muss existieren.
    ///
    /// # Errors
    /// [`LocalError::NotFound`], wenn `root` kein Ordner ist.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, LocalError> {
        let root = root.into();
        if root.is_dir() {
            Ok(Self { root })
        } else {
            Err(LocalError::NotFound(RelativePath::root()))
        }
    }

    /// Der Wurzelordner.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn absolute(&self, path: &RelativePath) -> PathBuf {
        self.root.join(path.to_path_buf())
    }

    fn trash_dir(&self) -> PathBuf {
        self.root.join(TRASH_FOLDER)
    }

    fn trash_entry_dir(&self, stored_as: &str) -> Result<PathBuf, LocalError> {
        // Der Schlüssel stammt von uns, wird aber trotzdem geprüft, damit ein
        // beschädigter Zustand nie aus dem Papierkorb herausführt.
        NodeName::new(stored_as).map_err(|error| LocalError::Io(error.to_string()))?;
        Ok(self.trash_dir().join(stored_as))
    }

    fn scan_folder(
        &self,
        folder: &RelativePath,
        entries: &mut Vec<LocalEntry>,
    ) -> Result<(), LocalError> {
        let mut children = Vec::new();
        let listing = fs::read_dir(self.absolute(folder)).map_err(|e| map_io(&e, folder))?;
        for child in listing {
            let child = child.map_err(|e| map_io(&e, folder))?;
            let Some(name) = child.file_name().to_str().map(str::to_owned) else {
                continue; // Namen, die kein UTF-8 sind, kann Proton nicht abbilden.
            };
            if name.starts_with(INTERNAL_PREFIX) {
                continue;
            }
            let Ok(name) = NodeName::new(name) else {
                continue;
            };
            let file_type = child.file_type().map_err(|e| map_io(&e, folder))?;
            if file_type.is_symlink() {
                continue;
            }
            children.push((folder.join(&name), file_type.is_dir()));
        }
        children.sort();
        for (path, is_dir) in children {
            if is_dir {
                entries.push(LocalEntry {
                    path: path.clone(),
                    kind: LocalKind::Folder,
                });
                self.scan_folder(&path, entries)?;
            } else if let Some(entry) = self.entry(&path)? {
                entries.push(entry);
            }
        }
        Ok(())
    }

    fn require_parent(&self, path: &RelativePath) -> Result<PathBuf, LocalError> {
        let parent = path
            .parent()
            .ok_or_else(|| LocalError::NotFound(path.clone()))?;
        let absolute = self.absolute(&parent);
        match fs::metadata(&absolute) {
            Ok(metadata) if metadata.is_dir() => Ok(absolute),
            Ok(_) => Err(LocalError::NotAFolder(parent)),
            Err(error) => Err(map_io(&error, &parent)),
        }
    }
}

fn map_io(error: &io::Error, path: &RelativePath) -> LocalError {
    match error.kind() {
        io::ErrorKind::NotFound => LocalError::NotFound(path.clone()),
        io::ErrorKind::AlreadyExists => LocalError::AlreadyExists(path.clone()),
        _ => LocalError::Io(format!("{path}: {error}")),
    }
}

fn to_timestamp(time: SystemTime) -> Timestamp {
    let seconds = match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_secs()).unwrap_or(i64::MAX),
        Err(before) => -i64::try_from(before.duration().as_secs()).unwrap_or(i64::MAX),
    };
    Timestamp::from_unix_seconds(seconds)
}

fn to_system_time(timestamp: Timestamp) -> SystemTime {
    let seconds = timestamp.unix_seconds();
    let offset = Duration::from_secs(seconds.unsigned_abs());
    if seconds >= 0 {
        UNIX_EPOCH + offset
    } else {
        UNIX_EPOCH - offset
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            // Schreiben in einen String kann nicht fehlschlagen.
            let _ = write!(out, "{byte:02x}");
            out
        })
}

impl LocalStorage for FsLocal {
    type Reader = File;
    type Staging = NamedTempFile;

    fn scan(&self) -> Result<Vec<LocalEntry>, LocalError> {
        let mut entries = Vec::new();
        self.scan_folder(&RelativePath::root(), &mut entries)?;
        Ok(entries)
    }

    fn entry(&self, path: &RelativePath) -> Result<Option<LocalEntry>, LocalError> {
        let metadata = match fs::symlink_metadata(self.absolute(path)) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(map_io(&error, path)),
        };
        let kind = if metadata.is_dir() {
            LocalKind::Folder
        } else if metadata.is_file() {
            let modified = metadata.modified().map_err(|e| map_io(&e, path))?;
            LocalKind::File(Fingerprint {
                size: metadata.len(),
                modified: to_timestamp(modified),
                content_hash: None,
            })
        } else {
            return Ok(None); // Symlinks, Geräte, Sockets werden nicht synchronisiert.
        };
        Ok(Some(LocalEntry {
            path: path.clone(),
            kind,
        }))
    }

    fn open(&self, path: &RelativePath) -> Result<Self::Reader, LocalError> {
        File::open(self.absolute(path)).map_err(|e| map_io(&e, path))
    }

    fn content_hash(&self, path: &RelativePath) -> Result<String, LocalError> {
        let mut file = self.open(path)?;
        let mut hasher = Sha1::new();
        let mut buffer = vec![0; HASH_BUFFER_BYTES];
        loop {
            let read = file.read(&mut buffer).map_err(|e| map_io(&e, path))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(hex(&hasher.finalize()))
    }

    fn stage(&self, path: &RelativePath) -> Result<Self::Staging, LocalError> {
        let parent = self.require_parent(path)?;
        tempfile::Builder::new()
            .prefix(TEMP_PREFIX)
            .tempfile_in(parent)
            .map_err(|e| map_io(&e, path))
    }

    fn commit(
        &self,
        staging: Self::Staging,
        path: &RelativePath,
        modified: Timestamp,
    ) -> Result<Fingerprint, LocalError> {
        let file = staging.as_file();
        file.sync_all().map_err(|e| map_io(&e, path))?;
        file.set_modified(to_system_time(modified))
            .map_err(|e| map_io(&e, path))?;
        let size = file.metadata().map_err(|e| map_io(&e, path))?.len();
        staging
            .persist(self.absolute(path))
            .map_err(|e| map_io(&e.error, path))?;
        Ok(Fingerprint {
            size,
            modified,
            content_hash: None,
        })
    }

    fn create_folder(&self, path: &RelativePath) -> Result<(), LocalError> {
        self.require_parent(path)?;
        match fs::create_dir(self.absolute(path)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if self.absolute(path).is_dir() {
                    Ok(())
                } else {
                    Err(LocalError::NotAFolder(path.clone()))
                }
            }
            Err(error) => Err(map_io(&error, path)),
        }
    }

    fn rename(&self, from: &RelativePath, to: &RelativePath) -> Result<(), LocalError> {
        if self.entry(from)?.is_none() {
            return Err(LocalError::NotFound(from.clone()));
        }
        if fs::symlink_metadata(self.absolute(to)).is_ok() {
            return Err(LocalError::AlreadyExists(to.clone()));
        }
        self.require_parent(to)?;
        fs::rename(self.absolute(from), self.absolute(to)).map_err(|e| map_io(&e, from))
    }

    fn move_to_trash(&self, path: &RelativePath) -> Result<String, LocalError> {
        if self.entry(path)?.is_none() {
            return Err(LocalError::NotFound(path.clone()));
        }
        let trash = self.trash_dir();
        fs::create_dir_all(&trash).map_err(|e| map_io(&e, path))?;
        let entry_dir = tempfile::Builder::new()
            .prefix("t")
            .tempdir_in(&trash)
            .map_err(|e| map_io(&e, path))?
            .keep();
        let stored_as = entry_dir
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .ok_or_else(|| LocalError::Io("ungültiger Papierkorbname".into()))?;
        fs::rename(self.absolute(path), entry_dir.join(TRASHED_CONTENT))
            .map_err(|e| map_io(&e, path))?;
        Ok(stored_as)
    }

    fn restore_from_trash(&self, stored_as: &str, path: &RelativePath) -> Result<(), LocalError> {
        let entry_dir = self.trash_entry_dir(stored_as)?;
        let content = entry_dir.join(TRASHED_CONTENT);
        if fs::symlink_metadata(&content).is_err() {
            return Err(LocalError::NotFound(path.clone()));
        }
        if fs::symlink_metadata(self.absolute(path)).is_ok() {
            return Err(LocalError::AlreadyExists(path.clone()));
        }
        self.require_parent(path)?;
        fs::rename(&content, self.absolute(path)).map_err(|e| map_io(&e, path))?;
        fs::remove_dir(&entry_dir).map_err(|e| map_io(&e, path))
    }

    fn purge_trash(&self, stored_as: &str) -> Result<(), LocalError> {
        let entry_dir = self.trash_entry_dir(stored_as)?;
        fs::remove_dir_all(&entry_dir).map_err(|e| map_io(&e, &RelativePath::root()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_roundtrip_through_system_time() {
        for seconds in [0, 1_760_000_000, -86_400] {
            let timestamp = Timestamp::from_unix_seconds(seconds);
            assert_eq!(to_timestamp(to_system_time(timestamp)), timestamp);
        }
    }

    #[test]
    fn rejects_missing_root() {
        assert!(FsLocal::new("/gibt/es/wirklich/nicht").is_err());
    }

    #[test]
    fn trash_keys_cannot_escape_the_trash() {
        let dir = tempfile::tempdir().unwrap();
        let local = FsLocal::new(dir.path()).unwrap();
        assert!(local.purge_trash("../..").is_err());
        assert!(local.purge_trash("a/b").is_err());
    }
}
