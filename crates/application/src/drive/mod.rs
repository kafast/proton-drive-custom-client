//! The virtual drive (on demand): Proton folders as a file system,
//! contents are only loaded when read and cached locally.
//!
//! [`VirtualDrive`] does not know FUSE. Its methods correspond to the
//! file system calls (lookup, listing, reading, writing, …); the
//! `fuse` crate only translates. This way everything can be tested against fakes.
//!
//! Locking: the state sits behind a `std::sync::Mutex` that is never held
//! across an `.await`. Network access runs without the lock;
//! afterwards it is checked whether the state has changed in the meantime.
//! (The compiler helps here: a held lock is not `Send`, so a
//! future holding a lock across an `.await` would not compile.)
//!
//! Uploads and renames of the same file additionally pass an async gate
//! (`upload_gate`), so they never overlap.
//!
//! See ADR 0010.

mod content;
mod maintenance;
mod status;
mod tree;
mod upload_gate;

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use protonsync_domain::node::{Node, NodeId, NodeKind, NodeName, RevisionId};
use protonsync_domain::path::RelativePath;
use protonsync_domain::pin::{PinRules, PinState};
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::time::Timestamp;
use protonsync_domain::trash::{TrashEntry, TrashOrigin};
use thiserror::Error;

use crate::ports::{
    CacheKey, CacheRecord, Clock, ContentCache, DriveRepo, EventCursor, LocalError, RemoteError,
    RemoteStorage, StateError, StateRepo,
};

pub use maintenance::{CacheUsage, EvictionReport, PrefetchReport, RefreshReport};
pub use status::{DriveContent, DrivePathStatus};
use tree::{EntryKind, Tree};
pub use tree::{Ino, ROOT_INO};
use upload_gate::UploadGates;

/// Errors of the virtual drive. The FUSE crate translates them into `errno` values.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DriveError {
    /// The entry does not exist.
    #[error("not found")]
    NotFound,
    /// The name is already taken.
    #[error("already exists")]
    AlreadyExists,
    /// A folder was expected.
    #[error("not a folder")]
    NotAFolder,
    /// A file was expected.
    #[error("is a folder")]
    IsAFolder,
    /// The folder is not empty.
    #[error("folder is not empty")]
    NotEmpty,
    /// Invalid name.
    #[error("invalid name: {0}")]
    InvalidName(String),
    /// Error from Proton.
    #[error(transparent)]
    Remote(#[from] RemoteError),
    /// Error in the local cache.
    #[error(transparent)]
    Local(#[from] LocalError),
    /// Error in the state store.
    #[error(transparent)]
    State(#[from] StateError),
    /// Internal error (e.g. a poisoned lock after a thread crashed).
    #[error("internal error: {0}")]
    Internal(&'static str),
}

/// Settings of a drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveSettings {
    /// Identifier in the state, e.g. `mount:home` (also the owner in the trash).
    pub id: SyncPairId,
    /// Applies where no pin rule matches („Immer lokal halten“ as the default).
    pub default_pin: PinState,
}

/// Details of an entry as the file system shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attributes {
    /// Number.
    pub ino: Ino,
    /// Folder or file.
    pub is_folder: bool,
    /// Size in bytes (folders: 0).
    pub size: u64,
    /// Modification time.
    pub modified: Timestamp,
}

/// An entry when listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// Number.
    pub ino: Ino,
    /// Name.
    pub name: NodeName,
    /// Folder or file.
    pub is_folder: bool,
}

/// Mutable state behind the lock.
struct Inner {
    tree: Tree,
    records: HashMap<CacheKey, CacheRecord>,
    key_of_node: HashMap<NodeId, CacheKey>,
    key_of_ino: HashMap<Ino, CacheKey>,
    /// Number of open handles per entry.
    open: HashMap<Ino, u32>,
    /// Counts writes per content; an upload only counts if the counter
    /// did not change in the meantime.
    generation: HashMap<CacheKey, u64>,
    next_key: u64,
    pins: PinRules,
    cursor: Option<EventCursor>,
}

