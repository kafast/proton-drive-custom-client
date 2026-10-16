//! API data types for the photo sync and duplicate detection (ADR 0012).

use serde::{Deserialize, Serialize};

use crate::{Direction, FailureInfo, SkippedInfo};

/// The photos' trash entries appear under this name in [`crate::TrashInfo::pair`].
pub const PHOTOS_TRASH_OWNER: &str = "photos:library";

/// Status of the photo sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoStatus {
    /// Local photo folder.
    pub folder: String,
    /// Direction.
    pub direction: Direction,
    /// Whether a sync is running.
    pub syncing: bool,
    /// Number of photos in Proton; absent until the library has been read.
    pub photos: Option<usize>,
    /// Result of the last run.
    pub last_run: Option<PhotoRunInfo>,
    /// Entries in the trash.
    pub trash_entries: usize,
}

/// A run of the photo sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoRunInfo {
    /// End (Unix seconds).
    pub finished_at: i64,
    /// Outcome.
    pub outcome: PhotoOutcomeInfo,
}

/// Outcome of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PhotoOutcomeInfo {
    /// Dry run: what would happen.
    Planned(PhotoCounts),
    /// Applied.
    Applied(PhotoCounts),
    /// Failed before anything happened.
    Failed {
        /// Error message.
        message: String,
    },
}

/// Figures of a (planned or applied) run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoCounts {
    /// Downloaded.
    pub downloaded: usize,
    /// Uploaded.
    pub uploaded: usize,
    /// Only linked (content equal on both sides).
    pub linked: usize,
    /// To the trash locally.
    pub trashed_local: usize,
    /// To the trash in Proton.
    pub trashed_remote: usize,
    /// Not transferred because of the direction.
    pub ignored: usize,
    /// Skipped paths.
    pub skipped: Vec<SkippedInfo>,
    /// Failed steps (only for applied runs).
    pub failures: Vec<FailureInfo>,
}

/// Why photos count as duplicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateReasonInfo {
    /// Same content: certainly the same photo.
    SameContent,
    /// Same name and same capture second: probably the same photo.
    SameNameAndCaptureTime,
}

/// A group of duplicates; only a suggestion, nothing is deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateGroupInfo {
    /// Reason.
    pub reason: DuplicateReasonInfo,
    /// Capture time (Unix seconds).
    pub captured: i64,
    /// Suggestion: keep this photo.
    pub keep: PhotoInfo,
    /// The remaining photos (candidates for deletion).
    pub others: Vec<PhotoInfo>,
}

/// A photo in Proton.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhotoInfo {
    /// Id; used to delete a duplicate.
    pub id: String,
    /// File name.
    pub name: String,
    /// Size in bytes.
    pub size: u64,
    /// Uploaded (Unix seconds).
    pub uploaded: i64,
    /// Marked as favourite.
    pub favorite: bool,
    /// In this many albums.
    pub albums: usize,
}

/// A duplicate that was (or would be) moved to the trash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrashedPhotoInfo {
    /// Id.
    pub id: String,
    /// Path in the photo folder.
    pub path: String,
}
