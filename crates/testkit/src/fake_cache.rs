use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Mutex, MutexGuard};

use protonsync_application::ports::{CacheKey, ContentCache, LocalError};

/// In-memory cache of the virtual drive, for tests only.
#[derive(Debug, Default)]
pub struct InMemoryContentCache {
    files: Mutex<HashMap<CacheKey, Vec<u8>>>,
}

impl InMemoryContentCache {
    /// Empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of stored contents.
    #[must_use]
    pub fn len(&self) -> usize {
        self.files.lock().map_or(0, |files| files.len())
    }

    /// `true` if nothing is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn files(&self) -> Result<MutexGuard<'_, HashMap<CacheKey, Vec<u8>>>, LocalError> {
        self.files
            .lock()
            .map_err(|_| LocalError::Io("mutex poisoned".into()))
    }
}

fn index(value: u64) -> Result<usize, LocalError> {
    usize::try_from(value).map_err(|_| LocalError::Io("too large".into()))
}

impl ContentCache for InMemoryContentCache {
    type Reader = Cursor<Vec<u8>>;

    fn read(&self, key: CacheKey, offset: u64, length: usize) -> Result<Vec<u8>, LocalError> {
        let files = self.files()?;
        let Some(content) = files.get(&key) else {
            return Ok(Vec::new());
        };
        let start = index(offset)?.min(content.len());
        let end = start.saturating_add(length).min(content.len());
        Ok(content[start..end].to_vec())
    }

    fn write(&self, key: CacheKey, offset: u64, data: &[u8]) -> Result<(), LocalError> {
        let mut files = self.files()?;
        let content = files.entry(key).or_default();
        let start = index(offset)?;
        let end = start.saturating_add(data.len());
        if content.len() < end {
            content.resize(end, 0);
        }
        content[start..end].copy_from_slice(data);
        Ok(())
    }

    fn set_len(&self, key: CacheKey, size: u64) -> Result<(), LocalError> {
        let size = index(size)?;
        self.files()?.entry(key).or_default().resize(size, 0);
        Ok(())
    }

    fn open(&self, key: CacheKey) -> Result<Self::Reader, LocalError> {
        let files = self.files()?;
        let content = files
            .get(&key)
            .ok_or_else(|| LocalError::Io(format!("content {} missing", key.0)))?;
        Ok(Cursor::new(content.clone()))
    }

    fn remove(&self, key: CacheKey) -> Result<(), LocalError> {
        self.files()?.remove(&key);
        Ok(())
    }
}
