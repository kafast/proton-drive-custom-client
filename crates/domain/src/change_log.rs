//! Change log: what a sync run actually changed, on which side.
//!
//! Every executed step of a sync becomes one [`ChangeEntry`]. The log answers
//! "what happened to my files?", in particular "what changed in Proton?",
//! without looking at the sync plan or the logs of the daemon.

use crate::conflict::{ConflictKind, SyncAction};
use crate::path::RelativePath;
use crate::time::Timestamp;

/// Default number of days the change log keeps its entries.
pub const DEFAULT_CHANGE_LOG_DAYS: u32 = 30;

/// The side that was changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangedSide {
    /// The local folder was changed (the change came from Proton).
    Local,
    /// Proton was changed (the change came from the local folder).
    Remote,
}

/// What was done to the entry on the changed side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    /// Newly created (file or folder).
    Created,
    /// Content replaced by a new version.
    Modified,
    /// Moved to the trash.
    Trashed,
    /// Moved into the version archive (`_archiv`) instead of the trash.
    Archived,
    /// The old version was moved into the version archive and the new one put in its place.
    ArchivedAndReplaced,
    /// Deleted on the other side; the copy here was kept and is no longer tracked.
    KeptCopy,
    /// Both sides changed; both versions were kept (see the conflict list).
    Conflict,
}

/// One change made by a sync run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeEntry {
    /// When the change was made.
    pub at: Timestamp,
    /// Path relative to the root of the sync pair.
    pub path: RelativePath,
    /// The side that was changed.
    pub side: ChangedSide,
    /// What was done.
    pub kind: ChangeKind,
    /// `true` for a folder, `false` for a file.
    pub is_folder: bool,
}

/// Whether an entry existed on each side before a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Existed {
    /// Existed in the local folder.
    pub local: bool,
    /// Existed in Proton.
    pub remote: bool,
}

impl Existed {
    /// Existed on both sides.
    pub const BOTH: Self = Self {
        local: true,
        remote: true,
    };
}

/// Which side a step changes and how, or `None` if it changes nothing
/// worth logging (no-ops and changes ignored because of the sync direction).
///
/// `existed` says whether the entry existed locally and in Proton before
/// the step; on the changed side it separates [`ChangeKind::Created`] from
/// [`ChangeKind::Modified`]. `conflict` is the kind of a [`SyncAction::Conflict`].
#[must_use]
pub const fn classify(
    action: SyncAction,
    conflict: Option<ConflictKind>,
    existed: Existed,
) -> Option<(ChangedSide, ChangeKind)> {
    const fn created_or_modified(existed: bool) -> ChangeKind {
        if existed {
            ChangeKind::Modified
        } else {
            ChangeKind::Created
        }
    }
    match action {
        SyncAction::Upload => Some((ChangedSide::Remote, created_or_modified(existed.remote))),
        SyncAction::Download => Some((ChangedSide::Local, created_or_modified(existed.local))),
        SyncAction::TrashRemote => Some((ChangedSide::Remote, ChangeKind::Trashed)),
        SyncAction::TrashLocal => Some((ChangedSide::Local, ChangeKind::Trashed)),
        SyncAction::ArchiveRemote => Some((ChangedSide::Remote, ChangeKind::Archived)),
        SyncAction::ArchiveLocal => Some((ChangedSide::Local, ChangeKind::Archived)),
        SyncAction::ArchiveRemoteAndUpload => {
            Some((ChangedSide::Remote, ChangeKind::ArchivedAndReplaced))
        }
        SyncAction::ArchiveAndDownload => {
            Some((ChangedSide::Local, ChangeKind::ArchivedAndReplaced))
        }
        SyncAction::KeepLocalCopy => Some((ChangedSide::Local, ChangeKind::KeptCopy)),
        // Only this conflict writes to Proton (the local version is uploaded
        // again); all others keep a conflict copy locally.
        SyncAction::Conflict => match conflict {
            Some(ConflictKind::LocalModifiedRemoteDeleted) => {
                Some((ChangedSide::Remote, ChangeKind::Conflict))
            }
            _ => Some((ChangedSide::Local, ChangeKind::Conflict)),
        },
        SyncAction::Nothing
        | SyncAction::RecordConverged
        | SyncAction::IgnoreLocalChange
        | SyncAction::IgnoreRemoteChange => None,
    }
}

