//! API data types for file managers: status of individual paths (ADR 0013).

use serde::{Deserialize, Serialize};

/// Status of a path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathStatusInfo {
    /// The requested path.
    pub path: String,
    /// Status.
    pub state: PathState,
    /// Whether the path is in a drive (where pinning and freeing exist).
    pub in_drive: bool,
    /// Whether "Always keep local" applies (only in drives).
    pub pinned: bool,
}

/// What protonsync knows about a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathState {
    /// Not in any drive, sync or photo folder (or internal, e.g. the trash).
    Outside,
    /// A folder in a drive, sync or photo folder.
    Folder,
    /// Drive: placeholder only, the content loads on opening.
    OnlineOnly,
    /// Drive: partly downloaded.
    Partial,
    /// Drive: fully local.
    Local,
    /// Drive: changed locally, not yet uploaded.
    PendingUpload,
    /// Sync or photo folder: unchanged since the last sync.
    Synced,
    /// Sync or photo folder: new or changed; the next sync transfers it.
    Pending,
    /// Sync folder: open conflict.
    Conflict,
}
