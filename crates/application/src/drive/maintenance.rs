//! Drive maintenance: applying Proton events, pin rules,
//! prefetching pinned files and freeing space.

use std::collections::VecDeque;

use protonsync_domain::node::{Node, NodeId, NodeName};
use protonsync_domain::path::RelativePath;
use protonsync_domain::pin::{CachedFile, EvictionPolicy, PinState};

use super::tree::EntryKind;
use super::{DriveError, Inner, Ino, ROOT_INO, VirtualDrive};
use crate::ports::{
    CacheKey, Clock, ContentCache, DriveRepo, RemoteError, RemoteEvent, RemoteStorage, StateRepo,
};

/// From this many events on, the folders are rather read again instead of asking
/// Proton for every node individually.
const MAX_EVENTS_PER_REFRESH: usize = 200;

/// Result of [`VirtualDrive::refresh`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RefreshReport {
    /// Processed events.
    pub events: usize,
    /// `true` if all read folders were discarded (gap or too many events).
    pub listings_forgotten: bool,
}

/// Result of [`VirtualDrive::prefetch_pinned`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PrefetchReport {
    /// Fully loaded files (previously incomplete).
    pub files: usize,
    /// Bytes loaded for this (roughly: size of the files).
    pub bytes: u64,
}

/// Result of [`VirtualDrive::evict`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EvictionReport {
    /// Freed files (or, in a dry run: files to be freed).
    pub files: Vec<RelativePath>,
    /// Bytes freed by this.
    pub bytes: u64,
    /// Skipped: Proton did not confirm the file (any more), or it is
    /// (for [`VirtualDrive::free`]) pinned, changed or open.
    pub skipped: usize,
}

/// An entry scheduled for freeing.
struct PlannedEviction {
    key: CacheKey,
    node: Option<NodeId>,
    path: RelativePath,
}

/// Cache usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheUsage {
    /// Files with locally present content.
    pub files: usize,
    /// Used bytes.
    pub bytes: u64,
    /// Files with local changes that are still to be uploaded.
    pub pending_uploads: usize,
}

