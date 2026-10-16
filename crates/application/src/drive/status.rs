//! State of individual paths for file managers (ADR 0013), from memory only:
//! showing a folder must never cause network traffic.

use protonsync_domain::node::NodeName;
use protonsync_domain::path::RelativePath;
use protonsync_domain::pin::PinState;

use super::tree::EntryKind;
use super::{DriveError, Inner, Ino, ROOT_INO, VirtualDrive};
use crate::ports::{CacheRecord, Clock, ContentCache, DriveRepo, RemoteStorage, StateRepo};

/// How much of the content of a path is present locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveContent {
    /// A folder.
    Folder,
    /// Placeholder only; the content is loaded when opened (also for paths not read yet).
    OnlineOnly,
    /// Partially loaded.
    Partial,
    /// Fully local.
    Local,
    /// Changed locally, not uploaded yet.
    PendingUpload,
}

/// State of a path in the drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrivePathStatus {
    /// Content.
    pub content: DriveContent,
    /// Whether „Immer lokal halten“ applies.
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
    /// State of `path`, without asking Proton.
    ///
    /// # Errors
    /// [`DriveError::Internal`] on a poisoned lock.
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
            // Folder not read yet (e.g. after a restart): the index knows the path.
            None => file_content(
                inner.records.values().find(|record| &record.path == path),
                None,
            ),
        };
        Ok(DrivePathStatus { content, pinned })
    }
}

/// Entry for `path`, as far as the folders on the way have already been read.
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
