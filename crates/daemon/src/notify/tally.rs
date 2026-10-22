//! Counters for the weekly report, collected from the daemon events.
//!
//! They live in memory only: after a restart the report covers the time since
//! the start (it says so in its period). Everything that is stored anyway
//! (conflicts, trash, jobs) comes fresh from the daemon instead.

use std::collections::BTreeMap;

use protonsync_api::{
    Event, JobOutcomeInfo, PairSyncResult, PhotoOutcomeInfo, PhotoRunInfo, SyncOutcomeInfo,
};
use protonsync_domain::time::Timestamp;

/// Figures of one sync pair.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PairTally {
    /// Uploaded.
    pub uploaded: usize,
    /// Downloaded.
    pub downloaded: usize,
    /// Moved to the Proton trash.
    pub trashed_remote: usize,
    /// Moved to the local trash.
    pub trashed_local: usize,
    /// New conflicts.
    pub conflicts: usize,
    /// Runs that failed as a whole.
    pub failed_runs: usize,
    /// Single files that failed.
    pub step_failures: usize,
}

/// Figures of one upload job.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobTally {
    /// Successful runs.
    pub succeeded: usize,
    /// Failed attempts.
    pub failed: usize,
    /// Uploaded files.
    pub uploaded: u64,
}

/// Figures of the photo sync.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhotoTally {
    /// Downloaded.
    pub downloaded: usize,
    /// Uploaded.
    pub uploaded: usize,
    /// Failed runs.
    pub failed_runs: usize,
}

/// All counters since `since`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeekTally {
    /// Start of the period (last report or start of the daemon).
    pub since: Timestamp,
    /// Per sync pair.
    pub pairs: BTreeMap<String, PairTally>,
    /// Per upload job.
    pub jobs: BTreeMap<String, JobTally>,
    /// Photo sync, if it ran.
    pub photos: Option<PhotoTally>,
}

impl WeekTally {
    /// Empty counters starting at `since`.
    #[must_use]
    pub fn new(since: Timestamp) -> Self {
        Self {
            since,
            pairs: BTreeMap::new(),
            jobs: BTreeMap::new(),
            photos: None,
        }
    }

    /// Counts what `event` reports.
    pub fn record(&mut self, event: &Event) {
        match event {
            Event::SyncFinished(result) => self.record_sync(result),
            Event::JobFinished(run) => {
                let job = self.jobs.entry(run.job.clone()).or_default();
                match &run.outcome {
                    JobOutcomeInfo::Succeeded { uploaded, .. } => {
                        job.succeeded += 1;
                        job.uploaded += u64::from(*uploaded);
                    }
                    JobOutcomeInfo::Failed { .. } => job.failed += 1,
                    JobOutcomeInfo::Planned { .. } => {}
                }
            }
            Event::PhotoSyncFinished(run) => self.record_photos(run),
            _ => {}
        }
    }

    fn record_sync(&mut self, result: &PairSyncResult) {
        let pair = self.pairs.entry(result.pair.clone()).or_default();
        match &result.outcome {
            SyncOutcomeInfo::Applied(summary) => {
                pair.uploaded += summary.uploaded;
                pair.downloaded += summary.downloaded;
                pair.trashed_remote += summary.trashed_remote;
                pair.trashed_local += summary.trashed_local;
                pair.conflicts += summary.conflicts;
                pair.step_failures += summary.failures.len();
            }
            SyncOutcomeInfo::Failed { .. } => pair.failed_runs += 1,
            SyncOutcomeInfo::Planned { .. } => {}
        }
    }

    fn record_photos(&mut self, run: &PhotoRunInfo) {
        let photos = self.photos.get_or_insert_with(PhotoTally::default);
        match &run.outcome {
            PhotoOutcomeInfo::Applied(counts) => {
                photos.downloaded += counts.downloaded;
                photos.uploaded += counts.uploaded;
            }
            PhotoOutcomeInfo::Failed { .. } => photos.failed_runs += 1,
            PhotoOutcomeInfo::Planned(_) => {}
        }
    }
}