impl Inner {
    /// Key of the content for an entry, if there is one.
    fn key_of(&mut self, ino: Ino) -> Option<CacheKey> {
        if let Some(key) = self.key_of_ino.get(&ino) {
            return Some(*key);
        }
        let node = self.tree.get(ino)?.node.clone()?;
        let key = *self.key_of_node.get(&node)?;
        self.key_of_ino.insert(ino, key);
        Some(key)
    }

    fn is_open(&self, ino: Ino) -> bool {
        self.open.get(&ino).is_some_and(|count| *count > 0)
    }

    fn new_key(&mut self) -> CacheKey {
        let key = CacheKey(self.next_key);
        self.next_key += 1;
        key
    }

    fn insert_record(&mut self, ino: Option<Ino>, record: CacheRecord) {
        if let Some(node) = &record.node {
            self.key_of_node.insert(node.clone(), record.key);
        }
        if let Some(ino) = ino {
            self.key_of_ino.insert(ino, record.key);
        }
        self.records.insert(record.key, record);
    }

    fn take_record(&mut self, key: CacheKey) -> Option<CacheRecord> {
        let record = self.records.remove(&key)?;
        if let Some(node) = &record.node {
            self.key_of_node.remove(node);
        }
        self.key_of_ino.retain(|_, other| *other != key);
        self.generation.remove(&key);
        Some(record)
    }

    fn attributes(&self, ino: Ino) -> Result<Attributes, DriveError> {
        let inode = self.tree.get(ino).ok_or(DriveError::NotFound)?;
        Ok(match &inode.kind {
            EntryKind::Folder { .. } => Attributes {
                ino,
                is_folder: true,
                size: 0,
                modified: Timestamp::from_unix_seconds(0),
            },
            EntryKind::File { size, modified, .. } => Attributes {
                ino,
                is_folder: false,
                size: *size,
                modified: *modified,
            },
        })
    }

    fn node_of(&self, ino: Ino) -> Result<NodeId, DriveError> {
        self.tree
            .get(ino)
            .ok_or(DriveError::NotFound)?
            .node
            .clone()
            .ok_or(DriveError::NotFound)
    }
}

/// Revision of a file, `None` for folders.
fn revision_of(node: &Node) -> Option<&RevisionId> {
    match &node.kind {
        NodeKind::File { revision, .. } => Some(revision),
        NodeKind::Folder => None,
    }
}

/// Replaces the prefix `from` of a path with `to` (for renaming a folder).
fn rebase(path: &RelativePath, from: &RelativePath, to: &RelativePath) -> Option<RelativePath> {
    if path == from {
        return Some(to.clone());
    }
    if !path.starts_with(from) {
        return None;
    }
    let rest = if from.is_root() {
        path.as_str()
    } else {
        path.as_str().get(from.as_str().len() + 1..)?
    };
    if to.is_root() {
        RelativePath::parse(rest).ok()
    } else {
        RelativePath::parse(&format!("{}/{rest}", to.as_str())).ok()
    }
}

/// The virtual drive on top of a folder in Proton.
pub struct VirtualDrive<R, C, D, S, K> {
    remote: R,
    cache: C,
    repo: D,
    state: S,
    clock: K,
    settings: DriveSettings,
    inner: Mutex<Inner>,
    /// Keeps uploads and renames of the same file apart (see `upload_gate`).
    gates: UploadGates,
}

