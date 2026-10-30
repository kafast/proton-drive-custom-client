//! Conversion between domain/application types and the API types.

use protonsync_api::{
    ApiError, ConflictInfo, ConflictKindInfo, Direction, ErrorCode, FailureInfo, FreeResult,
    JobDetails, JobInfo, JobOutcomeInfo, JobRunInfo, JobVerification, LoginStep, PinChoice,
    PlannedStepInfo, ResolutionChoice, SkippedInfo, StepAction, SyncOutcomeInfo, SyncSummary,
    TrashInfo, TrashSide,
};
use protonsync_application::drive::{DriveError, EvictionReport};
use protonsync_application::ports::{
    AccountError, ConflictId, LocalError, LoginProgress, RemoteError, StateError, TrashId,
};
use protonsync_application::sync::{SkippedPath, SyncError, SyncPlan, SyncReport};
use protonsync_application::use_cases::{JobResult, UseCaseError};
use protonsync_domain::conflict::{Conflict, ConflictKind, Resolution, SyncAction};
use protonsync_domain::job::{JobId, JobOutcome, JobRun, JobTrigger, UploadJob, Verification};
use protonsync_domain::node::NodeName;
use protonsync_domain::pin::PinState;
use protonsync_domain::sync_pair::SyncDirection;
use protonsync_domain::trash::{RetentionPeriod, TrashEntry, TrashOrigin};

use crate::job_config::ON_NEW_FILE;

mod photos;

pub(crate) use photos::*;

pub(crate) fn direction(direction: SyncDirection) -> Direction {
    match direction {
        SyncDirection::TwoWay => Direction::TwoWay,
        SyncDirection::DownloadOnly => Direction::DownloadOnly,
        SyncDirection::UploadOnly => Direction::UploadOnly,
        SyncDirection::BackupOnly => Direction::BackupOnly,
    }
}

/// The reverse of [`direction`].
pub(crate) fn sync_direction(direction: Direction) -> SyncDirection {
    match direction {
        Direction::TwoWay => SyncDirection::TwoWay,
        Direction::DownloadOnly => SyncDirection::DownloadOnly,
        Direction::UploadOnly => SyncDirection::UploadOnly,
        Direction::BackupOnly => SyncDirection::BackupOnly,
    }
}

pub(crate) fn login_step(progress: LoginProgress) -> LoginStep {
    LoginStep {
        needs_second_factor: progress.needs_second_factor,
        needs_data_password: progress.needs_data_password,
    }
}

pub(crate) fn resolution(choice: ResolutionChoice) -> Resolution {
    match choice {
        ResolutionChoice::KeepLocal => Resolution::KeepLocal,
        ResolutionChoice::KeepRemote => Resolution::KeepRemote,
        ResolutionChoice::KeepBoth => Resolution::KeepBoth,
    }
}

fn step_action(action: SyncAction) -> Option<StepAction> {
    match action {
        SyncAction::Nothing => None,
        SyncAction::RecordConverged => Some(StepAction::RecordConverged),
        SyncAction::Upload => Some(StepAction::Upload),
        SyncAction::Download => Some(StepAction::Download),
        SyncAction::TrashRemote => Some(StepAction::TrashRemote),
        SyncAction::TrashLocal => Some(StepAction::TrashLocal),
        SyncAction::IgnoreLocalChange => Some(StepAction::IgnoreLocalChange),
        SyncAction::IgnoreRemoteChange => Some(StepAction::IgnoreRemoteChange),
        SyncAction::Conflict => Some(StepAction::Conflict),
        SyncAction::ArchiveAndDownload => Some(StepAction::ArchiveAndDownload),
        SyncAction::KeepLocalCopy => Some(StepAction::KeepLocalCopy),
        SyncAction::ArchiveLocal => Some(StepAction::ArchiveLocal),
        SyncAction::ArchiveRemote => Some(StepAction::ArchiveRemote),
        SyncAction::ArchiveRemoteAndUpload => Some(StepAction::ArchiveRemoteAndUpload),
    }
}

