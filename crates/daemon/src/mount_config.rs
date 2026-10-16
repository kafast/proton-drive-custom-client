//! FUSE-Laufwerke (`[[mount]]`) und ihr Zwischenspeicher (`[cache]`) in der
//! Konfigurationsdatei (Platzhalter-Modus, ADR 0010).
//!
//! Beispiel:
//!
//! ```toml
//! [[mount]]
//! id = "home"
//! mountpoint = "/home/krischan/ProtonDrive"
//! remote = ""             # Ordner in „Meine Dateien“; leer für alles
//! keep_local = false      # Voreinstellung „Immer lokal halten“
//!
//! [cache]
//! evict_after_days = 30   # unbenutzte Dateien danach freigeben
//! max_size_gb = 10        # Größenlimit des Zwischenspeichers
//! ```

use std::path::PathBuf;

use protonsync_domain::path::RelativePath;
use protonsync_domain::pin::EvictionPolicy;
use serde::Deserialize;

use crate::config::{ConfigError, is_valid_id, parse_remote};

const DEFAULT_EVICT_AFTER_DAYS: u32 = 30;
const DEFAULT_MAX_SIZE_GB: u64 = 10;
const BYTES_PER_GB: u64 = 1_000_000_000;

/// Ein FUSE-Laufwerk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountConfig {
    /// Kennung (Kleinbuchstaben, Ziffern, `-`, `_`).
    pub id: String,
    /// Einhängepunkt (absolut); wird beim Start angelegt.
    pub mountpoint: PathBuf,
    /// Ordner in „Meine Dateien“; leer für „Meine Dateien“ selbst.
    pub remote: RelativePath,
    /// Voreinstellung „Immer lokal halten“, wo keine Regel greift.
    pub keep_local: bool,
}

/// Einstellungen des Zwischenspeichers (für alle Laufwerke).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheSettings {
    /// Wann freigegeben wird.
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
                "Ungültige Laufwerks-Kennung {:?} (erlaubt: a-z, 0-9, -, _)",
                raw.id
            )));
        }
        if !raw.mountpoint.is_absolute() {
            return Err(ConfigError::Invalid(format!(
                "Laufwerk {}: Einhängepunkt muss ein absoluter Pfad sein",
                raw.id
            )));
        }
        let remote = parse_remote(&raw.remote).map_err(|message| {
            ConfigError::Invalid(format!("Laufwerk {}: entfernter Ordner: {message}", raw.id))
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

/// Kennungen der Laufwerke müssen eindeutig sein.
pub(crate) fn ensure_unique_mount_ids(mounts: &[MountConfig]) -> Result<(), ConfigError> {
    for (index, mount) in mounts.iter().enumerate() {
        if mounts[..index].iter().any(|other| other.id == mount.id) {
            return Err(ConfigError::Invalid(format!(
                "Laufwerks-Kennung {} doppelt",
                mount.id
            )));
        }
    }
    Ok(())
}
