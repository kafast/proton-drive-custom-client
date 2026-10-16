//! Das virtuelle Laufwerk („nur bei Bedarf“): Proton-Ordner als Dateisystem,
//! Inhalte werden erst beim Lesen geladen und lokal zwischengespeichert.
//!
//! [`VirtualDrive`] kennt FUSE nicht. Seine Methoden entsprechen den
//! Dateisystem-Aufrufen (Nachschlagen, Auflisten, Lesen, Schreiben, …); das
//! Crate `fuse` übersetzt nur. So lässt sich alles gegen Fakes testen.
//!
//! Sperren: Der Zustand liegt hinter einem `std::sync::Mutex`, der nie über
//! ein `.await` hinweg gehalten wird. Netzwerkzugriffe laufen ohne Sperre;
//! danach wird geprüft, ob sich der Zustand inzwischen geändert hat.
//! (Der Compiler hilft dabei: Eine gehaltene Sperre ist nicht `Send`, eine
//! Zukunft mit Sperre über ein `.await` würde nicht übersetzen.)
//!
//! Siehe ADR 0010.

mod content;
mod maintenance;
mod tree;

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
use tree::{EntryKind, Tree};
pub use tree::{Ino, ROOT_INO};

/// Fehler des virtuellen Laufwerks. Das FUSE-Crate übersetzt sie in `errno`-Werte.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DriveError {
    /// Eintrag gibt es nicht.
    #[error("nicht gefunden")]
    NotFound,
    /// Name ist schon vergeben.
    #[error("existiert bereits")]
    AlreadyExists,
    /// Ein Ordner wurde erwartet.
    #[error("kein Ordner")]
    NotAFolder,
    /// Eine Datei wurde erwartet.
    #[error("ist ein Ordner")]
    IsAFolder,
    /// Ordner ist nicht leer.
    #[error("Ordner ist nicht leer")]
    NotEmpty,
    /// Ungültiger Name.
    #[error("ungültiger Name: {0}")]
    InvalidName(String),
    /// Fehler bei Proton.
    #[error(transparent)]
    Remote(#[from] RemoteError),
    /// Fehler im lokalen Zwischenspeicher.
    #[error(transparent)]
    Local(#[from] LocalError),
    /// Fehler im Zustandsspeicher.
    #[error(transparent)]
    State(#[from] StateError),
    /// Interner Fehler (z. B. vergiftete Sperre nach einem Absturz eines Threads).
    #[error("interner Fehler: {0}")]
    Internal(&'static str),
}

/// Einstellungen eines Laufwerks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveSettings {
    /// Kennung im Zustand, z. B. `mount:home` (auch Besitzer im Papierkorb).
    pub id: SyncPairId,
    /// Gilt, wo keine Anheft-Regel greift („Immer lokal halten“ als Voreinstellung).
    pub default_pin: PinState,
}

/// Angaben zu einem Eintrag, wie das Dateisystem sie zeigt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attributes {
    /// Nummer.
    pub ino: Ino,
    /// Ordner oder Datei.
    pub is_folder: bool,
    /// Größe in Bytes (Ordner: 0).
    pub size: u64,
    /// Änderungszeit.
    pub modified: Timestamp,
}

/// Ein Eintrag beim Auflisten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// Nummer.
    pub ino: Ino,
    /// Name.
    pub name: NodeName,
    /// Ordner oder Datei.
    pub is_folder: bool,
}

/// Veränderlicher Zustand hinter der Sperre.
struct Inner {
    tree: Tree,
    records: HashMap<CacheKey, CacheRecord>,
    key_of_node: HashMap<NodeId, CacheKey>,
    key_of_ino: HashMap<Ino, CacheKey>,
    /// Anzahl offener Handles je Eintrag.
    open: HashMap<Ino, u32>,
    /// Zählt Schreibvorgänge je Inhalt; ein Upload gilt nur, wenn sich der
    /// Zähler währenddessen nicht geändert hat.
    generation: HashMap<CacheKey, u64>,
    next_key: u64,
    pins: PinRules,
    cursor: Option<EventCursor>,
}