fn skipped(paths: &[SkippedPath]) -> Vec<SkippedInfo> {
    paths
        .iter()
        .map(|s| SkippedInfo {
            path: s.path.to_string(),
            reason: s.reason.clone(),
        })
        .collect()
}

pub(crate) fn planned(plan: &SyncPlan) -> SyncOutcomeInfo {
    SyncOutcomeInfo::Planned {
        steps: plan
            .steps
            .iter()
            .filter_map(|step| {
                step_action(step.action).map(|action| PlannedStepInfo {
                    path: step.path.to_string(),
                    action,
                })
            })
            .collect(),
        skipped: skipped(&plan.skipped),
    }
}

pub(crate) fn applied(report: &SyncReport) -> SyncOutcomeInfo {
    SyncOutcomeInfo::Applied(SyncSummary {
        uploaded: report.uploaded,
        downloaded: report.downloaded,
        trashed_remote: report.trashed_remote,
        trashed_local: report.trashed_local,
        archived: report.archived_local + report.archived_remote,
        conflicts: report.conflicts,
        ignored: report.ignored,
        skipped: skipped(&report.skipped),
        failures: report
            .failures
            .iter()
            .map(|f| FailureInfo {
                path: f.path.to_string(),
                message: f.message.clone(),
            })
            .collect(),
    })
}

pub(crate) fn conflict(pair: &str, id: ConflictId, conflict: &Conflict) -> ConflictInfo {
    ConflictInfo {
        pair: pair.to_owned(),
        id: id.0,
        path: conflict.relative_path.to_string(),
        kind: match conflict.kind {
            ConflictKind::BothModified => ConflictKindInfo::BothModified,
            ConflictKind::LocalModifiedRemoteDeleted => {
                ConflictKindInfo::LocalModifiedRemoteDeleted
            }
            ConflictKind::LocalDeletedRemoteModified => {
                ConflictKindInfo::LocalDeletedRemoteModified
            }
            ConflictKind::LocalModifiedRemoteProtected => {
                ConflictKindInfo::LocalModifiedRemoteProtected
            }
        },
        conflict_copy: conflict.conflict_copy.as_ref().map(ToString::to_string),
        detected_at: conflict.detected_at.unix_seconds(),
    }
}

pub(crate) fn trash(
    pair: &str,
    id: TrashId,
    entry: &TrashEntry,
    retention: RetentionPeriod,
) -> TrashInfo {
    TrashInfo {
        pair: pair.to_owned(),
        id: id.0,
        path: entry.original_path.to_string(),
        side: match entry.origin {
            TrashOrigin::Local { .. } => TrashSide::Local,
            TrashOrigin::Remote { .. } => TrashSide::Remote,
        },
        trashed_at: entry.trashed_at.unix_seconds(),
        expires_at: entry.expires_at(retention).unix_seconds(),
    }
}

fn remote_error(error: &RemoteError) -> ApiError {
    let code = match error {
        RemoteError::Unauthorized => ErrorCode::NotLoggedIn,
        RemoteError::NotFound(_) => ErrorCode::NotFound,
        RemoteError::Unavailable(_) | RemoteError::Transfer(_) => ErrorCode::Unavailable,
        _ => ErrorCode::Internal,
    };
    ApiError::new(code, error.to_string())
}

fn local_error(error: &LocalError) -> ApiError {
    let code = match error {
        LocalError::NotFound(_) => ErrorCode::NotFound,
        LocalError::Io(_) => ErrorCode::Unavailable,
        _ => ErrorCode::Internal,
    };
    ApiError::new(code, error.to_string())
}

fn state_error(error: &StateError) -> ApiError {
    ApiError::new(ErrorCode::Internal, error.to_string())
}