impl<R, C, D, S, K> VirtualDrive<R, C, D, S, K>
where
    R: RemoteStorage,
    C: ContentCache,
    D: DriveRepo,
    S: StateRepo,
    K: Clock,
{
    /// Applies the changes in Proton since the last call. The first
    /// call only remembers the starting point.
    ///
    /// # Errors
    /// Errors from Proton; the starting point then stays unchanged.
    pub async fn refresh(&self) -> Result<RefreshReport, DriveError> {
        let cursor = self.lock()?.cursor.clone();
        let (events, next) = self.remote.events_since(cursor.as_ref()).await?;
        let Some(_) = cursor else {
            self.lock()?.cursor = Some(next);
            return Ok(RefreshReport::default());
        };
        let forget = events.len() > MAX_EVENTS_PER_REFRESH
            || events.iter().any(|event| matches!(event, RemoteEvent::Gap));
        if forget {
            self.lock()?.tree.forget_listings();
        } else {
            for event in &events {
                match event {
                    RemoteEvent::Created(id) | RemoteEvent::Updated(id) => {
                        match self.remote.node(id).await {
                            Ok(node) => self.apply_node(&node)?,
                            Err(RemoteError::NotFound(_)) => self.apply_trashed(id)?,
                            Err(error) => return Err(error.into()),
                        }
                    }
                    RemoteEvent::Trashed(id) => self.apply_trashed(id)?,
                    RemoteEvent::Gap => {}
                }
            }
        }
        self.lock()?.cursor = Some(next);
        Ok(RefreshReport {
            events: events.len(),
            listings_forgotten: forget,
        })
    }

    /// A changed or new node: only apply it if it or its parent folder
    /// is known (otherwise it lies outside what has been read).
    fn apply_node(&self, node: &Node) -> Result<(), DriveError> {
        let mut inner = self.lock()?;
        let parent = node.parent.as_ref().and_then(|p| inner.tree.ino_of(p));
        let known = inner.tree.ino_of(&node.id);
        match (known, parent) {
            (_, Some(parent)) if inner.tree.is_listed(parent) => {
                inner.tree.upsert(parent, node);
            }
            (Some(ino), _) if ino != ROOT_INO => {
                // Moved away from the read area.
                inner.tree.remove(ino);
            }
            _ => {}
        }
        Ok(())
    }

    /// A node is now in the Proton trash. Local changes below it
    /// are kept and uploaded later as a new file.
    fn apply_trashed(&self, id: &NodeId) -> Result<(), DriveError> {
        let mut inner = self.lock()?;
        let Some(ino) = inner.tree.ino_of(id) else {
            return Ok(());
        };
        let Some(path) = inner.tree.path(ino) else {
            return Ok(());
        };
        inner.tree.remove(ino);
        let below: Vec<CacheKey> = inner
            .records
            .values()
            .filter(|record| record.path.starts_with(&path))
            .map(|record| record.key)
            .collect();
        for key in below {
            let Some(mut record) = inner.take_record(key) else {
                continue;
            };
            if record.changed {
                record.node = None;
                record.revision = None;
                self.save(&record)?;
                inner.insert_record(None, record);
            } else {
                self.cache.remove(key)?;
                self.repo.remove_cache_record(&self.settings.id, key)?;
            }
        }
        Ok(())
    }

    /// Sets „Immer lokal halten“ (`Pinned`), „Nur bei Bedarf“ (`Unpinned`)
    /// or removes the rule (`None`) for `path`. Rules below it are dropped,
    /// so that the new rule applies to the whole folder.
    ///
    /// # Errors
    /// [`DriveError::State`].
    pub fn set_pin(&self, path: &RelativePath, state: Option<PinState>) -> Result<(), DriveError> {
        let mut inner = self.lock()?;
        let below: Vec<RelativePath> = inner
            .pins
            .iter()
            .filter(|(rule, _)| rule.starts_with(path))
            .map(|(rule, _)| rule.clone())
            .collect();
        for rule in below {
            self.repo.save_pin_rule(&self.settings.id, &rule, None)?;
        }
        inner.pins.clear_below(path);
        if let Some(state) = state {
            self.repo
                .save_pin_rule(&self.settings.id, path, Some(state))?;
            inner.pins.set(path.clone(), state);
        }
        Ok(())
    }

    /// What applies to `path`.
    ///
    /// # Errors
    /// [`DriveError::Internal`] on a poisoned lock.
    pub fn pin_state(&self, path: &RelativePath) -> Result<PinState, DriveError> {
        Ok(self.lock()?.pins.effective(path, self.settings.default_pin))
    }

    /// The explicitly set rules.
    ///
    /// # Errors
    /// [`DriveError::Internal`] on a poisoned lock.
    pub fn pin_rules(&self) -> Result<Vec<(RelativePath, PinState)>, DriveError> {
        Ok(self
            .lock()?
            .pins
            .iter()
            .map(|(path, state)| (path.clone(), state))
            .collect())
    }

    /// Entry for a path; folders on the way are read if needed.
    ///
    /// # Errors
    /// [`DriveError::NotFound`] or errors from Proton.
    pub async fn resolve(&self, path: &RelativePath) -> Result<Ino, DriveError> {
        let mut current = ROOT_INO;
        if path.is_root() {
            return Ok(current);
        }
        for segment in path.as_str().split('/') {
            let name =
                NodeName::new(segment).map_err(|e| DriveError::InvalidName(e.to_string()))?;
            self.ensure_listed(current).await?;
            current = self
                .lock()?
                .tree
                .child(current, &name)
                .ok_or(DriveError::NotFound)?;
        }
        Ok(current)
    }

    /// Fully loads all files for which „Immer lokal halten“ applies.
    /// For this, the pinned folders are read completely once.
    ///
    /// # Errors
    /// Errors from Proton or in the cache (what is already loaded stays).
    pub async fn prefetch_pinned(&self) -> Result<PrefetchReport, DriveError> {
        let mut roots: Vec<RelativePath> = self
            .pin_rules()?
            .into_iter()
            .filter(|(_, state)| *state == PinState::Pinned)
            .map(|(path, _)| path)
            .collect();
        if self.settings.default_pin == PinState::Pinned {
            roots = vec![RelativePath::root()];
        }
        let mut report = PrefetchReport::default();
        let mut queue: VecDeque<Ino> = VecDeque::new();
        for root in &roots {
            match self.resolve(root).await {
                Ok(ino) => queue.push_back(ino),
                // A rule for a path deleted in the meantime: nothing to do.
                Err(DriveError::NotFound) => {}
                Err(error) => return Err(error),
            }
        }
        while let Some(ino) = queue.pop_front() {
            let (is_folder, path) = {
                let inner = self.lock()?;
                let Some(inode) = inner.tree.get(ino) else {
                    continue;
                };
                (inode.is_folder(), inner.tree.path(ino).unwrap_or_default())
            };
            if is_folder {
                for entry in self.list(ino).await? {
                    queue.push_back(entry.ino);
                }
            } else if self.pin_state(&path)? == PinState::Pinned {
                let size = self.missing_bytes(ino)?;
                if size > 0 {
                    self.ensure_complete(ino).await?;
                    report.files += 1;
                    report.bytes += size;
                }
            }
        }
        Ok(report)
    }

    /// How many bytes of a file are still missing (file size minus what is present).
    fn missing_bytes(&self, ino: Ino) -> Result<u64, DriveError> {
        let mut inner = self.lock()?;
        let size = match inner.tree.get(ino).map(|inode| &inode.kind) {
            Some(EntryKind::File { size, .. }) => *size,
            _ => return Ok(0),
        };
        let present = inner
            .key_of(ino)
            .and_then(|key| inner.records.get(&key))
            .map_or(0, |record| record.blocks.present_bytes());
        Ok(size.saturating_sub(present))
    }

    /// Frees local space (see [`EvictionPolicy::plan`]). Before each
    /// removal Proton confirms that the file is there. With `dry_run`
    /// it only reports.
    ///
    /// # Errors
    /// [`DriveError::Internal`] or errors in the cache.
    pub async fn evict(
        &self,
        policy: EvictionPolicy,
        dry_run: bool,
    ) -> Result<EvictionReport, DriveError> {
        let planned = {
            let inner = self.lock()?;
            let files = self.cached_files(&inner, |_| true);
            let keys = policy.plan(&files, self.clock.now());
            Self::planned_entries(&inner, keys)
        };
        self.remove_planned(planned, dry_run, EvictionReport::default())
            .await
    }

    /// „Speicher freigeben“ for `path` (file or folder): immediately removes
    /// all local contents below it, regardless of age. Pinned,
    /// changed and open files stay and count as skipped.
    ///
    /// # Errors
    /// [`DriveError::Internal`] or errors in the cache.
    pub async fn free(
        &self,
        path: &RelativePath,
        dry_run: bool,
    ) -> Result<EvictionReport, DriveError> {
        let (planned, kept) = {
            let inner = self.lock()?;
            let files = self.cached_files(&inner, |record_path| record_path.starts_with(path));
            let (evictable, kept): (Vec<_>, Vec<_>) = files
                .into_iter()
                .filter(|file| file.bytes > 0)
                .partition(CachedFile::is_evictable);
            let keys = evictable.into_iter().map(|file| file.key).collect();
            (Self::planned_entries(&inner, keys), kept.len())
        };
        let report = EvictionReport {
            skipped: kept,
            ..EvictionReport::default()
        };
        self.remove_planned(planned, dry_run, report).await
    }

    /// The cache entries (whose path satisfies `include`) as
    /// candidates for freeing.
    fn cached_files(
        &self,
        inner: &Inner,
        include: impl Fn(&RelativePath) -> bool,
    ) -> Vec<CachedFile<CacheKey>> {
        inner
            .records
            .values()
            .filter(|record| include(&record.path))
            .map(|record| CachedFile {
                key: record.key,
                pin: inner
                    .pins
                    .effective(&record.path, self.settings.default_pin),
                last_access: record.last_access,
                bytes: record.blocks.present_bytes(),
                changed_locally: record.changed,
                open: Self::record_is_open(inner, record.node.as_ref()),
            })
            .collect()
    }

    fn record_is_open(inner: &Inner, node: Option<&NodeId>) -> bool {
        node.and_then(|node| inner.tree.ino_of(node))
            .is_some_and(|ino| inner.is_open(ino))
    }

    fn planned_entries(inner: &Inner, keys: Vec<CacheKey>) -> Vec<PlannedEviction> {
        keys.into_iter()
            .filter_map(|key| {
                inner.records.get(&key).map(|record| PlannedEviction {
                    key,
                    node: record.node.clone(),
                    path: record.path.clone(),
                })
            })
            .collect()
    }

    /// Removes the planned contents (or only reports, with `dry_run`).
    async fn remove_planned(
        &self,
        planned: Vec<PlannedEviction>,
        dry_run: bool,
        mut report: EvictionReport,
    ) -> Result<EvictionReport, DriveError> {
        for PlannedEviction { key, node, path } in planned {
            let Some(node) = node else {
                report.skipped += 1;
                continue;
            };
            let bytes = {
                let inner = self.lock()?;
                inner
                    .records
                    .get(&key)
                    .map_or(0, |r| r.blocks.present_bytes())
            };
            if dry_run {
                report.files.push(path);
                report.bytes += bytes;
                continue;
            }
            // Locally unchanged means: this content is in Proton. Proton only
            // has to confirm that the file is still there.
            match self.remote.node(&node).await {
                Ok(remote) if !remote.is_folder() => {}
                _ => {
                    report.skipped += 1;
                    continue;
                }
            }
            let mut inner = self.lock()?;
            let still_evictable = inner.records.get(&key).is_some_and(|record| {
                !record.changed && !Self::record_is_open(&inner, Some(&node))
            });
            if !still_evictable {
                report.skipped += 1;
                continue;
            }
            inner.take_record(key);
            self.cache.remove(key)?;
            self.repo.remove_cache_record(&self.settings.id, key)?;
            report.files.push(path);
            report.bytes += bytes;
        }
        Ok(report)
    }

    /// Cache usage.
    ///
    /// # Errors
    /// [`DriveError::Internal`] on a poisoned lock.
    pub fn usage(&self) -> Result<CacheUsage, DriveError> {
        let inner = self.lock()?;
        let mut usage = CacheUsage::default();
        for record in inner.records.values() {
            let bytes = record.blocks.present_bytes();
            if bytes > 0 {
                usage.files += 1;
                usage.bytes += bytes;
            }
            if record.changed {
                usage.pending_uploads += 1;
            }
        }
        Ok(usage)
    }
}
