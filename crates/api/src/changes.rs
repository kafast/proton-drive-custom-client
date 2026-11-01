//! API data types for the change log.

use serde::{Deserialize, Serialize};

/// A change protonsync made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeInfo {
    /// Who made it: the id of a sync pair, or for other changes the name of
    /// their trash owner (`job:<id>`, `mount:<id>`, `photos:library`).
    pub pair: String,
    /// When (Unix seconds).
    pub at: i64,
    /// Path relative to the root of the sync pair, job target, drive or photo folder.
    pub path: String,
    /// The side that was changed.
    pub side: ChangedSideInfo,
    /// What was done.
    pub kind: ChangeKindInfo,
    /// `true` for a folder.
    pub is_folder: bool,
}

/// The side that was changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangedSideInfo {
    /// The local folder (the change came from Proton).
    Local,
    /// Proton (the change came from the local folder).
    Remote,
}

/// What was done to the entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKindInfo {
    /// Newly created.
    Created,
    /// New version.
    Modified,
    /// Moved to the trash.
    Trashed,
    /// Moved into the version archive.
    Archived,
    /// Old version archived, new version put in its place.
    ArchivedAndReplaced,
    /// Deleted on the other side, the copy here was kept.
    KeptCopy,
    /// Conflict: both versions were kept.
    Conflict,
    /// Restored from the trash at its original place.
    Restored,
}