impl Inner {
    /// Schlüssel des Inhalts zu einem Eintrag, falls es einen gibt.
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

/// Revision einer Datei, `None` bei Ordnern.
fn revision_of(node: &Node) -> Option<&RevisionId> {
    match &node.kind {
        NodeKind::File { revision, .. } => Some(revision),
        NodeKind::Folder => None,
    }
}

/// Ersetzt den Anfang `from` eines Pfades durch `to` (für Umbenennen eines Ordners).
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

/// Das virtuelle Laufwerk über einem Ordner in Proton.
pub struct VirtualDrive<R, C, D, S, K> {
    remote: R,
    cache: C,
    repo: D,
    state: S,
    clock: K,
    settings: DriveSettings,
    inner: Mutex<Inner>,
}

impl<R, C, D, S, K> VirtualDrive<R, C, D, S, K>
where
    R: RemoteStorage,
    C: ContentCache,
    D: DriveRepo,
    S: StateRepo,
    K: Clock,
{
    /// Baut das Laufwerk über dem Ordner `root` und lädt den gespeicherten
    /// Zwischenspeicher-Index samt Anheft-Regeln.
    ///
    /// # Errors
    /// [`DriveError::State`], wenn der Index nicht lesbar ist.
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
        })
    }

    /// Einstellungen des Laufwerks.
    #[must_use]
    pub fn settings(&self) -> &DriveSettings {
        &self.settings
    }

    fn lock(&self) -> Result<MutexGuard<'_, Inner>, DriveError> {
        self.inner
            .lock()
            .map_err(|_| DriveError::Internal("Sperre vergiftet"))
    }

    fn save(&self, record: &CacheRecord) -> Result<(), DriveError> {
        Ok(self.repo.save_cache_record(&self.settings.id, record)?)
    }

    /// Liest den Ordner bei Proton, falls noch nicht geschehen.
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

    /// Angaben zu einem Eintrag.
    ///
    /// # Errors
    /// [`DriveError::NotFound`].
    pub fn attributes(&self, ino: Ino) -> Result<Attributes, DriveError> {
        self.lock()?.attributes(ino)
    }

    /// Sucht `name` in `parent`.
    ///
    /// # Errors
    /// [`DriveError::NotFound`], [`DriveError::NotAFolder`] oder Fehler bei Proton.
    pub async fn lookup(&self, parent: Ino, name: &NodeName) -> Result<Attributes, DriveError> {
        self.ensure_listed(parent).await?;
        let inner = self.lock()?;
        let ino = inner.tree.child(parent, name).ok_or(DriveError::NotFound)?;
        inner.attributes(ino)
    }

    /// Inhalt eines Ordners, sortiert nach Name.
    ///
    /// # Errors
    /// [`DriveError::NotFound`], [`DriveError::NotAFolder`] oder Fehler bei Proton.
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

    /// Legt einen Ordner an (sofort in Proton).
    ///
    /// # Errors
    /// [`DriveError::AlreadyExists`] oder Fehler bei Proton.
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
        // Ein neuer Ordner ist leer; Nachfragen bei Proton ist unnötig.
        inner.tree.set_children(ino, &[]);
        inner.attributes(ino)
    }

    /// Legt eine Datei in den Papierkorb (Proton und protonsync).
    ///
    /// # Errors
    /// [`DriveError::NotFound`], [`DriveError::IsAFolder`] oder Fehler bei Proton.
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

    /// Legt einen leeren Ordner in den Papierkorb.
    ///
    /// # Errors
    /// [`DriveError::NotEmpty`], [`DriveError::NotAFolder`], [`DriveError::NotFound`]
    /// oder Fehler bei Proton.
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

    /// Papierkorb für einen Eintrag: in Proton und als Eintrag bei protonsync.
    /// Eine noch nicht hochgeladene Datei wird nur lokal verworfen.
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

    /// Entfernt einen Eintrag samt Inhalten im Zwischenspeicher darunter.
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

    /// Benennt um oder verschiebt. Ein vorhandenes Ziel wird ersetzt wie unter
    /// Linux üblich: Eine Datei bekommt dabei den neuen Inhalt als neue Revision
    /// (so speichern viele Programme: erst in eine Hilfsdatei, dann umbenennen),
    /// ein leerer Ordner kommt in den Papierkorb.
    ///
    /// # Errors
    /// [`DriveError::NotFound`], [`DriveError::NotEmpty`], [`DriveError::IsAFolder`],
    /// [`DriveError::NotAFolder`] oder Fehler bei Proton.
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
        if let Some(target) = target {
            if target == source {
                return Ok(());
            }
            if self.replace_target(source, target).await? {
                return Ok(());
            }
        }
        self.move_entry(source, new_parent, new_name).await
    }

    /// Räumt ein vorhandenes Ziel. `true`, wenn die Quelle dabei schon im Ziel
    /// aufgegangen ist (Datei ersetzt Datei).
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
                // Das Ziel war selbst noch nicht hochgeladen: einfach verwerfen.
                self.trash_entry(target).await?;
                Ok(false)
            }
            (false, false) => {
                self.replace_file_content(source, target).await?;
                Ok(true)
            }
        }
    }

    /// Verschiebt bzw. benennt einen Eintrag um, ohne Ziel am neuen Ort.
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
