//! Upload-Jobs im Daemon-Kern: ausführen, Verlauf führen, Ereignisse melden.

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

/// Ein Upload-Job beim Zusammenbau: Konfiguration und Quellordner.
pub struct JobSetup<L> {
    /// Der Job.
    pub job: UploadJob,
    /// Lokaler Speicher mit dem Quellordner als Wurzel.
    pub source: L,
}

/// Was ein automatischer Lauf ergeben hat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobAttempt {
    /// Nicht gelaufen (nicht angemeldet oder nichts Neues im Quellordner).
    NotRun,
    /// Gelaufen; `waiting`, wenn Dateien wegen der Ruhezeit noch ausstehen.
    Done {
        /// Dateien warten noch.
        waiting: bool,
    },
    /// Gescheitert.
    Failed {
        /// Fehlermeldung.
        message: String,
    },
}

/// Stand der passenden Dateien im Quellordner (Name, Größe, Änderungszeit),
/// um „neue Datei“ zu erkennen, ohne Proton zu fragen.
type SourceSignature = Vec<(String, u64, i64)>;

pub(super) struct JobRuntime<R, L> {
    pub(super) job: UploadJob,
    /// Unter dieser Kennung stehen die Papierkorbeinträge des Jobs.
    pub(super) owner: SyncPairId,
    pub(super) source: L,
    /// Zielordner in Proton; wird beim ersten Lauf geöffnet.
    pub(super) remote: Mutex<Option<Arc<R>>>,
    running: AtomicBool,
    /// Stand des Quellordners beim letzten abgeschlossenen Lauf.
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
    ApiError::new(ErrorCode::NotFound, format!("Unbekannter Job {id}"))
}

impl<A, L, S, C> Core<A, L, S, C>
where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: DaemonState,
    C: Clock + 'static,
{
    /// Anzahl der Upload-Jobs (Indizes `0..job_count()`).
    #[must_use]
    pub fn job_count(&self) -> usize {
        self.jobs.len()
    }

    /// Konfiguration eines Jobs.
    #[must_use]
    pub fn job(&self, index: usize) -> Option<&UploadJob> {
        self.jobs.get(index).map(|job| &job.job)
    }

    /// Signal für Änderungen im Quellordner eines Jobs (vom Watcher).
    #[must_use]
    pub fn job_trigger(&self, index: usize) -> Option<&Notify> {
        self.jobs.get(index).map(|job| &job.trigger)
    }

    /// Aktuelle Uhrzeit des Kerns (für den Zeitplan).
    #[must_use]
    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }

    /// Nächster geplanter Lauf nach jetzt; `None` bei „bei neuer Datei“.
    #[must_use]
    pub fn next_job_run(&self, index: usize) -> Option<Timestamp> {
        match &self.jobs.get(index)?.job.trigger {
            JobTrigger::Schedule(schedule) => self.zone.next_run(schedule, self.clock.now()),
            JobTrigger::OnNewFile => None,
        }
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

    /// Öffnet (einmalig) den Zielordner eines Jobs in Proton.
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

    /// Führt einen Job aus (mit `dry_run` nur planen), schreibt den Verlauf
    /// und meldet Beginn und Ende als Ereignis.
    ///
    /// # Errors
    /// [`ApiError`], wenn der Job unbekannt ist oder nicht angemeldet. Fehler
    /// des Laufs selbst stehen im Ergebnis.
    pub async fn run_job(
        &self,
        index: usize,
        dry_run: bool,
        attempt: u32,
    ) -> Result<(JobRunInfo, JobDetails), ApiError> {
        let remote = self.job_remote(index).await?;
        let job = self.jobs.get(index).ok_or_else(|| unknown_job(""))?;
        // Immer nur ein Job zur Zeit; Abgleiche laufen unabhängig davon.
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
            tracing::warn!(job = job.job.id.as_str(), %error, "Verlauf nicht gespeichert");
        }
        let info = convert::job_run(&job.job.id, &run);
        job.running.store(false, Ordering::Relaxed);
        self.publish(Event::JobFinished(info.clone()));
        Ok((info, details))
    }

    /// Automatischer Lauf aus dem Zeitplan; beachtet die Probelauf-Einstellung.
    ///
    /// Mit `only_if_changed` läuft der Job nur, wenn sich passende Dateien im
    /// Quellordner seit dem letzten abgeschlossenen Lauf geändert haben. Nach
    /// dem letzten erfolglosen Versuch wird `JobGaveUp` gemeldet.
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
        // Nicht angemeldet: nichts zu tun, bis jemand sich anmeldet.
        let Ok((run, details)) = self.run_job(index, self.settings.dry_run, attempt).await else {
            return JobAttempt::NotRun;
        };
        let last_attempt = attempt >= job.job.max_attempts;
        let attempt_result = match run.outcome {
            protonsync_api::JobOutcomeInfo::Failed { message, .. } => {
                if last_attempt {
                    tracing::error!(job = job.job.id.as_str(), %message, attempt, "Job aufgegeben");
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
        // Nach Erfolg oder Aufgeben gilt der Stand als erledigt; erst eine neue
        // Datei löst wieder einen Lauf aus.
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
            tracing::warn!(job = job.job.id.as_str(), %error, "Quellordner nicht lesbar");
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
