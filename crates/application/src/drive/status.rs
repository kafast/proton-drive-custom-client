//! Zustand einzelner Pfade für Dateimanager (ADR 0013), nur aus dem Speicher:
//! Das Anzeigen eines Ordners darf nie Netzverkehr auslösen.

use protonsync_domain::node::NodeName;
use protonsync_domain::path::RelativePath;
use protonsync_domain::pin::PinState;

use super::tree::EntryKind;
use super::{DriveError, Inner, Ino, ROOT_INO, VirtualDrive};
use crate::ports::{CacheRecord, Clock, ContentCache, DriveRepo, RemoteStorage, StateRepo};

/// Wie weit der Inhalt eines Pfads lokal vorliegt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveContent {
    /// Ein Ordner.
    Folder,
    /// Nur Platzhalter; der Inhalt wird beim Öffnen geladen (auch für noch nicht gelesene Pfade).
    OnlineOnly,
    /// Teilweise geladen.
    Partial,
    /// Vollständig lokal.
    Local,
    /// Lokal geändert, noch nicht hochgeladen.
    PendingUpload,
}

/// Zustand eines Pfads im Laufwerk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrivePathStatus {
    /// Inhalt.
    pub content: DriveContent,
    /// Ob „Immer lokal halten“ gilt.
    pub pinned: bool,
}

impl<R, C, D, S, K> VirtualDrive<R, C, D, S, K>
where
    R: RemoteStorage,
    C: ContentCache,
    D: DriveRepo,
    S: StateRepo,
    K: Clock,
{
    /// Zustand von `path`, ohne Proton zu fragen.
    ///
    /// # Errors
    /// [`DriveError::Internal`] bei vergifteter Sperre.
    pub fn path_status(&self, path: &RelativePath) -> Result<DrivePathStatus, DriveError> {
        let mut inner = self.lock()?;
        let pinned = inner.pins.effective(path, self.settings.default_pin) == PinState::Pinned;
        let content = match known_ino(&inner, path) {
            Some(ino)
                if inner
                    .tree
                    .get(ino)
                    .is_some_and(super::tree::Inode::is_folder) =>
            {
                DriveContent::Folder
            }
            Some(ino) => {
                let size = match inner.tree.get(ino).map(|inode| &inode.kind) {
                    Some(EntryKind::File { size, .. }) => Some(*size),
                    _ => None,
                };
                let record = inner.key_of(ino).and_then(|key| inner.records.get(&key));
                file_content(record, size)
            }
            // Ordner noch nicht gelesen (z. B. nach einem Neustart): der Index kennt den Pfad.
            None => file_content(
                inner.records.values().find(|record| &record.path == path),
                None,
            ),
        };
        Ok(DrivePathStatus { content, pinned })
    }
}

/// Eintrag zu `path`, soweit die Ordner auf dem Weg schon gelesen sind.
fn known_ino(inner: &Inner, path: &RelativePath) -> Option<Ino> {
    if path.is_root() {
        return Some(ROOT_INO);
    }
    let mut current = ROOT_INO;
    for segment in path.as_str().split('/') {
        let name = NodeName::new(segment).ok()?;
        current = inner.tree.child(current, &name)?;
    }
    Some(current)
}

fn file_content(record: Option<&CacheRecord>, size: Option<u64>) -> DriveContent {
    let Some(record) = record else {
        return DriveContent::OnlineOnly;
    };
    if record.changed {
        return DriveContent::PendingUpload;
    }
    let present = record.blocks.present_bytes();
    let complete = match size {
        Some(size) => present >= size,
        None => record.blocks.is_complete(),
    };
    if complete {
        DriveContent::Local
    } else if present > 0 {
        DriveContent::Partial
    } else {
        DriveContent::OnlineOnly
    }
}
