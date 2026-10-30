//! Upload jobs in the daemon core: run them, keep the history, report events.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use protonsync_api::{ApiError, ErrorCode, Event, JobDetails, JobInfo, JobRunInfo};
use protonsync_application::ports::{
    Account, Clock, JOB_HISTORY_LIMIT, JobLog, LocalKind, LocalStorage, RemoteError,
};
use protonsync_application::use_cases::{UploadJobRunner, UseCaseError};
use protonsync_domain::job::{JobRun, JobTrigger, UploadJob};
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::time::Timestamp;
use tokio::sync::{Mutex, Notify};

use super::{Core, DaemonState, not_logged_in};
use crate::convert;

/// An upload job during wiring: configuration and source folder.
pub struct JobSetup<L> {
    /// The job.
    pub job: UploadJob,
    /// Local storage with the source folder as its root.
    pub source: L,
}

/// What an automatic run resulted in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobAttempt {
    /// Did not run (not logged in or nothing new in the source folder).
    NotRun,
    /// Ran; `waiting` if files are still pending because of the settle time.
    Done {
        /// Files are still waiting.
        waiting: bool,
    },
    /// Failed.
    Failed {
        /// Error message.
        message: String,
    },
}

/// State of the matching files in the source folder (name, size, modification
/// time), to detect a "new file" without asking Proton.
type SourceSignature = Vec<(String, u64, i64)>;

pub(super) struct JobRuntime<R, L> {
    pub(super) job: UploadJob,
    /// The job's trash entries are stored under this identifier.
    pub(super) owner: SyncPairId,
    pub(super) source: L,
    /// Target folder in Proton; opened on the first run.
    pub(super) remote: Mutex<Option<Arc<R>>>,
    running: AtomicBool,
    /// State of the source folder at the last completed run.
    signature: std::sync::Mutex<Option<SourceSignature>>,
    pub(super) trigger: Notify,
}

impl<R, L> JobRuntime<R, L> {
    pub(super) fn new(setup: JobSetup<L>) -> Self {
        Self {
            owner: SyncPairId::new(format!(
                "{}{}",
                protonsync_api::JOB_TRASH_PREFIX,
                setup.job.id.as_str()
            )),
            job: setup.job,
            source: setup.source,
            remote: Mutex::new(None),
            running: AtomicBool::new(false),
            signature: std::sync::Mutex::new(None),
            trigger: Notify::new(),
        }
    }
}

fn unknown_job(id: &str) -> ApiError {
    ApiError::new(ErrorCode::NotFound, format!("unknown job {id}"))
}

