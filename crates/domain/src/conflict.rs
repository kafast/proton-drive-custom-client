//! Deciding what happens to an entry during sync, and conflicts.
//!
//! The basis is a three-way comparison: for each side (local, remote) the
//! current state is compared with the last state known to both ("base").
//! The sides have different characteristics (local: size and
//! modification time, remote: revision id), so each side is condensed
//! into a [`Change`] on its own.

use crate::path::RelativePath;
use crate::sync_pair::SyncDirection;
use crate::time::Timestamp;

/// How a side has changed since the base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Unchanged (or absent at both points in time).
    Unchanged,
    /// Newly created or content changed.
    Modified,
    /// Deleted since the base.
    Deleted,
}

impl Change {
    /// Compares the base and the current state of a side. `None` = absent.
    #[must_use]
    pub fn detect<T: PartialEq>(base: Option<&T>, current: Option<&T>) -> Self {
        match (base, current) {
            (None, None) => Self::Unchanged,
            (Some(_), None) => Self::Deleted,
            (Some(base), Some(current)) if base == current => Self::Unchanged,
            (_, Some(_)) => Self::Modified,
        }
    }
}

/// What the sync should do for an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncAction {
    /// Do nothing, both sides match the base.
    Nothing,
    /// Both sides changed the same way; only update the base.
    RecordConverged,
    /// Upload the local state.
    Upload,
    /// Download the remote state.
    Download,
    /// Move the remote entry to the trash (deleted locally).
    TrashRemote,
    /// Move the local entry to the trash (deleted remotely).
    TrashLocal,
    /// Local change is not transferred because of the sync direction.
    IgnoreLocalChange,
    /// Remote change is not applied because of the sync direction.
    IgnoreRemoteChange,
    /// Both sides changed differently: keep both versions and ask.
    Conflict,
}

/// Determines the action for an entry.
///
/// `contents_identical` says whether both sides have the same content after the change
/// (only relevant if both were changed; `false` when in doubt).
///
/// Rule: with changes on both sides, a side is **never** overwritten automatically,
/// not even with a one-way sync direction. This always becomes
/// a [`SyncAction::Conflict`], unless both sides are equal.
#[must_use]
pub fn decide(
    local: Change,
    remote: Change,
    contents_identical: bool,
    direction: SyncDirection,
) -> SyncAction {
    use Change::{Deleted, Modified, Unchanged};
    match (local, remote) {
        (Unchanged, Unchanged) => SyncAction::Nothing,
        (Modified | Deleted, Unchanged) if !direction.allows_upload() => {
            SyncAction::IgnoreLocalChange
        }
        (Modified, Unchanged) => SyncAction::Upload,
        (Deleted, Unchanged) => SyncAction::TrashRemote,
        (Unchanged, Modified | Deleted) if !direction.allows_download() => {
            SyncAction::IgnoreRemoteChange
        }
        (Unchanged, Modified) => SyncAction::Download,
        (Unchanged, Deleted) => SyncAction::TrashLocal,
        (Deleted, Deleted) => SyncAction::RecordConverged,
        (Modified, Modified) if contents_identical => SyncAction::RecordConverged,
        (Modified, Modified | Deleted) | (Deleted, Modified) => SyncAction::Conflict,
    }
}

/// How a conflict should be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// The local version wins, the remote one goes to the trash.
    KeepLocal,
    /// The remote version wins, the local one goes to the trash.
    KeepRemote,
    /// Keep both; the local one is renamed as a conflict copy.
    KeepBoth,
}

/// Which changes led to the conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    /// Both sides changed the content differently.
    BothModified,
    /// Changed locally, deleted remotely. The local version is kept.
    LocalModifiedRemoteDeleted,
    /// Deleted locally, changed remotely. The remote version is kept.
    LocalDeletedRemoteModified,
}

impl ConflictKind {
    /// Derives the kind from the changes on both sides; `None` if this is not a conflict.
    #[must_use]
    pub const fn from_changes(local: Change, remote: Change) -> Option<Self> {
        match (local, remote) {
            (Change::Modified, Change::Modified) => Some(Self::BothModified),
            (Change::Modified, Change::Deleted) => Some(Self::LocalModifiedRemoteDeleted),
            (Change::Deleted, Change::Modified) => Some(Self::LocalDeletedRemoteModified),
            _ => None,
        }
    }
}

/// An open conflict in the queue.
///
/// On detection both versions are always kept: with
/// [`ConflictKind::BothModified`] the local version is placed next to it as a conflict copy
/// (`conflict_copy`), the remote one stays under the original name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// Path relative to the root folder of the sync pair.
    pub relative_path: RelativePath,
    /// Kind of conflict.
    pub kind: ConflictKind,
    /// Path of the conflict copy relative to the root folder, if one was created.
    pub conflict_copy: Option<RelativePath>,
    /// When the conflict was detected.
    pub detected_at: Timestamp,
}

