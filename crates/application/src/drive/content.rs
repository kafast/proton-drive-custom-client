//! Contents of the drive: reading through the block cache, writing,
//! creating and uploading changed files.

use protonsync_domain::cache::{BLOCK_BYTES, BlockMap, ByteRange};
use protonsync_domain::conflict::conflict_copy_name;
use protonsync_domain::node::{Node, NodeId, NodeName, RevisionId};
use protonsync_domain::path::RelativePath;

use super::tree::EntryKind;
use super::{Attributes, DriveError, Inner, Ino, ROOT_INO, VirtualDrive, revision_of};
use crate::ports::{
    CacheKey, CacheRecord, Clock, ContentCache, DriveRepo, RemoteError, RemoteStorage, StateRepo,
};

/// This much is read ahead beyond the request when reading (one block).
const READ_AHEAD_BYTES: u64 = BLOCK_BYTES;

/// What is needed to load missing blocks (usable without the lock).
struct FetchPlan {
    key: CacheKey,
    node: Option<NodeId>,
    revision: Option<RevisionId>,
    missing: Vec<ByteRange>,
}

/// State of a content before uploading.
struct UploadSnapshot {
    record: CacheRecord,
    generation: u64,
    ino: Option<Ino>,
    parent_node: Option<NodeId>,
    name: NodeName,
}

/// Result of an upload.
struct Uploaded {
    node: Node,
    /// On a conflict: the current state of the original in Proton.
    conflict_with: Option<Node>,
}

