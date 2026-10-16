//! Port für den Verlauf der Upload-Jobs.

use protonsync_domain::job::{JobId, JobRun};

use super::StateError;

/// So viele Läufe je Job bleiben im Verlauf; ältere werden verworfen.
pub const JOB_HISTORY_LIMIT: usize = 50;

/// Verlauf der Upload-Jobs (getrennt von [`super::StateRepo`], weil nur der
/// Daemon ihn braucht, nicht die Sync-Engine).
pub trait JobLog: Send + Sync {
    /// Hängt einen Lauf an; über [`JOB_HISTORY_LIMIT`] hinaus fallen die ältesten weg.
    ///
    /// # Errors
    /// [`StateError`], wenn der Speicher nicht erreichbar ist.
    fn record_job_run(&self, job: &JobId, run: &JobRun) -> Result<(), StateError>;

    /// Die letzten `limit` Läufe, neueste zuerst.
    ///
    /// # Errors
    /// [`StateError`], wenn der Speicher nicht erreichbar oder beschädigt ist.
    fn job_runs(&self, job: &JobId, limit: usize) -> Result<Vec<JobRun>, StateError>;
}

/// Ein geteilter Verlauf (`Arc`) ist selbst ein Verlauf.
impl<T: JobLog + ?Sized> JobLog for std::sync::Arc<T> {
    fn record_job_run(&self, job: &JobId, run: &JobRun) -> Result<(), StateError> {
        (**self).record_job_run(job, run)
    }

    fn job_runs(&self, job: &JobId, limit: usize) -> Result<Vec<JobRun>, StateError> {
        (**self).job_runs(job, limit)
    }
}