pub(crate) fn sync_error(error: &SyncError) -> ApiError {
    match error {
        SyncError::Remote(e) => remote_error(e),
        SyncError::Local(e) => local_error(e),
        SyncError::State(e) => state_error(e),
    }
}

pub(crate) const fn pin_choice(state: PinState) -> PinChoice {
    match state {
        PinState::Pinned => PinChoice::Pinned,
        PinState::Unpinned => PinChoice::Unpinned,
    }
}

pub(crate) const fn pin_state(choice: PinChoice) -> PinState {
    match choice {
        PinChoice::Pinned => PinState::Pinned,
        PinChoice::Unpinned => PinState::Unpinned,
    }
}

pub(crate) fn free_result(mount: &str, dry_run: bool, report: &EvictionReport) -> FreeResult {
    FreeResult {
        mount: mount.to_owned(),
        dry_run,
        files: report.files.iter().map(ToString::to_string).collect(),
        bytes: report.bytes,
        skipped: u64::try_from(report.skipped).unwrap_or(u64::MAX),
    }
}

pub(crate) fn drive_error(error: &DriveError) -> ApiError {
    match error {
        DriveError::Remote(e) => remote_error(e),
        DriveError::Local(e) => local_error(e),
        DriveError::State(e) => state_error(e),
        DriveError::NotFound => ApiError::new(ErrorCode::NotFound, error.to_string()),
        DriveError::AlreadyExists
        | DriveError::NotAFolder
        | DriveError::IsAFolder
        | DriveError::NotEmpty
        | DriveError::InvalidName(_) => ApiError::new(ErrorCode::InvalidRequest, error.to_string()),
        DriveError::Internal(_) => ApiError::new(ErrorCode::Internal, error.to_string()),
    }
}

pub(crate) fn use_case_error(error: &UseCaseError) -> ApiError {
    match error {
        UseCaseError::Remote(e) => remote_error(e),
        UseCaseError::Local(e) => local_error(e),
        UseCaseError::State(e) => state_error(e),
        UseCaseError::UnknownConflict(_) | UseCaseError::UnknownTrashEntry(_) => {
            ApiError::new(ErrorCode::NotFound, error.to_string())
        }
        UseCaseError::RemoteProtected(_) => {
            ApiError::new(ErrorCode::InvalidRequest, error.to_string())
        }
    }
}

pub(crate) fn account_error(error: &AccountError) -> ApiError {
    let code = match error {
        AccountError::NotLoggedIn => ErrorCode::NotLoggedIn,
        AccountError::Rejected(_) => ErrorCode::LoginRejected,
        AccountError::WrongStep(_) => ErrorCode::InvalidRequest,
        AccountError::Unavailable(_) => ErrorCode::Unavailable,
    };
    ApiError::new(code, error.to_string())
}

pub(crate) fn state_api_error(error: &StateError) -> ApiError {
    state_error(error)
}

