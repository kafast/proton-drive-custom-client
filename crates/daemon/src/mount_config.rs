//! FUSE drives (`[[mount]]`) and their cache (`[cache]`) in the
//! configuration file (placeholder mode, ADR 0010).
//!
//! Example:
//!
//! ```toml
//! [[mount]]
//! id = "home"
//! mountpoint = "/home/krischan/ProtonDrive"
//! remote = ""             # folder in "My files"; empty for everything
//! keep_local = false      # default for "always keep local"
//!
//! [cache]
//! evict_after_days = 30   # free unused files after this
//! max_size_gb = 10        # size limit of the cache
//! ```

use std::path::PathBuf;

use protonsync_domain::path::RelativePath;
use protonsync_domain::pin::EvictionPolicy;
use serde::Deserialize;

use crate::config::{ConfigError, is_valid_id, parse_remote};

const DEFAULT_EVICT_AFTER_DAYS: u32 = 30;
const DEFAULT_MAX_SIZE_GB: u64 = 10;
const BYTES_PER_GB: u64 = 1_000_000_000;

/// A FUSE drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountConfig {
    /// Identifier (lowercase letters, digits, `-`, `_`).
    pub id: String,
    /// Mount point (absolute); created at startup.
    pub mountpoint: PathBuf,
    /// Folder in "My files"; empty for "My files" itself.
    pub remote: RelativePath,
    /// Default for "always keep local" where no rule applies.
    pub keep_local: bool,
}

/// Settings of the cache (for all drives).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheSettings {
    /// When files are freed.
    pub policy: EvictionPolicy,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawMount {
    id: String,
    mountpoint: PathBuf,
    #[serde(default)]
    remote: String,
    #[serde(default)]
    keep_local: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawCache {
    evict_after_days: Option<u32>,
    max_size_gb: Option<u64>,
}

impl TryFrom<RawMount> for MountConfig {
    type Error = ConfigError;

    fn try_from(raw: RawMount) -> Result<Self, Self::Error> {
        if !is_valid_id(&raw.id) {
            return Err(ConfigError::Invalid(format!(
                "invalid drive identifier {:?} (allowed: a-z, 0-9, -, _)",
                raw.id
            )));
        }
        if !raw.mountpoint.is_absolute() {
            return Err(ConfigError::Invalid(format!(
                "drive {}: mount point must be an absolute path",
                raw.id
            )));
        }
        let remote = parse_remote(&raw.remote).map_err(|message| {
            ConfigError::Invalid(format!("drive {}: remote folder: {message}", raw.id))
        })?;
        Ok(Self {
            id: raw.id,
            mountpoint: raw.mountpoint,
            remote,
            keep_local: raw.keep_local,
        })
    }
}

impl RawCache {
    pub(crate) fn into_settings(self) -> Result<CacheSettings, ConfigError> {
        let days = self.evict_after_days.unwrap_or(DEFAULT_EVICT_AFTER_DAYS);
        let max_bytes = self
            .max_size_gb
            .unwrap_or(DEFAULT_MAX_SIZE_GB)
            .saturating_mul(BYTES_PER_GB);
        let policy = EvictionPolicy::new(days, max_bytes)
            .map_err(|e| ConfigError::Invalid(format!("[cache] evict_after_days: {e}")))?;
        Ok(CacheSettings { policy })
    }
}

/// Drive identifiers must be unique.
pub(crate) fn ensure_unique_mount_ids(mounts: &[MountConfig]) -> Result<(), ConfigError> {
    for (index, mount) in mounts.iter().enumerate() {
        if mounts[..index].iter().any(|other| other.id == mount.id) {
            return Err(ConfigError::Invalid(format!(
                "duplicate drive identifier {}",
                mount.id
            )));
        }
    }
    Ok(())
}
