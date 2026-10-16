//! Übersetzung der Foto-Typen in API-Typen.

use protonsync_api::{
    ApiError, DuplicateGroupInfo, DuplicateReasonInfo, ErrorCode, FailureInfo, PhotoCounts,
    PhotoInfo, PhotoOutcomeInfo, TrashedPhotoInfo,
};
use protonsync_application::photos::{
    PhotoError, PhotoIndex, PhotoOutcome, PhotoPlan, PhotoReport, PhotoStepKind, TrashedDuplicate,
};
use protonsync_domain::node::NodeId;
use protonsync_domain::photo::Photo;
use protonsync_domain::photo::duplicates::{DuplicateGroup, DuplicateReason};

use super::{local_error, remote_error, skipped, state_error};

pub(crate) fn photo_error(error: &PhotoError) -> ApiError {
    match error {
        PhotoError::Remote(e) => remote_error(e),
        PhotoError::Local(e) => local_error(e),
        PhotoError::State(e) => state_error(e),
        PhotoError::Selection(_) => ApiError::new(ErrorCode::InvalidRequest, error.to_string()),
    }
}

pub(crate) fn photo_outcome(outcome: &Result<PhotoOutcome, PhotoError>) -> PhotoOutcomeInfo {
    match outcome {
        Ok(PhotoOutcome::Planned(plan)) => PhotoOutcomeInfo::Planned(planned_counts(plan)),
        Ok(PhotoOutcome::Applied(report)) => PhotoOutcomeInfo::Applied(applied_counts(report)),
        Err(error) => PhotoOutcomeInfo::Failed {
            message: photo_error(error).message,
        },
    }
}

fn planned_counts(plan: &PhotoPlan) -> PhotoCounts {
    let mut counts = PhotoCounts {
        ignored: plan.ignored,
        skipped: skipped(&plan.skipped),
        ..PhotoCounts::default()
    };
    for step in &plan.steps {
        match step.kind {
            PhotoStepKind::Download(_) => counts.downloaded += 1,
            PhotoStepKind::Link(_) => counts.linked += 1,
            PhotoStepKind::Upload => counts.uploaded += 1,
            PhotoStepKind::TrashLocal(_) => counts.trashed_local += 1,
            PhotoStepKind::TrashRemote(_) => counts.trashed_remote += 1,
            PhotoStepKind::Forget => {}
        }
    }
    counts
}

fn applied_counts(report: &PhotoReport) -> PhotoCounts {
    PhotoCounts {
        downloaded: report.downloaded,
        uploaded: report.uploaded,
        linked: report.linked,
        trashed_local: report.trashed_local,
        trashed_remote: report.trashed_remote,
        ignored: report.ignored,
        skipped: skipped(&report.skipped),
        failures: report
            .failures
            .iter()
            .map(|failure| FailureInfo {
                path: failure.path.to_string(),
                message: failure.message.clone(),
            })
            .collect(),
    }
}

/// Eine Duplikat-Gruppe mit den Angaben aus dem Index; Fotos, die der Index
/// nicht (mehr) kennt, fallen weg.
pub(crate) fn duplicate_group(
    group: &DuplicateGroup,
    index: &PhotoIndex,
) -> Option<DuplicateGroupInfo> {
    let info = |id: &NodeId| index.get(id).map(photo_info);
    Some(DuplicateGroupInfo {
        reason: match group.reason {
            DuplicateReason::SameContent => DuplicateReasonInfo::SameContent,
            DuplicateReason::SameNameAndCaptureTime => DuplicateReasonInfo::SameNameAndCaptureTime,
        },
        captured: group.captured.unix_seconds(),
        keep: info(&group.keep)?,
        others: group.others.iter().filter_map(info).collect(),
    })
}

fn photo_info(photo: &Photo) -> PhotoInfo {
    PhotoInfo {
        id: photo.id.as_str().to_owned(),
        name: photo.name.as_str().to_owned(),
        size: photo.size,
        uploaded: photo.uploaded.unix_seconds(),
        favorite: photo.favorite,
        albums: photo.albums,
    }
}

pub(crate) fn trashed_photo(trashed: &TrashedDuplicate) -> TrashedPhotoInfo {
    TrashedPhotoInfo {
        id: trashed.id.as_str().to_owned(),
        path: trashed.path.to_string(),
    }
}