/// The oldest point in time still kept when entries are kept for `days` days.
#[must_use]
pub const fn keep_since(now: Timestamp, days: u32) -> Timestamp {
    // `as i64` is lossless: a u32 always fits into an i64.
    Timestamp::from_unix_seconds(
        now.unix_seconds()
            .saturating_sub(days as i64 * crate::time::SECONDS_PER_DAY),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfers_are_created_or_modified_on_the_target_side() {
        assert_eq!(
            classify(
                SyncAction::Upload,
                None,
                Existed {
                    local: true,
                    remote: false
                }
            ),
            Some((ChangedSide::Remote, ChangeKind::Created))
        );
        assert_eq!(
            classify(SyncAction::Upload, None, Existed::BOTH),
            Some((ChangedSide::Remote, ChangeKind::Modified))
        );
        assert_eq!(
            classify(
                SyncAction::Download,
                None,
                Existed {
                    local: false,
                    remote: true
                }
            ),
            Some((ChangedSide::Local, ChangeKind::Created))
        );
        assert_eq!(
            classify(SyncAction::Download, None, Existed::BOTH),
            Some((ChangedSide::Local, ChangeKind::Modified))
        );
    }

    #[test]
    fn deletions_and_archiving_name_the_changed_side() {
        use ChangeKind::{Archived, ArchivedAndReplaced, KeptCopy, Trashed};
        use ChangedSide::{Local, Remote};
        let cases = [
            (SyncAction::TrashRemote, Remote, Trashed),
            (SyncAction::TrashLocal, Local, Trashed),
            (SyncAction::ArchiveRemote, Remote, Archived),
            (SyncAction::ArchiveLocal, Local, Archived),
            (
                SyncAction::ArchiveRemoteAndUpload,
                Remote,
                ArchivedAndReplaced,
            ),
            (SyncAction::ArchiveAndDownload, Local, ArchivedAndReplaced),
            (SyncAction::KeepLocalCopy, Local, KeptCopy),
        ];
        for (action, side, kind) in cases {
            assert_eq!(
                classify(action, None, Existed::BOTH),
                Some((side, kind)),
                "{action:?}"
            );
        }
    }

    #[test]
    fn only_the_reupload_conflict_changes_proton() {
        assert_eq!(
            classify(
                SyncAction::Conflict,
                Some(ConflictKind::LocalModifiedRemoteDeleted),
                Existed::default()
            ),
            Some((ChangedSide::Remote, ChangeKind::Conflict))
        );
        for kind in [
            ConflictKind::BothModified,
            ConflictKind::LocalDeletedRemoteModified,
            ConflictKind::LocalModifiedRemoteProtected,
        ] {
            assert_eq!(
                classify(SyncAction::Conflict, Some(kind), Existed::BOTH),
                Some((ChangedSide::Local, ChangeKind::Conflict))
            );
        }
    }

    #[test]
    fn no_ops_and_ignored_changes_are_not_logged() {
        for action in [
            SyncAction::Nothing,
            SyncAction::RecordConverged,
            SyncAction::IgnoreLocalChange,
            SyncAction::IgnoreRemoteChange,
        ] {
            assert_eq!(classify(action, None, Existed::BOTH), None);
        }
    }

    #[test]
    fn keep_since_goes_back_whole_days_without_overflow() {
        let now = Timestamp::from_unix_seconds(10 * crate::time::SECONDS_PER_DAY);
        assert_eq!(
            keep_since(now, 3),
            Timestamp::from_unix_seconds(7 * crate::time::SECONDS_PER_DAY)
        );
        let early = Timestamp::from_unix_seconds(i64::MIN + 1);
        assert_eq!(keep_since(early, 1).unix_seconds(), i64::MIN);
    }
}
