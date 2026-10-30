//! API data types for the settings that the front ends may change (ADR 0021).
//!
//! Only a few, frequently changed settings are editable this way; everything
//! else stays in the configuration file. The daemon writes a change into the
//! file (comments are kept) and applies it right away, without a restart.

use serde::{Deserialize, Serialize};

use crate::Direction;

/// The editable settings as they currently apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsInfo {
    /// Path of the configuration file the daemon writes changes to.
    pub config_path: String,
    /// Whether changes can be saved (`false`: file not writable or not set up).
    pub editable: bool,
    /// Only plan automatic syncs and trash purges.
    pub dry_run: bool,
    /// Days after which trash entries expire.
    pub trash_retention_days: u32,
    /// Settings of every sync pair, in the order of the configuration.
    pub pairs: Vec<PairSettings>,
}

/// Editable settings of one sync pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairSettings {
    /// Sync pair.
    pub id: String,
    /// Direction.
    pub direction: Direction,
    /// Local deletions and replacements are not passed on to Proton.
    pub protect_remote: bool,
    /// Deleted and replaced versions are moved into `_archiv` on both sides.
    pub archive_versions: bool,
}

/// A change of settings; fields without a value stay as they are.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsChange {
    /// New value for `dry_run`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    /// New value for `trash_retention_days`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trash_retention_days: Option<u32>,
    /// Changes of sync pairs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pairs: Vec<PairSettingsChange>,
}

/// A change of one sync pair; fields without a value stay as they are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairSettingsChange {
    /// Sync pair.
    pub id: String,
    /// New direction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<Direction>,
    /// New value for `protect_remote`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protect_remote: Option<bool>,
    /// New value for `archive_versions`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_versions: Option<bool>,
}

impl PairSettingsChange {
    /// A change of the pair `id` that changes nothing yet.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            direction: None,
            protect_remote: None,
            archive_versions: None,
        }
    }
}

impl SettingsChange {
    /// Whether the change sets nothing at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.dry_run.is_none()
            && self.trash_retention_days.is_none()
            && self.pairs.iter().all(|pair| {
                pair.direction.is_none()
                    && pair.protect_remote.is_none()
                    && pair.archive_versions.is_none()
            })
    }
}
