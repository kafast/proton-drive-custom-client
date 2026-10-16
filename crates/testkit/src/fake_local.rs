use std::collections::BTreeMap;
use std::io::{Cursor, Write};
use std::sync::{Mutex, MutexGuard};

use protonsync_application::ports::{LocalEntry, LocalError, LocalKind, LocalStorage};
use protonsync_domain::node::Fingerprint;
use protonsync_domain::path::RelativePath;
use protonsync_domain::time::Timestamp;

use crate::hash::sha1_hex;

/// In-memory stand-in for the local file system of a sync pair.
///
/// With [`FakeLocal::write`] and [`FakeLocal::remove`] tests simulate what a
/// user does locally.
#[derive(Debug, Default)]
pub struct FakeLocal {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    entries: BTreeMap<RelativePath, FakeEntry>,
    /// Trash: key → (path relative to the deleted entry, entry).
    trash: BTreeMap<String, Vec<(String, FakeEntry)>>,
    next_trash_key: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FakeEntry {
    Folder,
    File {
        content: Vec<u8>,
        modified: Timestamp,
    },
}

/// File that is being written (see [`LocalStorage::stage`]).
#[derive(Debug, Default)]
pub struct FakeStaging(Vec<u8>);

impl Write for FakeStaging {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl FakeLocal {
    /// Empty root folder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> Result<MutexGuard<'_, State>, LocalError> {
        self.state
            .lock()
            .map_err(|_| LocalError::Io("mutex poisoned".into()))
    }

    /// Simulates the user: creates a file with any missing folders, or overwrites it.
    pub fn write(&self, path: &str, content: &[u8], modified: Timestamp) {
        let path = parse(path);
        if let Ok(mut state) = self.state() {
            let mut parent = path.parent();
            while let Some(folder) = parent.filter(|p| !p.is_root()) {
                state
                    .entries
                    .entry(folder.clone())
                    .or_insert(FakeEntry::Folder);
                parent = folder.parent();
            }
            state.entries.insert(
                path,
                FakeEntry::File {
                    content: content.to_vec(),
                    modified,
                },
            );
        }
    }

    /// Simulates the user: creates a folder (with its parents).
    pub fn mkdir(&self, path: &str) {
        let path = parse(path);
        if let Ok(mut state) = self.state() {
            let mut current = Some(path);
            while let Some(folder) = current.filter(|p| !p.is_root()) {
                state
                    .entries
                    .entry(folder.clone())
                    .or_insert(FakeEntry::Folder);
                current = folder.parent();
            }
        }
    }

    /// Simulates the user: deletes a file or folder (with its content) without trash.
    pub fn remove(&self, path: &str) {
        let path = parse(path);
        if let Ok(mut state) = self.state() {
            state
                .entries
                .retain(|candidate, _| !candidate.starts_with(&path));
        }
    }

    /// Content of a file, if present (for checks in tests).
    #[must_use]
    pub fn content(&self, path: &str) -> Option<Vec<u8>> {
        let state = self.state().ok()?;
        match state.entries.get(&parse(path))? {
            FakeEntry::File { content, .. } => Some(content.clone()),
            FakeEntry::Folder => None,
        }
    }

