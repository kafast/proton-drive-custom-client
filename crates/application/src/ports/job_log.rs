//! Port for the history of the upload jobs.

use protonsync_domain::job::{JobId, JobRun};

use super::StateError;

/// This many runs per job stay in the history; older ones are discarded.
pub const JOB_HISTORY_LIMIT: usize = 50;

/// History of the upload jobs (separate from [`super::StateRepo`], because only the
/// daemon needs it, not the sync engine).
pub trait JobLog: Send + Sync {
    /// Appends a run; beyond [`JOB_HISTORY_LIMIT`] the oldest are dropped.
    ///
    /// # Errors
    /// [`StateError`] if the store is unreachable.
    fn record_job_run(&self, job: &JobId, run: &JobRun) -> Result<(), StateError>;

    /// The last `limit` runs, newest first.
    ///
    /// # Errors
    /// [`StateError`] if the store is unreachable or damaged.
    fn job_runs(&self, job: &JobId, limit: usize) -> Result<Vec<JobRun>, StateError>;
}

/// A shared history (`Arc`) is itself a history.
impl<T: JobLog + ?Sized> JobLog for std::sync::Arc<T> {
    fn record_job_run(&self, job: &JobId, run: &JobRun) -> Result<(), StateError> {
        (**self).record_job_run(job, run)
    }

    fn job_runs(&self, job: &JobId, limit: usize) -> Result<Vec<JobRun>, StateError> {
        (**self).job_runs(job, limit)
    }
}