/// Name of the conflict copy, e.g. `report (Konflikt 1760000000).pdf`.
///
/// The marker text `Konflikt` is part of the file name format and stays as it is.
/// The extension is kept so that the copy opens with the same program.
#[must_use]
pub fn conflict_copy_name(original: &str, detected_at: Timestamp) -> String {
    let marker = format!("(Konflikt {})", detected_at.unix_seconds());
    match original.rsplit_once('.') {
        // A leading dot (".bashrc") is not an extension.
        Some((stem, extension)) if !stem.is_empty() => format!("{stem} {marker}.{extension}"),
        _ => format!("{original} {marker}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Change::{Deleted, Modified, Unchanged};
    use proptest::prelude::*;

    const TWO_WAY: SyncDirection = SyncDirection::TwoWay;

    #[test]
    fn detect_compares_base_and_current() {
        assert_eq!(Change::detect::<u8>(None, None), Unchanged);
        assert_eq!(Change::detect(Some(&1), Some(&1)), Unchanged);
        assert_eq!(Change::detect(Some(&1), Some(&2)), Modified);
        assert_eq!(Change::detect(None, Some(&1)), Modified);
        assert_eq!(Change::detect(Some(&1), None), Deleted);
    }

    #[test]
    fn unchanged_needs_nothing() {
        assert_eq!(
            decide(Unchanged, Unchanged, false, TWO_WAY),
            SyncAction::Nothing
        );
    }

    #[test]
    fn one_sided_changes_are_transferred() {
        assert_eq!(
            decide(Modified, Unchanged, false, TWO_WAY),
            SyncAction::Upload
        );
        assert_eq!(
            decide(Unchanged, Modified, false, TWO_WAY),
            SyncAction::Download
        );
    }

    #[test]
    fn deletions_go_to_trash() {
        assert_eq!(
            decide(Deleted, Unchanged, false, TWO_WAY),
            SyncAction::TrashRemote
        );
        assert_eq!(
            decide(Unchanged, Deleted, false, TWO_WAY),
            SyncAction::TrashLocal
        );
    }

    #[test]
    fn different_edits_on_both_sides_are_a_conflict() {
        assert_eq!(
            decide(Modified, Modified, false, TWO_WAY),
            SyncAction::Conflict
        );
    }

    #[test]
    fn edit_versus_delete_is_a_conflict() {
        assert_eq!(
            decide(Modified, Deleted, false, TWO_WAY),
            SyncAction::Conflict
        );
        assert_eq!(
            decide(Deleted, Modified, false, TWO_WAY),
            SyncAction::Conflict
        );
    }

    #[test]
    fn identical_results_converge() {
        assert_eq!(
            decide(Modified, Modified, true, TWO_WAY),
            SyncAction::RecordConverged
        );
        assert_eq!(
            decide(Deleted, Deleted, false, TWO_WAY),
            SyncAction::RecordConverged
        );
    }

    #[test]
    fn one_way_directions_ignore_the_other_side() {
        let download_only = SyncDirection::DownloadOnly;
        let upload_only = SyncDirection::UploadOnly;
        assert_eq!(
            decide(Modified, Unchanged, false, download_only),
            SyncAction::IgnoreLocalChange
        );
        assert_eq!(
            decide(Deleted, Unchanged, false, download_only),
            SyncAction::IgnoreLocalChange
        );
        assert_eq!(
            decide(Unchanged, Modified, false, upload_only),
            SyncAction::IgnoreRemoteChange
        );
        assert_eq!(
            decide(Unchanged, Deleted, false, upload_only),
            SyncAction::IgnoreRemoteChange
        );
    }

    #[test]
    fn conflict_kind_matches_changes() {
        assert_eq!(
            ConflictKind::from_changes(Modified, Modified),
            Some(ConflictKind::BothModified)
        );
        assert_eq!(
            ConflictKind::from_changes(Modified, Deleted),
            Some(ConflictKind::LocalModifiedRemoteDeleted)
        );
        assert_eq!(
            ConflictKind::from_changes(Deleted, Modified),
            Some(ConflictKind::LocalDeletedRemoteModified)
        );
        assert_eq!(ConflictKind::from_changes(Unchanged, Modified), None);
    }

    #[test]
    fn conflict_copy_keeps_extension() {
        let at = Timestamp::from_unix_seconds(42);
        assert_eq!(
            conflict_copy_name("report.pdf", at),
            "report (Konflikt 42).pdf"
        );
        assert_eq!(conflict_copy_name("Makefile", at), "Makefile (Konflikt 42)");
        assert_eq!(conflict_copy_name(".bashrc", at), ".bashrc (Konflikt 42)");
    }

    fn any_change() -> impl Strategy<Value = Change> {
        prop_oneof![Just(Unchanged), Just(Modified), Just(Deleted)]
    }

    fn any_direction() -> impl Strategy<Value = SyncDirection> {
        prop_oneof![
            Just(SyncDirection::TwoWay),
            Just(SyncDirection::DownloadOnly),
            Just(SyncDirection::UploadOnly),
        ]
    }

    proptest! {
        /// A side that has changed is never overwritten silently.
        #[test]
        fn never_overwrites_a_changed_side(
            local in any_change(), remote in any_change(), identical: bool,
            direction in any_direction(),
        ) {
            let action = decide(local, remote, identical, direction);
            if local != Unchanged {
                prop_assert!(!matches!(action, SyncAction::Download | SyncAction::TrashLocal));
            }
            if remote != Unchanged {
                prop_assert!(!matches!(action, SyncAction::Upload | SyncAction::TrashRemote));
            }
        }

        /// The sync direction is always respected.
        #[test]
        fn respects_direction(
            local in any_change(), remote in any_change(), identical: bool,
            direction in any_direction(),
        ) {
            let action = decide(local, remote, identical, direction);
            if !direction.allows_upload() {
                prop_assert!(!matches!(action, SyncAction::Upload | SyncAction::TrashRemote));
            }
            if !direction.allows_download() {
                prop_assert!(!matches!(action, SyncAction::Download | SyncAction::TrashLocal));
            }
        }

        /// A conflict can only arise if both sides were changed.
        #[test]
        fn conflicts_need_changes_on_both_sides(
            local in any_change(), remote in any_change(), identical: bool,
            direction in any_direction(),
        ) {
            if decide(local, remote, identical, direction) == SyncAction::Conflict {
                prop_assert!(local != Unchanged && remote != Unchanged);
            }
        }
    }
}
