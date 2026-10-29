//! Sync pairs: which local folder is synchronised with which remote folder, and how.

use std::path::PathBuf;

use crate::node::NodeId;

/// Sync direction of a sync pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyncDirection {
    /// Changes flow in both directions (default).
    #[default]
    TwoWay,
    /// Only remote changes are applied locally.
    DownloadOnly,
    /// Only local changes are uploaded.
    UploadOnly,
    /// Backup of Proton: like [`Self::DownloadOnly`], but nothing is ever deleted
    /// locally and a local file replaced by a newer remote version is moved to the
    /// version archive first (see [`crate::archive`] and
    /// [`crate::conflict::decide`]).
    BackupOnly,
}

impl SyncDirection {
    /// May local changes be transferred to Proton?
    #[must_use]
    pub const fn allows_upload(self) -> bool {
        matches!(self, Self::TwoWay | Self::UploadOnly)
    }

    /// May remote changes be applied locally?
    #[must_use]
    pub const fn allows_download(self) -> bool {
        matches!(self, Self::TwoWay | Self::DownloadOnly | Self::BackupOnly)
    }
}

/// How the files are present locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StorageMode {
    /// All files are fully on disk (sync folder).
    #[default]
    FullLocal,
    /// Files are placeholders and are loaded on demand (FUSE mount).
    OnDemand,
}

/// Id of a sync pair in the local configuration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SyncPairId(String);

impl SyncPairId {
    /// Creates an id.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The id as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Link between a local folder and a folder in Proton Drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPair {
    /// Id of the pair.
    pub id: SyncPairId,
    /// Local folder or mount point.
    pub local_root: PathBuf,
    /// Remote folder.
    pub remote_root: NodeId,
    /// Sync direction.
    pub direction: SyncDirection,
    /// Fully local or on demand.
    pub storage_mode: StorageMode,
    /// Protection of the remote side: local deletions and replacements are
    /// never passed on to Proton (see [`crate::conflict::protect_remote`]).
    pub protect_remote: bool,
    /// Version archive on both sides: deletions and replaced versions are moved
    /// into an `_archiv` folder next to them instead of the trash or being
    /// overwritten (see [`crate::conflict::archive_instead`]).
    pub archive_versions: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_way_is_the_default() {
        assert_eq!(SyncDirection::default(), SyncDirection::TwoWay);
    }

    #[test]
    fn direction_permissions() {
        assert!(SyncDirection::TwoWay.allows_upload());
        assert!(SyncDirection::TwoWay.allows_download());
        assert!(!SyncDirection::DownloadOnly.allows_upload());
        assert!(SyncDirection::DownloadOnly.allows_download());
        assert!(SyncDirection::UploadOnly.allows_upload());
        assert!(!SyncDirection::UploadOnly.allows_download());
        assert!(!SyncDirection::BackupOnly.allows_upload());
        assert!(SyncDirection::BackupOnly.allows_download());
    }
}