/// Outcome of a job run for the history and the API; `attempt` counts on failures.
pub(crate) fn job_result(
    result: &Result<JobResult, UseCaseError>,
    attempt: u32,
) -> (JobOutcome, JobDetails) {
    let names = |names: &[NodeName]| names.iter().map(ToString::to_string).collect();
    match result {
        Ok(JobResult::Planned(plan)) => (
            JobOutcome::Planned {
                uploads: count(plan.uploads.len()),
                expired: count(plan.expired.len()),
            },
            JobDetails {
                uploads: plan.uploads.iter().map(|u| u.name.to_string()).collect(),
                waiting: names(&plan.waiting),
                expired: names(&plan.expired),
                failures: Vec::new(),
            },
        ),
        Ok(JobResult::Applied(report)) => {
            let details = JobDetails {
                uploads: names(&report.uploaded),
                waiting: names(&report.waiting),
                expired: names(&report.expired),
                failures: report
                    .failures
                    .iter()
                    .map(|f| FailureInfo {
                        path: f.name.to_string(),
                        message: f.message.clone(),
                    })
                    .collect(),
            };
            let outcome = match report.failures.first() {
                None => JobOutcome::Succeeded {
                    uploaded: count(report.uploaded.len()),
                    unchanged: count(report.unchanged),
                    expired: count(report.expired.len()),
                },
                Some(first) => JobOutcome::Failed {
                    message: format!(
                        "{} of {} files failed, e.g. {}: {}",
                        report.failures.len(),
                        report.failures.len() + report.uploaded.len(),
                        first.name,
                        first.message
                    ),
                    attempt,
                },
            };
            (outcome, details)
        }
        Err(error) => (
            JobOutcome::Failed {
                message: use_case_error(error).message,
                attempt,
            },
            JobDetails::default(),
        ),
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

pub(crate) fn job_run(job: &JobId, run: &JobRun) -> JobRunInfo {
    JobRunInfo {
        job: job.as_str().to_owned(),
        started_at: run.started_at.unix_seconds(),
        finished_at: run.finished_at.unix_seconds(),
        outcome: match &run.outcome {
            JobOutcome::Planned { uploads, expired } => JobOutcomeInfo::Planned {
                uploads: *uploads,
                expired: *expired,
            },
            JobOutcome::Succeeded {
                uploaded,
                unchanged,
                expired,
            } => JobOutcomeInfo::Succeeded {
                uploaded: *uploaded,
                unchanged: *unchanged,
                expired: *expired,
            },
            JobOutcome::Failed { message, attempt } => JobOutcomeInfo::Failed {
                message: message.clone(),
                attempt: *attempt,
            },
        },
    }
}

pub(crate) fn job_info(
    job: &UploadJob,
    running: bool,
    last_run: Option<JobRunInfo>,
    next_run: Option<i64>,
) -> JobInfo {
    JobInfo {
        id: job.id.as_str().to_owned(),
        source: job.source.display().to_string(),
        pattern: job.pattern.as_str().to_owned(),
        target: job.target.as_str().to_owned(),
        schedule: match &job.trigger {
            JobTrigger::Schedule(schedule) => schedule.expression().to_owned(),
            JobTrigger::OnNewFile => ON_NEW_FILE.to_owned(),
        },
        keep_last: job.retention.keep_last(),
        max_age_days: job.retention.max_age_days(),
        verify: match job.verification {
            Verification::Size => JobVerification::Size,
            Verification::Hash => JobVerification::Hash,
        },
        running,
        last_run,
        next_run,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protonsync_domain::node::NodeId;

    #[test]
    fn errors_get_matching_codes() {
        assert_eq!(
            sync_error(&SyncError::Remote(RemoteError::Unauthorized)).code,
            ErrorCode::NotLoggedIn
        );
        assert_eq!(
            sync_error(&SyncError::Remote(RemoteError::NotFound(NodeId::new("x")))).code,
            ErrorCode::NotFound
        );
        assert_eq!(
            use_case_error(&UseCaseError::UnknownTrashEntry(TrashId(1))).code,
            ErrorCode::NotFound
        );
        assert_eq!(
            account_error(&AccountError::Rejected("x".into())).code,
            ErrorCode::LoginRejected
        );
        assert_eq!(
            sync_error(&SyncError::Local(LocalError::Io("x".into()))).code,
            ErrorCode::Unavailable
        );
    }

    #[test]
    fn choices_map_one_to_one() {
        assert_eq!(resolution(ResolutionChoice::KeepBoth), Resolution::KeepBoth);
        assert_eq!(direction(SyncDirection::UploadOnly), Direction::UploadOnly);
        assert_eq!(direction(SyncDirection::BackupOnly), Direction::BackupOnly);
        for api in [
            Direction::TwoWay,
            Direction::DownloadOnly,
            Direction::UploadOnly,
            Direction::BackupOnly,
        ] {
            assert_eq!(direction(sync_direction(api)), api);
        }
        assert_eq!(step_action(SyncAction::Nothing), None);
    }
}