impl<R, C, D, S, K> VirtualDrive<R, C, D, S, K>
where
    R: RemoteStorage,
    C: ContentCache,
    D: DriveRepo,
    S: StateRepo,
    K: Clock,
{
    /// Plans under the lock which blocks are missing for `wanted`. Creates the
    /// cache entry if needed and discards outdated contents.
    fn plan_fetch(
        &self,
        inner: &mut Inner,
        ino: Ino,
        wanted: ByteRange,
    ) -> Result<FetchPlan, DriveError> {
        let inode = inner.tree.get(ino).ok_or(DriveError::NotFound)?;
        let EntryKind::File {
            size,
            modified,
            revision,
        } = inode.kind.clone()
        else {
            return Err(DriveError::IsAFolder);
        };
        let node = inode.node.clone();
        let key = if let Some(key) = inner.key_of(ino) {
            key
        } else {
            let key = inner.new_key();
            let record = CacheRecord {
                key,
                node: node.clone(),
                path: inner.tree.path(ino).ok_or(DriveError::NotFound)?,
                revision: revision.clone(),
                blocks: BlockMap::empty(size),
                last_access: self.clock.now(),
                modified,
                changed: false,
            };
            inner.insert_record(Some(ino), record);
            key
        };
        let record = inner
            .records
            .get_mut(&key)
            .ok_or(DriveError::Internal("entry missing"))?;
        if !record.changed && (record.revision != revision || record.blocks.size() != size) {
            // There is a new version in Proton: discard the old local copy.
            self.cache.remove(key)?;
            record.revision.clone_from(&revision);
            record.blocks = BlockMap::empty(size);
            record.modified = modified;
        }
        Ok(FetchPlan {
            key,
            node,
            revision: record.revision.clone(),
            missing: record.blocks.missing(wanted),
        })
    }

    /// Loads the missing blocks (without the lock) and records them.
    async fn fetch(&self, plan: FetchPlan) -> Result<CacheKey, DriveError> {
        if plan.missing.is_empty() {
            return Ok(plan.key);
        }
        let node = plan
            .node
            .ok_or(DriveError::Internal("new file without content"))?;
        for range in plan.missing {
            let data = self
                .remote
                .download_range(&node, range.offset, range.length)
                .await?;
            if u64::try_from(data.len()).ok() != Some(range.length) {
                return Err(RemoteError::Transfer(format!(
                    "received {} instead of {} bytes",
                    data.len(),
                    range.length
                ))
                .into());
            }
            self.cache.write(plan.key, range.offset, &data)?;
            let mut inner = self.lock()?;
            if let Some(record) = inner.records.get_mut(&plan.key)
                && record.revision == plan.revision
            {
                record.blocks.mark_present(range);
                self.save(record)?;
            }
        }
        Ok(plan.key)
    }

    /// Loads all missing blocks of a file.
    pub(super) async fn ensure_complete(&self, ino: Ino) -> Result<CacheKey, DriveError> {
        self.ensure_prefix(ino, u64::MAX).await
    }

    /// Loads the missing blocks of the first `length` bytes of a file.
    async fn ensure_prefix(&self, ino: Ino, length: u64) -> Result<CacheKey, DriveError> {
        let plan = {
            let mut inner = self.lock()?;
            self.plan_fetch(&mut inner, ino, ByteRange { offset: 0, length })?
        };
        self.fetch(plan).await
    }

    /// Reads up to `length` bytes from `offset`; missing blocks are loaded.
    ///
    /// # Errors
    /// [`DriveError::IsAFolder`], [`DriveError::NotFound`] or errors from Proton
    /// or the cache.
    pub async fn read(&self, ino: Ino, offset: u64, length: u32) -> Result<Vec<u8>, DriveError> {
        let plan = {
            let mut inner = self.lock()?;
            self.plan_fetch(
                &mut inner,
                ino,
                ByteRange {
                    offset,
                    length: u64::from(length).saturating_add(READ_AHEAD_BYTES),
                },
            )?
        };
        let key = self.fetch(plan).await?;
        if let Some(record) = self.lock()?.records.get_mut(&key) {
            record.last_access = self.clock.now();
        }
        let length = usize::try_from(length).map_err(|_| DriveError::Internal("length"))?;
        Ok(self.cache.read(key, offset, length)?)
    }

    /// Opens a file. For writing it is fully loaded first.
    ///
    /// # Errors
    /// Errors while loading.
    pub async fn open(&self, ino: Ino, write: bool) -> Result<(), DriveError> {
        if write {
            self.ensure_complete(ino).await?;
        }
        let mut inner = self.lock()?;
        if inner.tree.get(ino).is_none() {
            return Err(DriveError::NotFound);
        }
        *inner.open.entry(ino).or_insert(0) += 1;
        Ok(())
    }

    /// Writes to a file opened for writing.
    ///
    /// # Errors
    /// [`DriveError::NotFound`] or errors in the cache.
    pub fn write(&self, ino: Ino, offset: u64, data: &[u8]) -> Result<u32, DriveError> {
        let mut inner = self.lock()?;
        let key = inner.key_of(ino).ok_or(DriveError::NotFound)?;
        self.cache.write(key, offset, data)?;
        let length = u64::try_from(data.len()).map_err(|_| DriveError::Internal("length"))?;
        let end = offset.saturating_add(length);
        let now = self.clock.now();
        let record = inner
            .records
            .get_mut(&key)
            .ok_or(DriveError::Internal("entry missing"))?;
        if end > record.blocks.size() {
            record.blocks.resize(end);
        }
        let first_change = !record.changed;
        record.changed = true;
        record.modified = now;
        record.last_access = now;
        let size = record.blocks.size();
        if first_change {
            self.save(record)?;
        }
        *inner.generation.entry(key).or_insert(0) += 1;
        set_file_size(&mut inner, ino, size, now);
        u32::try_from(data.len()).map_err(|_| DriveError::Internal("length"))
    }

    /// Changes the size (truncating or extending).
    ///
    /// # Errors
    /// Errors while loading or in the cache.
    pub async fn set_size(&self, ino: Ino, size: u64) -> Result<Attributes, DriveError> {
        // Only the part that is kept has to be present. Truncating to 0
        // (e.g. when overwriting with `O_TRUNC`) therefore loads nothing.
        let key = self.ensure_prefix(ino, size).await?;
        let mut inner = self.lock()?;
        self.cache.set_len(key, size)?;
        let now = self.clock.now();
        if let Some(record) = inner.records.get_mut(&key) {
            record.blocks.resize(size);
            record.changed = true;
            record.modified = now;
            self.save(record)?;
        }
        *inner.generation.entry(key).or_insert(0) += 1;
        set_file_size(&mut inner, ino, size, now);
        inner.attributes(ino)
    }

    /// Creates a new, empty file and opens it. It is uploaded after
    /// closing.
    ///
    /// # Errors
    /// [`DriveError::AlreadyExists`] or errors while listing.
    pub async fn create(&self, parent: Ino, name: &NodeName) -> Result<Attributes, DriveError> {
        self.ensure_listed(parent).await?;
        let mut inner = self.lock()?;
        if inner.tree.child(parent, name).is_some() {
            return Err(DriveError::AlreadyExists);
        }
        let now = self.clock.now();
        let ino = inner.tree.add_local_file(parent, name.clone(), now);
        let key = inner.new_key();
        let record = CacheRecord {
            key,
            node: None,
            path: inner.tree.path(ino).ok_or(DriveError::NotFound)?,
            revision: None,
            blocks: BlockMap::complete(0),
            last_access: now,
            modified: now,
            changed: true,
        };
        self.cache.set_len(key, 0)?;
        self.save(&record)?;
        inner.insert_record(Some(ino), record);
        inner.open.insert(ino, 1);
        inner.attributes(ino)
    }

    /// Closes a handle. After the last close a changed file is uploaded;
    /// if that fails, it stays marked for [`Self::upload_pending`].
    ///
    /// # Errors
    /// Errors while uploading.
    pub async fn release(&self, ino: Ino) -> Result<(), DriveError> {
        let key = {
            let mut inner = self.lock()?;
            if let Some(count) = inner.open.get_mut(&ino) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    inner.open.remove(&ino);
                }
            }
            if inner.is_open(ino) {
                return Ok(());
            }
            let Some(key) = inner.key_of(ino) else {
                return Ok(());
            };
            if let Some(record) = inner.records.get(&key) {
                self.save(record)?;
            }
            key
        };
        self.upload(key).await
    }

    /// Uploads all changed files that are not open (e.g. after a
    /// restart or network error). Returns the paths where it failed.
    pub async fn upload_pending(&self) -> Vec<(RelativePath, DriveError)> {
        let keys: Vec<CacheKey> = match self.lock() {
            Ok(inner) => inner
                .records
                .values()
                .filter(|record| record.changed)
                .map(|record| record.key)
                .collect(),
            Err(error) => return vec![(RelativePath::root(), error)],
        };
        let mut failures = Vec::new();
        for key in keys {
            if let Err(error) = self.upload(key).await {
                let path = self
                    .lock()
                    .ok()
                    .and_then(|inner| inner.records.get(&key).map(|r| r.path.clone()))
                    .unwrap_or_default();
                failures.push((path, error));
            }
        }
        failures
    }

    /// Replaces the content of the file `target` with that of `source` (renaming
    /// over an existing file). The target keeps its node and gets a new
    /// revision; the source disappears.
    pub(super) async fn replace_file_content(
        &self,
        source: Ino,
        target: Ino,
    ) -> Result<(), DriveError> {
        let source_key = self.ensure_complete(source).await?;
        // A source that was already uploaded disappears under its old name.
        let source_node = self.lock()?.tree.get(source).and_then(|i| i.node.clone());
        if let Some(node) = &source_node {
            self.remote.trash(node).await?;
        }
        let target_key = {
            let mut inner = self.lock()?;
            let (target_node, target_revision) = {
                let inode = inner.tree.get(target).ok_or(DriveError::NotFound)?;
                let revision = match &inode.kind {
                    EntryKind::File { revision, .. } => revision.clone(),
                    EntryKind::Folder { .. } => return Err(DriveError::IsAFolder),
                };
                (inode.node.clone(), revision)
            };
            let target_path = inner.tree.path(target).ok_or(DriveError::NotFound)?;
            if let Some(old) = inner.key_of(target) {
                inner.take_record(old);
                self.cache.remove(old)?;
                self.repo.remove_cache_record(&self.settings.id, old)?;
            }
            let mut record = inner
                .take_record(source_key)
                .ok_or(DriveError::Internal("entry missing"))?;
            record.node = target_node;
            record.revision = target_revision;
            record.path = target_path;
            record.changed = true;
            record.modified = self.clock.now();
            let size = record.blocks.size();
            let modified = record.modified;
            self.save(&record)?;
            inner.insert_record(Some(target), record);
            *inner.generation.entry(source_key).or_insert(0) += 1;
            inner.tree.remove(source);
            set_file_size(&mut inner, target, size, modified);
            source_key
        };
        self.upload(target_key).await
    }

    /// Uploads a changed content if it is not open.
    async fn upload(&self, key: CacheKey) -> Result<(), DriveError> {
        let Some(snapshot) = self.upload_snapshot(key)? else {
            return Ok(());
        };
        let uploaded = self.upload_content(&snapshot).await?;
        self.finish_upload(&snapshot, &uploaded)
    }

    fn upload_snapshot(&self, key: CacheKey) -> Result<Option<UploadSnapshot>, DriveError> {
        let mut inner = self.lock()?;
        let Some(record) = inner.records.get(&key).cloned() else {
            return Ok(None);
        };
        let ino = inner
            .key_of_ino
            .iter()
            .find(|(_, other)| **other == key)
            .map(|(ino, _)| *ino)
            .or_else(|| {
                record
                    .node
                    .as_ref()
                    .and_then(|node| inner.tree.ino_of(node))
            });
        if !record.changed || ino.is_some_and(|ino| inner.is_open(ino)) {
            return Ok(None);
        }
        if let Some(ino) = ino {
            inner.key_of_ino.insert(ino, key);
        }
        let inode = ino.and_then(|ino| inner.tree.get(ino));
        let name = match inode {
            Some(inode) => inode.name.clone(),
            None => record.path.file_name().ok_or(DriveError::NotFound)?,
        };
        let parent_node = inode
            .map(|inode| inode.parent)
            .and_then(|parent| inner.tree.get(parent))
            .and_then(|parent| parent.node.clone());
        Ok(Some(UploadSnapshot {
            generation: inner.generation.get(&key).copied().unwrap_or(0),
            record,
            ino,
            parent_node,
            name,
        }))
    }

    async fn upload_content(&self, snapshot: &UploadSnapshot) -> Result<Uploaded, DriveError> {
        let record = &snapshot.record;
        if let Some(id) = &record.node {
            match self.remote.node(id).await {
                Ok(current) if revision_of(&current) == record.revision.as_ref() => {
                    let reader = self.cache.open(record.key)?;
                    let node = self
                        .remote
                        .upload_revision(id, reader, record.blocks.size(), record.modified)
                        .await?;
                    return Ok(Uploaded {
                        node,
                        conflict_with: None,
                    });
                }
                Ok(current) => {
                    // Proton has a different version by now: keep both.
                    let parent = current.parent.clone().ok_or(DriveError::NotFound)?;
                    let copy = self.copy_name(&snapshot.name)?;
                    let node = self.upload_new(&parent, &copy, record).await?;
                    return Ok(Uploaded {
                        node,
                        conflict_with: Some(current),
                    });
                }
                // Deleted in Proton: keep the local version as a new file.
                Err(RemoteError::NotFound(_)) => {}
                Err(error) => return Err(error.into()),
            }
        }
        let parent = match &snapshot.parent_node {
            Some(parent) => parent.clone(),
            None => {
                self.resolve_folder(&record.path.parent().unwrap_or_default())
                    .await?
            }
        };
        match self.upload_new(&parent, &snapshot.name, record).await {
            Err(DriveError::Remote(RemoteError::AlreadyExists(_))) => {
                let copy = self.copy_name(&snapshot.name)?;
                let node = self.upload_new(&parent, &copy, record).await?;
                Ok(Uploaded {
                    node,
                    conflict_with: None,
                })
            }
            other => other.map(|node| Uploaded {
                node,
                conflict_with: None,
            }),
        }
    }

    fn copy_name(&self, name: &NodeName) -> Result<NodeName, DriveError> {
        NodeName::new(conflict_copy_name(name.as_str(), self.clock.now()))
            .map_err(|e| DriveError::InvalidName(e.to_string()))
    }

    async fn upload_new(
        &self,
        parent: &NodeId,
        name: &NodeName,
        record: &CacheRecord,
    ) -> Result<Node, DriveError> {
        let reader = self.cache.open(record.key)?;
        Ok(self
            .remote
            .upload_file(parent, name, reader, record.blocks.size(), record.modified)
            .await?)
    }

    fn finish_upload(
        &self,
        snapshot: &UploadSnapshot,
        uploaded: &Uploaded,
    ) -> Result<(), DriveError> {
        let mut inner = self.lock()?;
        let key = snapshot.record.key;
        let unchanged_since =
            inner.generation.get(&key).copied().unwrap_or(0) == snapshot.generation;
        let Some(mut record) = inner.take_record(key) else {
            return Ok(());
        };
        record.node = Some(uploaded.node.id.clone());
        record.revision = revision_of(&uploaded.node).cloned();
        record.changed = !unchanged_since;
        let local_size = record.blocks.size();
        let parent_ino = uploaded
            .node
            .parent
            .as_ref()
            .and_then(|parent| inner.tree.ino_of(parent));
        let ino = match (&uploaded.conflict_with, snapshot.ino) {
            (Some(original), Some(original_ino)) => {
                // The original shows the Proton version again, the copy gets the content.
                if let Some(parent) = parent_ino {
                    inner.tree.upsert(parent, original);
                }
                inner.key_of_ino.remove(&original_ino);
                parent_ino.map(|parent| inner.tree.upsert(parent, &uploaded.node))
            }
            (None, Some(ino)) if inner.tree.get(ino).is_some() => {
                inner.tree.bind(ino, &uploaded.node);
                Some(ino)
            }
            _ => parent_ino
                .filter(|parent| inner.tree.is_listed(*parent))
                .map(|parent| inner.tree.upsert(parent, &uploaded.node)),
        };
        if let Some(ino) = ino {
            if let Some(path) = inner.tree.path(ino) {
                record.path = path;
            }
            if record.changed {
                let now = record.modified;
                set_file_size(&mut inner, ino, local_size, now);
            }
        } else if uploaded.conflict_with.is_some()
            && let Some(parent) = record.path.parent()
        {
            record.path = parent.join(&uploaded.node.name);
        }
        self.save(&record)?;
        inner.insert_record(ino, record);
        Ok(())
    }

    /// Folder for a path in the drive; missing folders are created
    /// (so that a local version is never lost).
    async fn resolve_folder(&self, path: &RelativePath) -> Result<NodeId, DriveError> {
        let mut current = ROOT_INO;
        if !path.is_root() {
            for segment in path.as_str().split('/') {
                let name =
                    NodeName::new(segment).map_err(|e| DriveError::InvalidName(e.to_string()))?;
                self.ensure_listed(current).await?;
                let existing = self.lock()?.tree.child(current, &name);
                current = match existing {
                    Some(ino) => ino,
                    None => self.make_folder(current, &name).await?.ino,
                };
            }
        }
        self.lock()?.node_of(current)
    }
}

/// Sets size and modification time of a file in the table.
fn set_file_size(
    inner: &mut Inner,
    ino: Ino,
    new_size: u64,
    now: protonsync_domain::time::Timestamp,
) {
    if let Some(inode) = inner.tree.get_mut(ino)
        && let EntryKind::File { size, modified, .. } = &mut inode.kind
    {
        *size = new_size;
        *modified = now;
    }
}