impl<R, C, D, S, K> VirtualDrive<R, C, D, S, K>
where
    R: RemoteStorage,
    C: ContentCache,
    D: DriveRepo,
    S: StateRepo,
    K: Clock,
{
    /// Builds the drive on top of the folder `root` and loads the stored
    /// cache index including the pin rules.
    ///
    /// # Errors
    /// [`DriveError::State`] if the index cannot be read.
    pub fn new(
        remote: R,
        cache: C,
        repo: D,
        state: S,
        clock: K,
        settings: DriveSettings,
        root: &Node,
    ) -> Result<Self, DriveError> {
        let records = repo.cache_records(&settings.id)?;
        let pins = repo.pin_rules(&settings.id)?;
        let next_key = records.iter().map(|r| r.key.0 + 1).max().unwrap_or(1);
        let mut inner = Inner {
            tree: Tree::new(root),
            records: HashMap::new(),
            key_of_node: HashMap::new(),
            key_of_ino: HashMap::new(),
            open: HashMap::new(),
            generation: HashMap::new(),
            next_key,
            pins,
            cursor: None,
        };
        for record in records {
            inner.insert_record(None, record);
        }
        Ok(Self {
            remote,
            cache,
            repo,
            state,
            clock,
            settings,
            inner: Mutex::new(inner),
            gates: UploadGates::default(),
        })
    }

    /// Settings of the drive.
    #[must_use]
    pub fn settings(&self) -> &DriveSettings {
        &self.settings
    }

    fn lock(&self) -> Result<MutexGuard<'_, Inner>, DriveError> {
        self.inner
            .lock()
            .map_err(|_| DriveError::Internal("lock poisoned"))
    }

    fn save(&self, record: &CacheRecord) -> Result<(), DriveError> {
        Ok(self.repo.save_cache_record(&self.settings.id, record)?)
    }

    /// Reads the folder from Proton if not done yet.
    async fn ensure_listed(&self, folder: Ino) -> Result<(), DriveError> {
        let node = {
            let inner = self.lock()?;
            let inode = inner.tree.get(folder).ok_or(DriveError::NotFound)?;
            if !inode.is_folder() {
                return Err(DriveError::NotAFolder);
            }
            if inner.tree.is_listed(folder) {
                return Ok(());
            }
            inode.node.clone().ok_or(DriveError::NotFound)?
        };
        let children = self.remote.list_children(&node).await?;
        let mut inner = self.lock()?;
        if inner.tree.get(folder).is_some() && !inner.tree.is_listed(folder) {
            inner.tree.set_children(folder, &children);
        }
        Ok(())
    }

    /// Details of an entry.
    ///
    /// # Errors
    /// [`DriveError::NotFound`].
    pub fn attributes(&self, ino: Ino) -> Result<Attributes, DriveError> {
        self.lock()?.attributes(ino)
    }

    /// Looks up `name` in `parent`.
    ///
    /// # Errors
    /// [`DriveError::NotFound`], [`DriveError::NotAFolder`] or errors from Proton.
    pub async fn lookup(&self, parent: Ino, name: &NodeName) -> Result<Attributes, DriveError> {
        self.ensure_listed(parent).await?;
        let inner = self.lock()?;
        let ino = inner.tree.child(parent, name).ok_or(DriveError::NotFound)?;
        inner.attributes(ino)
    }

    /// Contents of a folder, sorted by name.
    ///
    /// # Errors
    /// [`DriveError::NotFound`], [`DriveError::NotAFolder`] or errors from Proton.
    pub async fn list(&self, folder: Ino) -> Result<Vec<DirEntry>, DriveError> {
        self.ensure_listed(folder).await?;
        let inner = self.lock()?;
        let children = inner.tree.children(folder).ok_or(DriveError::NotFound)?;
        Ok(children
            .into_iter()
            .filter_map(|(name, ino)| {
                let inode = inner.tree.get(ino)?;
                Some(DirEntry {
                    ino,
                    name,
                    is_folder: inode.is_folder(),
                })
            })
            .collect())
    }

    /// Creates a folder (immediately in Proton).
    ///
    /// # Errors
    /// [`DriveError::AlreadyExists`] or errors from Proton.
    pub async fn make_folder(
        &self,
        parent: Ino,
        name: &NodeName,
    ) -> Result<Attributes, DriveError> {
        self.ensure_listed(parent).await?;
        let parent_node = {
            let inner = self.lock()?;
            if inner.tree.child(parent, name).is_some() {
                return Err(DriveError::AlreadyExists);
            }
            inner.node_of(parent)?
        };
        let folder = self.remote.create_folder(&parent_node, name).await?;
        let mut inner = self.lock()?;
        let ino = inner.tree.upsert(parent, &folder);
        // A new folder is empty; asking Proton is unnecessary.
        inner.tree.set_children(ino, &[]);
        inner.attributes(ino)
    }

    /// Moves a file to the trash (Proton and protonsync).
    ///
    /// # Errors
    /// [`DriveError::NotFound`], [`DriveError::IsAFolder`] or errors from Proton.
    pub async fn remove_file(&self, parent: Ino, name: &NodeName) -> Result<(), DriveError> {
        self.ensure_listed(parent).await?;
        let ino = {
            let inner = self.lock()?;
            let ino = inner.tree.child(parent, name).ok_or(DriveError::NotFound)?;
            if inner.tree.get(ino).is_some_and(tree::Inode::is_folder) {
                return Err(DriveError::IsAFolder);
            }
            ino
        };
        self.trash_entry(ino).await
    }

    /// Moves an empty folder to the trash.
    ///
    /// # Errors
    /// [`DriveError::NotEmpty`], [`DriveError::NotAFolder`], [`DriveError::NotFound`]
    /// or errors from Proton.
    pub async fn remove_folder(&self, parent: Ino, name: &NodeName) -> Result<(), DriveError> {
        self.ensure_listed(parent).await?;
        let ino = {
            let inner = self.lock()?;
            inner.tree.child(parent, name).ok_or(DriveError::NotFound)?
        };
        self.ensure_listed(ino).await?;
        {
            let inner = self.lock()?;
            if !inner.tree.children(ino).unwrap_or_default().is_empty() {
                return Err(DriveError::NotEmpty);
            }
        }
        self.trash_entry(ino).await
    }

    /// Trash for an entry: in Proton and as an entry in protonsync.
    /// A file that was not uploaded yet is only discarded locally.
    async fn trash_entry(&self, ino: Ino) -> Result<(), DriveError> {
        let (node, path) = {
            let inner = self.lock()?;
            let inode = inner.tree.get(ino).ok_or(DriveError::NotFound)?;
            (
                inode.node.clone(),
                inner.tree.path(ino).ok_or(DriveError::NotFound)?,
            )
        };
        if let Some(node) = node {
            self.remote.trash(&node).await?;
            self.state.add_trash(
                &self.settings.id,
                &TrashEntry {
                    original_path: path.clone(),
                    origin: TrashOrigin::Remote { node },
                    trashed_at: self.clock.now(),
                },
            )?;
        }
        self.forget_below(ino, &path)
    }

    /// Removes an entry including the cached contents below it.
    fn forget_below(&self, ino: Ino, path: &RelativePath) -> Result<(), DriveError> {
        let mut inner = self.lock()?;
        inner.tree.remove(ino);
        let keys: Vec<CacheKey> = inner
            .records
            .values()
            .filter(|record| record.path.starts_with(path))
            .map(|record| record.key)
            .collect();
        for key in keys {
            inner.take_record(key);
            self.cache.remove(key)?;
            self.repo.remove_cache_record(&self.settings.id, key)?;
        }
        Ok(())
    }

    /// Renames or moves. An existing target is replaced as is usual on
    /// Linux: a file gets the new content as a new revision
    /// (this is how many programs save: first into a helper file, then rename),
    /// an empty folder goes to the trash.
    ///
    /// # Errors
    /// [`DriveError::NotFound`], [`DriveError::NotEmpty`], [`DriveError::IsAFolder`],
    /// [`DriveError::NotAFolder`] or errors from Proton.
    pub async fn rename(
        &self,
        parent: Ino,
        name: &NodeName,
        new_parent: Ino,
        new_name: &NodeName,
    ) -> Result<(), DriveError> {
        self.ensure_listed(parent).await?;
        self.ensure_listed(new_parent).await?;
        let (source, target) = {
            let inner = self.lock()?;
            let source = inner.tree.child(parent, name).ok_or(DriveError::NotFound)?;
            (source, inner.tree.child(new_parent, new_name))
        };
        if target == Some(source) {
            return Ok(());
        }
        // An upload of the source or target that is still running (the file
        // was just closed) has to finish first, otherwise it would use the
        // old name or a revision that is about to change.
        let _gates = self.gates.enter_entries(source, target).await?;
        if let Some(target) = target
            && self.replace_target(source, target).await?
        {
            return Ok(());
        }
        self.move_entry(source, new_parent, new_name).await
    }

    /// Clears an existing target. `true` if the source has already been merged
    /// into the target in the process (file replaces file).
    async fn replace_target(&self, source: Ino, target: Ino) -> Result<bool, DriveError> {
        let (source_is_folder, target_is_folder, target_is_new) = {
            let inner = self.lock()?;
            let source = inner.tree.get(source).ok_or(DriveError::NotFound)?;
            let target = inner.tree.get(target).ok_or(DriveError::NotFound)?;
            (
                source.is_folder(),
                target.is_folder(),
                target.node.is_none(),
            )
        };
        match (source_is_folder, target_is_folder) {
            (false, true) => Err(DriveError::IsAFolder),
            (true, false) => Err(DriveError::NotAFolder),
            (true, true) => {
                self.ensure_listed(target).await?;
                let empty = self
                    .lock()?
                    .tree
                    .children(target)
                    .unwrap_or_default()
                    .is_empty();
                if !empty {
                    return Err(DriveError::NotEmpty);
                }
                self.trash_entry(target).await?;
                Ok(false)
            }
            (false, false) if target_is_new => {
                // The target itself was not uploaded yet: simply discard it.
                self.trash_entry(target).await?;
                Ok(false)
            }
            (false, false) => {
                self.replace_file_content(source, target).await?;
                Ok(true)
            }
        }
    }

    /// Moves or renames an entry when there is no target at the new location.
    async fn move_entry(
        &self,
        source: Ino,
        new_parent: Ino,
        new_name: &NodeName,
    ) -> Result<(), DriveError> {
        let (node, old_parent, old_name, old_path, new_parent_node) = {
            let inner = self.lock()?;
            let inode = inner.tree.get(source).ok_or(DriveError::NotFound)?;
            (
                inode.node.clone(),
                inode.parent,
                inode.name.clone(),
                inner.tree.path(source).ok_or(DriveError::NotFound)?,
                inner.node_of(new_parent)?,
            )
        };
        let mut updated = None;
        if let Some(node) = &node {
            if old_parent != new_parent {
                updated = Some(self.remote.move_node(node, &new_parent_node).await?);
            }
            if &old_name != new_name {
                updated = Some(self.remote.rename(node, new_name).await?);
            }
        }
        let mut inner = self.lock()?;
        match updated {
            Some(node) => {
                inner.tree.upsert(new_parent, &node);
            }
            None => inner.tree.rename(source, new_parent, new_name.clone()),
        }
        let new_path = inner.tree.path(source).ok_or(DriveError::NotFound)?;
        self.rebase_records(&mut inner, &old_path, &new_path)?;
        self.rebase_pins(&mut inner, &old_path, &new_path)
    }

    fn rebase_records(
        &self,
        inner: &mut Inner,
        from: &RelativePath,
        to: &RelativePath,
    ) -> Result<(), DriveError> {
        for record in inner.records.values_mut() {
            if let Some(path) = rebase(&record.path, from, to) {
                record.path = path;
                self.repo.save_cache_record(&self.settings.id, record)?;
            }
        }
        Ok(())
    }

    fn rebase_pins(
        &self,
        inner: &mut Inner,
        from: &RelativePath,
        to: &RelativePath,
    ) -> Result<(), DriveError> {
        let moved: Vec<(RelativePath, PinState)> = inner
            .pins
            .iter()
            .filter(|(path, _)| path.starts_with(from))
            .map(|(path, state)| (path.clone(), state))
            .collect();
        for (path, state) in moved {
            if let Some(new_path) = rebase(&path, from, to) {
                inner.pins.clear(&path);
                self.repo.save_pin_rule(&self.settings.id, &path, None)?;
                inner.pins.set(new_path.clone(), state);
                self.repo
                    .save_pin_rule(&self.settings.id, &new_path, Some(state))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn path(text: &str) -> RelativePath {
        RelativePath::parse(text).unwrap()
    }

    #[test]
    fn rebase_replaces_the_prefix_only() {
        assert_eq!(
            rebase(&path("a/b"), &path("a"), &path("x")),
            Some(path("x/b"))
        );
        assert_eq!(
            rebase(&path("a"), &path("a"), &path("x/y")),
            Some(path("x/y"))
        );
        assert_eq!(rebase(&path("ab"), &path("a"), &path("x")), None);
        assert_eq!(
            rebase(&path("a/b"), &path("a"), &RelativePath::root()),
            Some(path("b"))
        );
        assert_eq!(
            rebase(&path("b"), &RelativePath::root(), &path("x")),
            Some(path("x/b"))
        );
    }
}
