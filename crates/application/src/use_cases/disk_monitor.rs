//! Regular check of the free space where protonsync writes (sync folders,
//! photo folder, data directory). Reports only when a disk's level changes,
//! so that a full disk does not cause a message at every check.

use std::collections::BTreeMap;
use std::path::PathBuf;

use protonsync_domain::disk::{DiskLevel, DiskSpace, DiskThresholds};

use crate::ports::DiskSpaceProbe;

/// A folder to watch, with the name it is reported under (e.g. the pair id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchedFolder {
    /// Name in messages.
    pub name: String,
    /// The folder.
    pub path: PathBuf,
}

/// A disk whose level changed since the last check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskChange {
    /// The new level.
    pub level: DiskLevel,
    /// Names of the watched folders on this disk.
    pub folders: Vec<String>,
    /// Path of the first of these folders.
    pub path: PathBuf,
    /// Size and free space.
    pub space: DiskSpace,
}

/// Remembers the level of every disk between checks.
#[derive(Debug, Default)]
pub struct DiskMonitor {
    thresholds: DiskThresholds,
    /// Last level per disk; a disk not listed counts as [`DiskLevel::Ok`].
    levels: BTreeMap<DiskKey, DiskLevel>,
}

/// Identifies a disk: its volume id, or the folder's path if the id is unknown.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum DiskKey {
    Volume(u64),
    Folder(PathBuf),
}

struct Disk {
    folders: Vec<String>,
    path: PathBuf,
    space: DiskSpace,
}

impl DiskMonitor {
    /// A monitor with the given thresholds; all disks start as [`DiskLevel::Ok`].
    #[must_use]
    pub fn new(thresholds: DiskThresholds) -> Self {
        Self {
            thresholds,
            levels: BTreeMap::new(),
        }
    }

    /// Checks all `folders` and returns the disks whose level changed.
    /// Folders that cannot be checked (e.g. not mounted yet) are skipped and
    /// logged by the caller via `on_error`.
    pub fn check(
        &mut self,
        probe: &dyn DiskSpaceProbe,
        folders: &[WatchedFolder],
        mut on_error: impl FnMut(&WatchedFolder, &std::io::Error),
    ) -> Vec<DiskChange> {
        let mut disks: BTreeMap<DiskKey, Disk> = BTreeMap::new();
        for folder in folders {
            let space = match probe.space(&folder.path) {
                Ok(space) => space,
                Err(error) => {
                    on_error(folder, &error);
                    continue;
                }
            };
            let key = space
                .volume
                .map_or_else(|| DiskKey::Folder(folder.path.clone()), DiskKey::Volume);
            disks
                .entry(key)
                .or_insert_with(|| Disk {
                    folders: Vec::new(),
                    path: folder.path.clone(),
                    space,
                })
                .folders
                .push(folder.name.clone());
        }
        let mut changes = Vec::new();
        for (key, disk) in disks {
            let level = self.thresholds.level(&disk.space);
            let previous = self.levels.insert(key, level).unwrap_or(DiskLevel::Ok);
            if level != previous {
                changes.push(DiskChange {
                    level,
                    folders: disk.folders,
                    path: disk.path,
                    space: disk.space,
                });
            }
        }
        changes
    }
}
