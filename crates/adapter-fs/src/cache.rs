//! Cache of the virtual drive (adapter for the port [`ContentCache`]).
//!
//! Each content is its own file `<folder>/<key>`. The files are sparse:
//! blocks that have not been loaded yet take no space on disk.
//! Folders and files are accessible only to the owning user,
//! because they hold decrypted content.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use protonsync_application::ports::{CacheKey, ContentCache, LocalError};

/// Contents in the folder `dir` (e.g. `<data directory>/cache/<drive>`).
#[derive(Debug, Clone)]
pub struct FsContentCache {
    dir: PathBuf,
}

#[allow(clippy::needless_pass_by_value)] // Signature fits `map_err`.
fn io_error(error: io::Error) -> LocalError {
    LocalError::Io(error.to_string())
}

impl FsContentCache {
    /// Uses (and creates if needed) the folder `dir` with permissions 0700.
    ///
    /// # Errors
    /// [`LocalError::Io`] if the folder cannot be created.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, LocalError> {
        let dir = dir.into();
        fs::create_dir_all(&dir).map_err(io_error)?;
        restrict_folder(&dir)?;
        Ok(Self { dir })
    }

    /// The folder holding the contents.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn file_path(&self, key: CacheKey) -> PathBuf {
        self.dir.join(key.0.to_string())
    }

    /// Opens for writing; new files get permissions 0600.
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
        // `take` limits to `length`; `read_to_end` reads up to there or to the end of the file.
        let limit = u64::try_from(length).unwrap_or(u64::MAX);
        file.take(limit)
            .read_to_end(&mut content)
            .map_err(io_error)?;
        Ok(content)
    }

    fn write(&self, key: CacheKey, offset: u64, data: &[u8]) -> Result<(), LocalError> {
        let mut file = self.open_for_writing(key)?;
        // Writing past the end leaves a gap that reads as zeros.
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
