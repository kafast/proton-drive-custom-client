//! Inhalte des Laufwerks: Lesen über den Block-Zwischenspeicher, Schreiben,
//! Anlegen und Hochladen geänderter Dateien.

use protonsync_domain::cache::{BLOCK_BYTES, BlockMap, ByteRange};
use protonsync_domain::conflict::conflict_copy_name;
use protonsync_domain::node::{Node, NodeId, NodeName, RevisionId};
use protonsync_domain::path::RelativePath;

use super::tree::EntryKind;
use super::{Attributes, DriveError, Inner, Ino, ROOT_INO, VirtualDrive, revision_of};
use crate::ports::{
    CacheKey, CacheRecord, Clock, ContentCache, DriveRepo, RemoteError, RemoteStorage, StateRepo,
};

/// So viel wird beim Lesen über die Anfrage hinaus vorausgeladen (ein Block).
const READ_AHEAD_BYTES: u64 = BLOCK_BYTES;

/// Was zum Laden fehlender Blöcke nötig ist (ohne Sperre benutzbar).
struct FetchPlan {
    key: CacheKey,
    node: Option<NodeId>,
    revision: Option<RevisionId>,
    missing: Vec<ByteRange>,
}

/// Stand eines Inhalts vor dem Hochladen.
struct UploadSnapshot {
    record: CacheRecord,
    generation: u64,
    ino: Option<Ino>,
    parent_node: Option<NodeId>,
    name: NodeName,
}

/// Ergebnis eines Uploads.
struct Uploaded {
    node: Node,
    /// Bei einem Konflikt: der aktuelle Stand des Originals in Proton.
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
    /// Plant unter der Sperre, welche Blöcke für `wanted` fehlen. Legt den
    /// Eintrag im Zwischenspeicher bei Bedarf an und verwirft veraltete Inhalte.
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
            .ok_or(DriveError::Internal("Eintrag fehlt"))?;
        if !record.changed && (record.revision != revision || record.blocks.size() != size) {
            // In Proton gibt es eine neue Fassung: die alte lokale Kopie verwerfen.
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

    /// Lädt die fehlenden Blöcke (ohne Sperre) und vermerkt sie.
    async fn fetch(&self, plan: FetchPlan) -> Result<CacheKey, DriveError> {
        if plan.missing.is_empty() {
            return Ok(plan.key);
        }
        let node = plan
            .node
            .ok_or(DriveError::Internal("neue Datei ohne Inhalt"))?;
        for range in plan.missing {
            let data = self
                .remote
                .download_range(&node, range.offset, range.length)
                .await?;
            if u64::try_from(data.len()).ok() != Some(range.length) {
                return Err(RemoteError::Transfer(format!(
                    "{} statt {} Bytes erhalten",
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

    /// Lädt alle fehlenden Blöcke einer Datei.
    pub(super) async fn ensure_complete(&self, ino: Ino) -> Result<CacheKey, DriveError> {
        let plan = {
            let mut inner = self.lock()?;
            self.plan_fetch(
                &mut inner,
                ino,
                ByteRange {
                    offset: 0,
                    length: u64::MAX,
                },
            )?
        };
        self.fetch(plan).await
    }

    /// Liest bis zu `length` Bytes ab `offset`; fehlende Blöcke werden geladen.
    ///
    /// # Errors
    /// [`DriveError::IsAFolder`], [`DriveError::NotFound`] oder Fehler bei Proton
    /// bzw. im Zwischenspeicher.
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
        let length = usize::try_from(length).map_err(|_| DriveError::Internal("Länge"))?;
        Ok(self.cache.read(key, offset, length)?)
    }

    /// Öffnet eine Datei. Zum Schreiben wird sie vorher vollständig geladen.
    ///
    /// # Errors
    /// Fehler beim Laden.
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

    /// Schreibt in eine zum Schreiben geöffnete Datei.
    ///
    /// # Errors
    /// [`DriveError::NotFound`] oder Fehler im Zwischenspeicher.
    pub fn write(&self, ino: Ino, offset: u64, data: &[u8]) -> Result<u32, DriveError> {
        let mut inner = self.lock()?;
        let key = inner.key_of(ino).ok_or(DriveError::NotFound)?;
        self.cache.write(key, offset, data)?;
        let length = u64::try_from(data.len()).map_err(|_| DriveError::Internal("Länge"))?;
        let end = offset.saturating_add(length);
        let now = self.clock.now();
        let record = inner
            .records
            .get_mut(&key)
            .ok_or(DriveError::Internal("Eintrag fehlt"))?;
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
        u32::try_from(data.len()).map_err(|_| DriveError::Internal("Länge"))
    }

    /// Ändert die Größe (Abschneiden oder Verlängern).
    ///
    /// # Errors
    /// Fehler beim Laden oder im Zwischenspeicher.
    pub async fn set_size(&self, ino: Ino, size: u64) -> Result<Attributes, DriveError> {
        let key = self.ensure_complete(ino).await?;
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

    /// Legt eine neue, leere Datei an und öffnet sie. Hochgeladen wird nach
    /// dem Schließen.
    ///
    /// # Errors
    /// [`DriveError::AlreadyExists`] oder Fehler beim Auflisten.
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

    /// Schließt ein Handle. Nach dem letzten Schließen wird eine geänderte
    /// Datei hochgeladen; scheitert das, bleibt sie für [`Self::upload_pending`] vorgemerkt.
    ///
    /// # Errors
    /// Fehler beim Hochladen.
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

    /// Lädt alle geänderten, nicht geöffneten Dateien hoch (z. B. nach einem
    /// Neustart oder Netzfehler). Liefert die Pfade, bei denen es scheiterte.
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

    /// Ersetzt den Inhalt der Datei `target` durch den von `source` (Umbenennen
    /// über eine vorhandene Datei). Das Ziel behält seinen Knoten und bekommt
    /// eine neue Revision; die Quelle verschwindet.
    pub(super) async fn replace_file_content(
        &self,
        source: Ino,
        target: Ino,
    ) -> Result<(), DriveError> {
        let source_key = self.ensure_complete(source).await?;
        // Eine schon hochgeladene Quelle verschwindet unter ihrem alten Namen.
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
                .ok_or(DriveError::Internal("Eintrag fehlt"))?;
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

    /// Lädt einen geänderten Inhalt hoch, wenn er nicht geöffnet ist.
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
                    // In Proton gibt es inzwischen eine andere Fassung: beide behalten.
                    let parent = current.parent.clone().ok_or(DriveError::NotFound)?;
                    let copy = self.copy_name(&snapshot.name)?;
                    let node = self.upload_new(&parent, &copy, record).await?;
                    return Ok(Uploaded {
                        node,
                        conflict_with: Some(current),
                    });
                }
                // In Proton gelöscht: die lokale Fassung als neue Datei behalten.
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
                // Das Original zeigt wieder die Fassung aus Proton, die Kopie bekommt den Inhalt.
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

    /// Ordner zu einem Pfad im Laufwerk; fehlende Ordner werden angelegt
    /// (damit eine lokale Fassung nie verloren geht).
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

/// Größe und Änderungszeit einer Datei in der Tabelle setzen.
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
