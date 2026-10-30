//! API data types for the free-space warning.

use serde::{Deserialize, Serialize};

/// How full a disk is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskLevelInfo {
    /// Enough free space again.
    Ok,
    /// Almost full (below the warning threshold).
    Low,
    /// Full (below the critical threshold); downloads will fail.
    Full,
}

/// A disk protonsync writes to has changed its level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskSpaceInfo {
    /// The new level.
    pub level: DiskLevelInfo,
    /// What lies on this disk: sync pair ids, `photos`, `data`.
    pub folders: Vec<String>,
    /// Path of the first of these folders.
    pub path: String,
    /// Bytes still free.
    pub available_bytes: u64,
    /// Size of the disk in bytes.
    pub total_bytes: u64,
}