    /// All paths (for checks in tests).
    #[must_use]
    pub fn paths(&self) -> Vec<String> {
        self.state()
            .map(|state| {
                state
                    .entries
                    .keys()
                    .map(|p| p.as_str().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Number of entries in the local trash.
    #[must_use]
    pub fn trash_len(&self) -> usize {
        self.state()
            .map(|state| state.trash.len())
            .unwrap_or_default()
    }
}

/// Test paths are literals; an invalid path is a bug in the test itself.
fn parse(path: &str) -> RelativePath {
    RelativePath::parse(path).unwrap_or_else(|error| unreachable!("invalid test path: {error}"))
}

fn to_entry(path: &RelativePath, entry: &FakeEntry) -> LocalEntry {
    let kind = match entry {
        FakeEntry::Folder => LocalKind::Folder,
        FakeEntry::File { content, modified } => LocalKind::File(Fingerprint {
            size: content.len() as u64,
            modified: *modified,
            content_hash: None,
        }),
    };
    LocalEntry {
        path: path.clone(),
        kind,
    }
}

impl State {
    fn require_parent_folder(&self, path: &RelativePath) -> Result<(), LocalError> {
        match path.parent() {
            None => Err(LocalError::NotFound(path.clone())),
            Some(parent) if parent.is_root() => Ok(()),
            Some(parent) => match self.entries.get(&parent) {
                Some(FakeEntry::Folder) => Ok(()),
                Some(FakeEntry::File { .. }) => Err(LocalError::NotAFolder(parent)),
                None => Err(LocalError::NotFound(parent)),
            },
        }
    }

    fn file(&self, path: &RelativePath) -> Result<&[u8], LocalError> {
        match self.entries.get(path) {
            Some(FakeEntry::File { content, .. }) => Ok(content),
            _ => Err(LocalError::NotFound(path.clone())),
        }
    }

    /// Takes out `path` with its subtree; the paths are stored relative to `path`.
    fn take_tree(&mut self, path: &RelativePath) -> Vec<(String, FakeEntry)> {
        let keys: Vec<RelativePath> = self
            .entries
            .keys()
            .filter(|candidate| candidate.starts_with(path))
            .cloned()
            .collect();
        keys.into_iter()
            .filter_map(|key| {
                let entry = self.entries.remove(&key)?;
                let suffix = key.as_str()[path.as_str().len()..].to_owned();
                Some((suffix, entry))
            })
            .collect()
    }
}

impl LocalStorage for FakeLocal {
    type Reader = Cursor<Vec<u8>>;
    type Staging = FakeStaging;

    fn scan(&self) -> Result<Vec<LocalEntry>, LocalError> {
        let state = self.state()?;
        Ok(state
            .entries
            .iter()
            .map(|(path, entry)| to_entry(path, entry))
            .collect())
    }

    fn entry(&self, path: &RelativePath) -> Result<Option<LocalEntry>, LocalError> {
        let state = self.state()?;
        Ok(state.entries.get(path).map(|entry| to_entry(path, entry)))
    }

    fn open(&self, path: &RelativePath) -> Result<Self::Reader, LocalError> {
        Ok(Cursor::new(self.state()?.file(path)?.to_vec()))
    }

    fn content_hash(&self, path: &RelativePath) -> Result<String, LocalError> {
        Ok(sha1_hex(self.state()?.file(path)?))
    }

    fn stage(&self, path: &RelativePath) -> Result<Self::Staging, LocalError> {
        self.state()?.require_parent_folder(path)?;
        Ok(FakeStaging::default())
    }

    fn commit(
        &self,
        staging: Self::Staging,
        path: &RelativePath,
        modified: Timestamp,
    ) -> Result<Fingerprint, LocalError> {
        let mut state = self.state()?;
        state.require_parent_folder(path)?;
        if matches!(state.entries.get(path), Some(FakeEntry::Folder)) {
            return Err(LocalError::AlreadyExists(path.clone()));
        }
        let size = staging.0.len() as u64;
        state.entries.insert(
            path.clone(),
            FakeEntry::File {
                content: staging.0,
                modified,
            },
        );
        Ok(Fingerprint {
            size,
            modified,
            content_hash: None,
        })
    }

    fn create_folder(&self, path: &RelativePath) -> Result<(), LocalError> {
        let mut state = self.state()?;
        state.require_parent_folder(path)?;
        match state.entries.get(path) {
            Some(FakeEntry::Folder) => Ok(()),
            Some(FakeEntry::File { .. }) => Err(LocalError::NotAFolder(path.clone())),
            None => {
                state.entries.insert(path.clone(), FakeEntry::Folder);
                Ok(())
            }
        }
    }

    fn rename(&self, from: &RelativePath, to: &RelativePath) -> Result<(), LocalError> {
        let mut state = self.state()?;
        if !state.entries.contains_key(from) {
            return Err(LocalError::NotFound(from.clone()));
        }
        if state.entries.contains_key(to) {
            return Err(LocalError::AlreadyExists(to.clone()));
        }
        state.require_parent_folder(to)?;
        for (suffix, entry) in state.take_tree(from) {
            let target = parse(&format!("{}{suffix}", to.as_str()));
            state.entries.insert(target, entry);
        }
        Ok(())
    }

    fn move_to_trash(&self, path: &RelativePath) -> Result<String, LocalError> {
        let mut state = self.state()?;
        if !state.entries.contains_key(path) {
            return Err(LocalError::NotFound(path.clone()));
        }
        let key = state.next_trash_key.to_string();
        state.next_trash_key += 1;
        let tree = state.take_tree(path);
        state.trash.insert(key.clone(), tree);
        Ok(key)
    }

    fn restore_from_trash(&self, stored_as: &str, path: &RelativePath) -> Result<(), LocalError> {
        let mut state = self.state()?;
        if state.entries.contains_key(path) {
            return Err(LocalError::AlreadyExists(path.clone()));
        }
        state.require_parent_folder(path)?;
        let tree = state
            .trash
            .remove(stored_as)
            .ok_or_else(|| LocalError::NotFound(path.clone()))?;
        for (suffix, entry) in tree {
            state
                .entries
                .insert(parse(&format!("{}{suffix}", path.as_str())), entry);
        }
        Ok(())
    }

    fn purge_trash(&self, stored_as: &str) -> Result<(), LocalError> {
        let mut state = self.state()?;
        state
            .trash
            .remove(stored_as)
            .map(|_| ())
            .ok_or_else(|| LocalError::NotFound(RelativePath::root()))
    }
}
