//! Zwischenspeicher des virtuellen Laufwerks (Adapter für den Port [`ContentCache`]).
//!
//! Jeder Inhalt ist eine eigene Datei `<Ordner>/<Schlüssel>`. Die Dateien sind
//! dünn belegt („sparse“): Noch nicht geladene Blöcke belegen keinen Platz auf
//! der Platte. Ordner und Dateien sind nur für den eigenen Benutzer zugänglich,
//! denn sie enthalten entschlüsselte Inhalte.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use protonsync_application::ports::{CacheKey, ContentCache, LocalError};

/// Inhalte im Ordner `dir` (z. B. `<Datenverzeichnis>/cache/<Laufwerk>`).
#[derive(Debug, Clone)]
pub struct FsContentCache {
    dir: PathBuf,
}

#[allow(clippy::needless_pass_by_value)] // Signatur passt zu `map_err`.
fn io_error(error: io::Error) -> LocalError {
    LocalError::Io(error.to_string())
}

impl FsContentCache {
    /// Benutzt (und erzeugt bei Bedarf) den Ordner `dir` mit Rechten 0700.
    ///
    /// # Errors
    /// [`LocalError::Io`], wenn der Ordner nicht angelegt werden kann.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, LocalError> {
        let dir = dir.into();
        fs::create_dir_all(&dir).map_err(io_error)?;
        restrict_folder(&dir)?;
        Ok(Self { dir })
    }

    /// Der Ordner mit den Inhalten.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn file_path(&self, key: CacheKey) -> PathBuf {
        self.dir.join(key.0.to_string())
    }

    /// Öffnet zum Schreiben; neue Dateien bekommen Rechte 0600.
    fn open_for_writing(&self, key: CacheKey) -> Result<File, LocalError> {
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(false);
        restrict_new_file(&mut options);
        options.open(self.file_path(key)).map_err(io_error)
    }
}

#[cfg(unix)]
fn restrict_folder(dir: &Path) -> Result<(), LocalError> {
    use std::os::unix::fs::PermissionsExt;
    const OWNER_ONLY: u32 = 0o700;
    fs::set_permissions(dir, fs::Permissions::from_mode(OWNER_ONLY)).map_err(io_error)
}

#[cfg(not(unix))]
fn restrict_folder(_dir: &Path) -> Result<(), LocalError> {
    Ok(())
}

#[cfg(unix)]
fn restrict_new_file(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    const OWNER_READ_WRITE: u32 = 0o600;
    options.mode(OWNER_READ_WRITE);
}

#[cfg(not(unix))]
fn restrict_new_file(_options: &mut OpenOptions) {}

impl ContentCache for FsContentCache {
    type Reader = File;

    fn read(&self, key: CacheKey, offset: u64, length: usize) -> Result<Vec<u8>, LocalError> {
        let mut file = match File::open(self.file_path(key)) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(io_error(error)),
        };
        file.seek(SeekFrom::Start(offset)).map_err(io_error)?;
        let mut content = Vec::with_capacity(length);
        // `take` begrenzt auf `length`; `read_to_end` liest bis dahin oder bis zum Dateiende.
        let limit = u64::try_from(length).unwrap_or(u64::MAX);
        file.take(limit)
            .read_to_end(&mut content)
            .map_err(io_error)?;
        Ok(content)
    }

    fn write(&self, key: CacheKey, offset: u64, data: &[u8]) -> Result<(), LocalError> {
        let mut file = self.open_for_writing(key)?;
        // Schreiben hinter dem Ende lässt eine Lücke, die als Nullen gelesen wird.
        file.seek(SeekFrom::Start(offset)).map_err(io_error)?;
        file.write_all(data).map_err(io_error)
    }

    fn set_len(&self, key: CacheKey, size: u64) -> Result<(), LocalError> {
        self.open_for_writing(key)?.set_len(size).map_err(io_error)
    }

    fn open(&self, key: CacheKey) -> Result<Self::Reader, LocalError> {
        File::open(self.file_path(key)).map_err(io_error)
    }

    fn remove(&self, key: CacheKey) -> Result<(), LocalError> {
        match fs::remove_file(self.file_path(key)) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(io_error(error)),
            _ => Ok(()),
        }
    }
}
