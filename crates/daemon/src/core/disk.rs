//! Regular check of the free disk space: publishes [`Event::DiskSpaceChanged`]
//! when a disk becomes almost full, full, or has space again.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use protonsync_api::{DiskLevelInfo, DiskSpaceInfo, Event};
use protonsync_application::ports::{Account, Clock, DiskSpaceProbe, LocalStorage};
use protonsync_application::use_cases::{DiskChange, DiskMonitor, WatchedFolder};
use protonsync_domain::disk::{DiskLevel, DiskThresholds};

use super::{Core, DaemonState};

/// What the check needs: how to measure, what to watch, how often.
pub struct DiskWatchSetup {
    /// Measures the free space.
    pub probe: Arc<dyn DiskSpaceProbe>,
    /// When a disk counts as almost full or full.
    pub thresholds: DiskThresholds,
    /// The folders protonsync writes to.
    pub folders: Vec<WatchedFolder>,
    /// Interval of the check.
    pub every: Duration,
}

pub(super) struct DiskWatch {
    probe: Arc<dyn DiskSpaceProbe>,
    folders: Vec<WatchedFolder>,
    every: Duration,
    // A plain mutex: the check never waits on anything while holding it.
    monitor: Mutex<DiskMonitor>,
}

impl From<DiskWatchSetup> for DiskWatch {
    fn from(setup: DiskWatchSetup) -> Self {
        Self {
            probe: setup.probe,
            folders: setup.folders,
            every: setup.every,
            monitor: Mutex::new(DiskMonitor::new(setup.thresholds)),
        }
    }
}

fn level_info(level: DiskLevel) -> DiskLevelInfo {
    match level {
        DiskLevel::Ok => DiskLevelInfo::Ok,
        DiskLevel::Low => DiskLevelInfo::Low,
        DiskLevel::Full => DiskLevelInfo::Full,
    }
}

fn info(change: DiskChange) -> DiskSpaceInfo {
    DiskSpaceInfo {
        level: level_info(change.level),
        folders: change.folders,
        path: change.path.display().to_string(),
        available_bytes: change.space.available_bytes,
        total_bytes: change.space.total_bytes,
    }
}

impl<A, L, S, C> Core<A, L, S, C>
where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: DaemonState,
    C: Clock + 'static,
{
    /// Switches on the regular check of the free disk space.
    #[must_use]
    pub fn with_disk_check(mut self, setup: DiskWatchSetup) -> Self {
        self.disk = Some(DiskWatch::from(setup));
        self
    }

    /// Interval of the disk check; `None` if it is switched off.
    #[must_use]
    pub fn disk_check_interval(&self) -> Option<Duration> {
        self.disk.as_ref().map(|disk| disk.every)
    }

    /// Checks the free space once and publishes every change of level.
    pub fn check_disk_space(&self) {
        let Some(disk) = &self.disk else {
            return;
        };
        let changes = disk
            .monitor
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .check(disk.probe.as_ref(), &disk.folders, |folder, error| {
                tracing::debug!(folder = %folder.name, %error, "free space not checked");
            });
        for change in changes {
            let info = info(change);
            match info.level {
                DiskLevelInfo::Ok => tracing::info!(?info, "enough disk space again"),
                DiskLevelInfo::Low | DiskLevelInfo::Full => {
                    tracing::warn!(?info, "disk running full");
                }
            }
            self.publish(Event::DiskSpaceChanged(info));
        }
    }
}
