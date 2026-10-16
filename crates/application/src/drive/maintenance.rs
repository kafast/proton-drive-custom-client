//! Pflege des Laufwerks: Proton-Ereignisse übernehmen, Anheft-Regeln,
//! angeheftete Dateien vorab laden und Speicher freigeben.

use std::collections::VecDeque;

use protonsync_domain::node::{Node, NodeId, NodeName};
use protonsync_domain::path::RelativePath;
use protonsync_domain::pin::{CachedFile, EvictionPolicy, PinState};

use super::tree::EntryKind;
use super::{DriveError, Ino, ROOT_INO, VirtualDrive};
use crate::ports::{
    CacheKey, Clock, ContentCache, DriveRepo, RemoteError, RemoteEvent, RemoteStorage, StateRepo,
};

/// Ab so vielen Ereignissen werden die Ordner lieber neu gelesen, statt jeden
/// Knoten einzeln bei Proton nachzufragen.
const MAX_EVENTS_PER_REFRESH: usize = 200;

/// Ergebnis von [`VirtualDrive::refresh`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RefreshReport {
    /// Verarbeitete Ereignisse.
    pub events: usize,
    /// `true`, wenn alle gelesenen Ordner verworfen wurden (Lücke oder zu viele Ereignisse).
    pub listings_forgotten: bool,
}

/// Ergebnis von [`VirtualDrive::prefetch_pinned`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PrefetchReport {
    /// Vollständig geladene Dateien (vorher unvollständig).
    pub files: usize,
    /// Dafür geladene Bytes (ungefähr: Größe der Dateien).
    pub bytes: u64,
}

/// Ergebnis von [`VirtualDrive::evict`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EvictionReport {
    /// Freigegebene (bzw. beim Probelauf: freizugebende) Dateien.
    pub files: Vec<RelativePath>,
    /// Dadurch frei werdende Bytes.
    pub bytes: u64,
    /// Übersprungen, weil Proton die Datei nicht (mehr) bestätigte.
    pub skipped: usize,
}

/// Belegung des Zwischenspeichers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheUsage {
    /// Dateien mit lokal vorhandenem Inhalt.
    pub files: usize,
    /// Belegte Bytes.
    pub bytes: u64,
    /// Dateien mit lokalen Änderungen, die noch hochgeladen werden.
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
    /// Übernimmt die Änderungen in Proton seit dem letzten Aufruf. Der erste
    /// Aufruf merkt sich nur den Startpunkt.
    ///
    /// # Errors
    /// Fehler bei Proton; der Startpunkt bleibt dann unverändert.
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

    /// Ein geänderter oder neuer Knoten: nur übernehmen, wenn er oder sein
    /// Elternordner bekannt ist (sonst liegt er außerhalb des Gelesenen).
    fn apply_node(&self, node: &Node) -> Result<(), DriveError> {
        let mut inner = self.lock()?;
        let parent = node.parent.as_ref().and_then(|p| inner.tree.ino_of(p));
        let known = inner.tree.ino_of(&node.id);
        match (known, parent) {
            (_, Some(parent)) if inner.tree.is_listed(parent) => {
                inner.tree.upsert(parent, node);
            }
            (Some(ino), _) if ino != ROOT_INO => {
                // Aus dem gelesenen Bereich weg verschoben.
                inner.tree.remove(ino);
            }
            _ => {}
        }
        Ok(())
    }

    /// Ein Knoten liegt jetzt im Proton-Papierkorb. Lokal Geändertes darunter
    /// bleibt erhalten und wird später als neue Datei hochgeladen.
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

    /// Setzt „Immer lokal halten“ (`Pinned`), „Nur bei Bedarf“ (`Unpinned`)
    /// oder entfernt die Regel (`None`) für `path`. Regeln darunter entfallen,
    /// damit die neue Regel für den ganzen Ordner gilt.
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

    /// Was für `path` gilt.
    ///
    /// # Errors
    /// [`DriveError::Internal`] bei vergifteter Sperre.
    pub fn pin_state(&self, path: &RelativePath) -> Result<PinState, DriveError> {
        Ok(self.lock()?.pins.effective(path, self.settings.default_pin))
    }

    /// Die ausdrücklich gesetzten Regeln.
    ///
    /// # Errors
    /// [`DriveError::Internal`] bei vergifteter Sperre.
    pub fn pin_rules(&self) -> Result<Vec<(RelativePath, PinState)>, DriveError> {
        Ok(self
            .lock()?
            .pins
            .iter()
            .map(|(path, state)| (path.clone(), state))
            .collect())
    }

    /// Eintrag zu einem Pfad; Ordner auf dem Weg werden bei Bedarf gelesen.
    ///
    /// # Errors
    /// [`DriveError::NotFound`] oder Fehler bei Proton.
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

    /// Lädt alle Dateien vollständig, für die „Immer lokal halten“ gilt.
    /// Dafür werden die angehefteten Ordner einmal ganz gelesen.
    ///
    /// # Errors
    /// Fehler bei Proton oder im Zwischenspeicher (bereits Geladenes bleibt).
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
                // Eine Regel für einen inzwischen gelöschten Pfad: nichts zu tun.
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

    /// Wie viele Bytes einer Datei noch fehlen (Dateigröße minus Vorhandenes).
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

    /// Gibt lokalen Speicher frei (siehe [`EvictionPolicy::plan`]). Vor jedem
    /// Entfernen bestätigt Proton, dass die Datei dort liegt. Mit `dry_run`
    /// wird nur berichtet.
    ///
    /// # Errors
    /// [`DriveError::Internal`] oder Fehler im Zwischenspeicher.
    pub async fn evict(
        &self,
        policy: EvictionPolicy,
        dry_run: bool,
    ) -> Result<EvictionReport, DriveError> {
        let planned = {
            let inner = self.lock()?;
            let files: Vec<CachedFile<CacheKey>> = inner
                .records
                .values()
                .map(|record| {
                    let open = record
                        .node
                        .as_ref()
                        .and_then(|node| inner.tree.ino_of(node))
                        .is_some_and(|ino| inner.is_open(ino));
                    CachedFile {
                        key: record.key,
                        pin: inner
                            .pins
                            .effective(&record.path, self.settings.default_pin),
                        last_access: record.last_access,
                        bytes: record.blocks.present_bytes(),
                        changed_locally: record.changed,
                        open,
                    }
                })
                .collect();
            let keys = policy.plan(&files, self.clock.now());
            let mut planned = Vec::with_capacity(keys.len());
            for key in keys {
                if let Some(record) = inner.records.get(&key) {
                    planned.push((key, record.node.clone(), record.path.clone()));
                }
            }
            planned
        };
        let mut report = EvictionReport::default();
        for (key, node, path) in planned {
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
            match self.remote.node(&node).await {
                Ok(remote) if !remote.is_folder() => {}
                _ => {
                    report.skipped += 1;
                    continue;
                }
            }
            let mut inner = self.lock()?;
            let still_evictable = inner.records.get(&key).is_some_and(|record| {
                !record.changed
                    && !inner
                        .tree
                        .ino_of(&node)
                        .is_some_and(|ino| inner.is_open(ino))
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

    /// Belegung des Zwischenspeichers.
    ///
    /// # Errors
    /// [`DriveError::Internal`] bei vergifteter Sperre.
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