impl<A, L, S, C> Core<A, L, S, C>
where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: DaemonState,
    C: Clock + 'static,
{
    /// Number of upload jobs (indices `0..job_count()`).
    #[must_use]
    pub fn job_count(&self) -> usize {
        self.jobs.len()
    }

    /// Configuration of a job.
    #[must_use]
    pub fn job(&self, index: usize) -> Option<&UploadJob> {
        self.jobs.get(index).map(|job| &job.job)
    }

    /// Signal for changes in a job's source folder (from the watcher).
    #[must_use]
    pub fn job_trigger(&self, index: usize) -> Option<&Notify> {
        self.jobs.get(index).map(|job| &job.trigger)
    }

    /// Current time of the core (for the schedule).
    #[must_use]
    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }

    /// Next scheduled run after now; `None` for "on new file".
    #[must_use]
    pub fn next_job_run(&self, index: usize) -> Option<Timestamp> {
        match &self.jobs.get(index)?.job.trigger {
            JobTrigger::Schedule(schedule) => self.zone.next_run(schedule, self.clock.now()),
            JobTrigger::OnNewFile => None,
        }
    }

    /// Whether a scheduled time has passed since the last run, e.g. because
    /// the machine was off. Without a previous run there is nothing to catch up.
    pub fn missed_job_run(&self, index: usize) -> bool {
        let Some(runtime) = self.jobs.get(index) else {
            return false;
        };
        let JobTrigger::Schedule(schedule) = &runtime.job.trigger else {
            return false;
        };
        let last_start = match self.state.job_runs(&runtime.job.id, 1) {
            Ok(runs) => runs.first().map(|run| run.started_at),
            Err(error) => {
                tracing::warn!(job = runtime.job.id.as_str(), %error, "cannot read the last run");
                None
            }
        };
        last_start
            .and_then(|last| self.zone.next_run(schedule, last))
            .is_some_and(|due| due <= self.clock.now())
    }

    pub(super) fn find_job(&self, id: &str) -> Result<usize, ApiError> {
        self.jobs
            .iter()
            .position(|job| job.job.id.as_str() == id)
            .ok_or_else(|| unknown_job(id))
    }

    pub(super) fn job_infos(&self) -> Result<Vec<JobInfo>, ApiError> {
        self.jobs
            .iter()
            .enumerate()
            .map(|(index, job)| {
                let last_run = self
                    .state
                    .job_runs(&job.job.id, 1)
                    .map_err(|e| convert::state_api_error(&e))?
                    .first()
                    .map(|run| convert::job_run(&job.job.id, run));
                Ok(convert::job_info(
                    &job.job,
                    job.running.load(Ordering::Relaxed),
                    last_run,
                    self.next_job_run(index).map(Timestamp::unix_seconds),
                ))
            })
            .collect()
    }

    pub(super) fn job_history(&self, id: &str, limit: u32) -> Result<Vec<JobRunInfo>, ApiError> {
        let job = &self.jobs[self.find_job(id)?].job;
        let limit = usize::try_from(limit)
            .unwrap_or(JOB_HISTORY_LIMIT)
            .min(JOB_HISTORY_LIMIT);
        Ok(self
            .state
            .job_runs(&job.id, limit)
            .map_err(|e| convert::state_api_error(&e))?
            .iter()
            .map(|run| convert::job_run(&job.id, run))
            .collect())
    }

    /// Opens (once) the target folder of a job in Proton.
    pub(super) async fn job_remote(&self, index: usize) -> Result<Arc<A::Remote>, ApiError> {
        let job = self.jobs.get(index).ok_or_else(|| unknown_job(""))?;
        if self.account.username().await.is_none() {
            return Err(not_logged_in());
        }
        let mut remote = job.remote.lock().await;
        if let Some(remote) = remote.as_ref() {
            return Ok(Arc::clone(remote));
        }
        let opened = Arc::new(
            self.account
                .open_remote(&job.job.target)
                .await
                .map_err(|e| convert::account_error(&e))?,
        );
        *remote = Some(Arc::clone(&opened));
        Ok(opened)
    }

    pub(super) async fn forget_job_remotes(&self) {
        for job in &self.jobs {
            *job.remote.lock().await = None;
        }
    }

    /// Runs a job (with `dry_run` only plans), writes the history and reports
    /// start and end as events.
    ///
    /// # Errors
    /// [`ApiError`] if the job is unknown or not logged in. Errors of the run
    /// itself are in the result.
    pub async fn run_job(
        &self,
        index: usize,
        dry_run: bool,
        attempt: u32,
    ) -> Result<(JobRunInfo, JobDetails), ApiError> {
        let remote = self.job_remote(index).await?;
        let job = self.jobs.get(index).ok_or_else(|| unknown_job(""))?;
        // Only one job at a time; syncs run independently of this.
        let _guard = self.job_lock.lock().await;
        job.running.store(true, Ordering::Relaxed);
        self.publish(Event::JobStarted {
            job: job.job.id.as_str().to_owned(),
        });
        let started_at = self.clock.now();
        let runner = UploadJobRunner {
            remote: remote.as_ref(),
            source: &job.source,
            state: &self.state,
            clock: &self.clock,
            job: &job.job,
            trash_owner: &job.owner,
        };
        let result = runner.run(dry_run).await;
        if matches!(result, Err(UseCaseError::Remote(RemoteError::Unauthorized))) {
            self.publish(Event::LoginRequired {
                message: RemoteError::Unauthorized.to_string(),
            });
        }
        let (outcome, details) = convert::job_result(&result, attempt);
        let run = JobRun {
            started_at,
            finished_at: self.clock.now(),
            outcome,
        };
        if let Err(error) = self.state.record_job_run(&job.job.id, &run) {
            tracing::warn!(job = job.job.id.as_str(), %error, "history not saved");
        }
        let info = convert::job_run(&job.job.id, &run);
        job.running.store(false, Ordering::Relaxed);
        self.publish(Event::JobFinished(info.clone()));
        Ok((info, details))
    }

    /// Automatic run from the schedule; respects the dry-run setting.
    ///
    /// With `only_if_changed` the job only runs if matching files in the source
    /// folder have changed since the last completed run. After the last failed
    /// attempt, `JobGaveUp` is reported.
    pub async fn scheduled_job(
        &self,
        index: usize,
        attempt: u32,
        only_if_changed: bool,
    ) -> JobAttempt {
        let Some(job) = self.jobs.get(index) else {
            return JobAttempt::NotRun;
        };
        let signature = source_signature(job);
        if only_if_changed
            && signature.is_some()
            && job.signature.lock().ok().and_then(|stored| stored.clone()) == signature
        {
            return JobAttempt::NotRun;
        }
        // Not logged in: nothing to do until someone logs in.
        let Ok((run, details)) = self.run_job(index, self.settings().dry_run, attempt).await else {
            return JobAttempt::NotRun;
        };
        let last_attempt = attempt >= job.job.max_attempts;
        let attempt_result = match run.outcome {
            protonsync_api::JobOutcomeInfo::Failed { message, .. } => {
                if last_attempt {
                    tracing::error!(job = job.job.id.as_str(), %message, attempt, "job given up");
                    self.publish(Event::JobGaveUp {
                        job: job.job.id.as_str().to_owned(),
                        message: message.clone(),
                        attempts: attempt,
                    });
                }
                JobAttempt::Failed { message }
            }
            _ => JobAttempt::Done {
                waiting: !details.waiting.is_empty(),
            },
        };
        // After success or giving up, the state counts as done; only a new file
        // triggers a run again.
        if (last_attempt || !matches!(attempt_result, JobAttempt::Failed { .. }))
            && let Ok(mut stored) = job.signature.lock()
        {
            *stored = signature;
        }
        attempt_result
    }
}

fn source_signature<R, L: LocalStorage>(job: &JobRuntime<R, L>) -> Option<SourceSignature> {
    let entries = match job.source.scan() {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(job = job.job.id.as_str(), %error, "cannot read the source folder");
            return None;
        }
    };
    let mut signature: SourceSignature = entries
        .into_iter()
        .filter(|entry| entry.path.parent().is_some_and(|p| p.is_root()))
        .filter_map(|entry| {
            let LocalKind::File(fingerprint) = entry.kind else {
                return None;
            };
            let name = entry.path.file_name()?.to_string();
            job.job
                .pattern
                .matches(&name)
                .then(|| (name, fingerprint.size, fingerprint.modified.unix_seconds()))
        })
        .collect();
    signature.sort();
    Some(signature)
}
