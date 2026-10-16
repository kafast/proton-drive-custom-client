//! API data types for FUSE drives (placeholder mode).

use serde::{Deserialize, Serialize};

/// Prefix under which a drive's trash entries appear in [`crate::TrashInfo::pair`],
/// e.g. `mount:home`.
pub const MOUNT_TRASH_PREFIX: &str = "mount:";

/// A drive with its status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountInfo {
    /// Id from the configuration.
    pub id: String,
    /// Mount point.
    pub mountpoint: String,
    /// Folder in "My files".
    pub remote_path: String,
    /// Whether the drive is currently mounted.
    pub mounted: bool,
    /// Default for "Always keep local".
    pub keep_local: bool,
    /// Cache usage; absent when not logged in.
    pub cache: Option<CacheInfo>,
    /// Explicitly set rules for "Always keep local".
    pub pins: Vec<PinInfo>,
}

/// Cache usage of a drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheInfo {
    /// Files with content present locally.
    pub files: u64,
    /// Bytes used.
    pub bytes: u64,
    /// Files with local changes that are still to be uploaded.
    pub pending_uploads: u64,
}

/// A rule for "Always keep local".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinInfo {
    /// Path relative to the mount point.
    pub path: String,
    /// What the rule says.
    pub state: PinChoice,
}

/// "Always keep local" or "only on demand".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinChoice {
    /// Always keep local (downloaded fully, never evicted).
    Pinned,
    /// Only download on demand (may be evicted).
    Unpinned,
}

/// Result of freeing space for a drive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreeResult {
    /// Drive.
    pub mount: String,
    /// `true`: only planned, nothing removed.
    pub dry_run: bool,
    /// Evicted (or to-be-evicted) files, relative to the mount point.
    pub files: Vec<String>,
    /// Bytes freed by this.
    pub bytes: u64,
    /// Skipped files (pinned, changed, open or not confirmed).
    pub skipped: u64,
}
