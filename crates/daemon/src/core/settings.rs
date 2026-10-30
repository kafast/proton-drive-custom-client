//! Reading and changing the editable settings at runtime (ADR 0021).
//!
//! A change is first saved in the configuration file and only then applied,
//! so the daemon never runs with settings that a restart would lose.

use std::sync::Arc;

use protonsync_api::{
    ApiError, ErrorCode, PairSettings, PairSettingsChange, SettingsChange, SettingsInfo,
};
use protonsync_application::ports::{Account, Clock, LocalStorage};
use protonsync_domain::sync_pair::SyncDirection;
use protonsync_domain::trash::RetentionPeriod;

use super::{Core, DaemonState, PairRuntime, Settings};
use crate::config::PairConfig;
use crate::convert;

/// Where changed settings are saved (the configuration file; a fake in tests).
pub trait SettingsFile: Send + Sync {
    /// Path shown to the user.
    fn path(&self) -> String;
    /// Whether changes can be saved (checked without changing anything).
    fn writable(&self) -> bool;
    /// Writes the change into the file.
    ///
    /// # Errors
    /// A message if the file cannot be read, changed or written, or if the
    /// changed configuration would be invalid.
    fn save(&self, change: &SettingsChange) -> Result<(), String>;
}

/// The options of a sync pair that can change at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PairOptions {
    pub(super) direction: SyncDirection,
    pub(super) protect_remote: bool,
    pub(super) archive_versions: bool,
}

impl From<&PairConfig> for PairOptions {
    fn from(config: &PairConfig) -> Self {
        Self {
            direction: config.direction,
            protect_remote: config.protect_remote,
            archive_versions: config.archive_versions,
        }
    }
}

impl PairOptions {
    /// The options after `change`.
    fn changed(self, change: &PairSettingsChange) -> Self {
        Self {
            direction: change
                .direction
                .map_or(self.direction, convert::sync_direction),
            protect_remote: change.protect_remote.unwrap_or(self.protect_remote),
            archive_versions: change.archive_versions.unwrap_or(self.archive_versions),
        }
    }
}

impl<R, L> PairRuntime<R, L> {
    /// The options that apply right now.
    pub(super) fn options(&self) -> PairOptions {
        // A poisoned lock only means that another thread panicked while
        // holding it; the plain `Copy` value inside is still intact.
        *self
            .options
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn set_options(&self, options: PairOptions) {
        *self
            .options
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = options;
    }
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(ErrorCode::InvalidRequest, message)
}

impl<A, L, S, C> Core<A, L, S, C>
where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: DaemonState,
    C: Clock + 'static,
{
    /// Lets the front ends change settings; they are saved in `file`.
    #[must_use]
    pub fn with_settings_file(mut self, file: Arc<dyn SettingsFile>) -> Self {
        self.settings_file = Some(file);
        self
    }

    /// The general settings that apply right now.
    pub(super) fn settings(&self) -> Settings {
        *self
            .settings
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn settings_info(&self) -> SettingsInfo {
        let settings = self.settings();
        SettingsInfo {
            config_path: self
                .settings_file
                .as_ref()
                .map(|file| file.path())
                .unwrap_or_default(),
            editable: self
                .settings_file
                .as_ref()
                .is_some_and(|file| file.writable()),
            dry_run: settings.dry_run,
            trash_retention_days: settings.retention.in_days(),
            pairs: self
                .pairs
                .iter()
                .map(|pair| {
                    let options = pair.options();
                    PairSettings {
                        id: pair.config.id.clone(),
                        direction: convert::direction(options.direction),
                        protect_remote: options.protect_remote,
                        archive_versions: options.archive_versions,
                    }
                })
                .collect(),
        }
    }

    /// Checks, saves and applies a change.
    pub(super) async fn change_settings(
        &self,
        change: SettingsChange,
    ) -> Result<SettingsInfo, ApiError> {
        if change.is_empty() {
            return Err(invalid("nothing to change"));
        }
        let retention = change
            .trash_retention_days
            .map(RetentionPeriod::days)
            .transpose()
            .map_err(|e| invalid(e.to_string()))?;
        let pair_changes = change
            .pairs
            .iter()
            .map(|pair| Ok((self.find_pair(&pair.id)?, pair)))
            .collect::<Result<Vec<_>, ApiError>>()?;
        let file = self.settings_file.as_ref().ok_or_else(|| {
            ApiError::new(
                ErrorCode::Unavailable,
                "settings cannot be changed: no configuration file",
            )
        })?;
        // No sync runs while the settings change (see `sync_pair`).
        let _guard = self.sync_lock.lock().await;
        file.save(&change).map_err(|message| {
            ApiError::new(
                ErrorCode::Unavailable,
                format!("cannot save the settings: {message}"),
            )
        })?;
        self.apply_general(change.dry_run, retention);
        for (index, pair_change) in pair_changes {
            self.apply_pair(index, pair_change).await;
        }
        tracing::info!(?change, "settings changed");
        Ok(self.settings_info())
    }

    fn apply_general(&self, dry_run: Option<bool>, retention: Option<RetentionPeriod>) {
        let mut settings = self
            .settings
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(dry_run) = dry_run {
            settings.dry_run = dry_run;
        }
        if let Some(retention) = retention {
            settings.retention = retention;
        }
    }

    /// New options for a pair. Its connection carries the options, so it is
    /// rebuilt right away (with a fresh remote tree).
    async fn apply_pair(&self, index: usize, change: &PairSettingsChange) {
        let Some(pair) = self.pairs.get(index) else {
            return;
        };
        let options = pair.options().changed(change);
        if options == pair.options() {
            return;
        }
        pair.set_options(options);
        *pair.connection.write().await = None;
        if self.account.username().await.is_none() {
            return;
        }
        match self.connect(pair).await {
            Ok(connection) => *pair.connection.write().await = Some(Arc::new(connection)),
            // The next sync tries again (`ensure_connection`).
            Err(error) => {
                tracing::warn!(pair = %pair.config.id, %error, "sync pair not reconnected");
            }
        }
    }
}
