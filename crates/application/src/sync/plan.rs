//! Planning: determine for each path what to do, without changing anything.
//!
//! The plan is also the result of a dry run.

use std::collections::HashMap;

use protonsync_domain::conflict::{Change, ConflictKind, SyncAction, decide, protect_remote};
use protonsync_domain::node::{Fingerprint, Node, NodeId, NodeKind};
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::{SyncDirection, SyncPair};

use super::snapshot::Snapshot;
use crate::ports::{BaseEntry, LocalKind, LocalStorage};

/// A planned step for a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedStep {
    /// Affected path.
    pub path: RelativePath,
    /// What is done.
    pub action: SyncAction,
    /// Kind of conflict, only for [`SyncAction::Conflict`].
    pub conflict: Option<ConflictKind>,
    pub(crate) local: Option<LocalKind>,
    pub(crate) remote: Option<Node>,
    /// Identifier in Proton according to the base (for re-checking before deleting locally).
    pub(crate) base_remote: Option<NodeId>,
}

/// A path that cannot be synced, with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedPath {
    /// Affected path.
    pub path: RelativePath,
    /// Why it is skipped.
    pub reason: String,
}

/// The complete plan of a run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncPlan {
    /// Steps in execution order (parents before children).
    pub steps: Vec<PlannedStep>,
    /// Paths that are skipped.
    pub skipped: Vec<SkippedPath>,
    /// Known remote identifiers per path (for parent folders when uploading).
    pub(crate) remote_ids: HashMap<RelativePath, NodeId>,
}

impl SyncPlan {
    /// `true` if there is nothing to do.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty() && self.skipped.is_empty()
    }
}

/// Creates the plan from a snapshot, following the direction and the
/// remote protection of `pair`.
pub(crate) fn make_plan<L: LocalStorage>(
    snapshot: &Snapshot,
    local_storage: &L,
    pair: &SyncPair,
) -> SyncPlan {
    let mut plan = SyncPlan {
        remote_ids: snapshot
            .remote
            .iter()
            .map(|(path, node)| (path.clone(), node.id.clone()))
            .collect(),
        ..SyncPlan::default()
    };
    // Subtrees that are already handled by a step on the folder.
    let mut covered: Vec<RelativePath> = Vec::new();

    for path in snapshot.all_paths() {
        if covered.iter().any(|done| path.starts_with(done)) {
            continue;
        }
        let local = snapshot.local.get(&path);
        let remote = snapshot.remote.get(&path);
        let base = snapshot.base.get(&path);

        if !kinds_agree(local, remote, base) {
            plan.skipped.push(SkippedPath {
                path: path.clone(),
                reason: "file and folder with the same name".into(),
            });
            covered.push(path);
            continue;
        }

        let local_change = local_change(base, local);
        let remote_change = remote_change(base, remote);
        let identical = local_change == Change::Modified
            && remote_change == Change::Modified
            && contents_identical(local_storage, &path, local, remote);
        let mut action = decide(local_change, remote_change, identical, pair.direction);

        if action == SyncAction::ArchiveAndDownload && !matches!(local, Some(LocalKind::File(_))) {
            // Nothing local to archive (new remote entry or a folder).
            action = SyncAction::Download;
        }

        let is_folder = base.is_some_and(BaseEntry::is_folder);
        if is_folder {
            action = protect_folder_contents(snapshot, &path, action, pair.direction);
        }
        if pair.protect_remote {
            action = protect_remote_entry(local_storage, &path, action, local, remote);
        }
        if is_folder
            && matches!(
                action,
                SyncAction::TrashLocal | SyncAction::TrashRemote | SyncAction::KeepLocalCopy
            )
        {
            covered.push(path.clone());
        }

        if action != SyncAction::Nothing {
            plan.steps.push(PlannedStep {
                conflict: (action == SyncAction::Conflict).then(|| {
                    ConflictKind::from_changes(local_change, remote_change)
                        // Only the protection turns a one-sided change into a conflict.
                        .unwrap_or(ConflictKind::LocalModifiedRemoteProtected)
                }),
                path,
                action,
                local: local.cloned(),
                remote: remote.cloned(),
                base_remote: base.map(|entry| entry.remote_id.clone()),
            });
        }
    }
    plan
}

/// Applies [`protect_remote`]. A local file that replaces a remote one with the
/// same content (e.g. deleted and restored) is no conflict, only the base is updated.
fn protect_remote_entry<L: LocalStorage>(
    local_storage: &L,
    path: &RelativePath,
    action: SyncAction,
    local: Option<&LocalKind>,
    remote: Option<&Node>,
) -> SyncAction {
    let protected = protect_remote(action, remote.is_some());
    if action == SyncAction::Upload
        && protected == SyncAction::Conflict
        && contents_identical(local_storage, path, local, remote)
    {
        SyncAction::RecordConverged
    } else {
        protected
    }
}

/// A deleted folder is not moved to the trash if something inside it was
/// changed on the other side. Instead it is recreated,
/// and its children are synced individually.
///
/// Without upload (`download_only`) a folder deleted in Proton is never
/// recreated there: it stays locally and its children are decided one by one.
/// A folder deleted locally whose remote content changed is recreated locally,
/// otherwise the changed files could never be downloaded (their parent is missing).
fn protect_folder_contents(
    snapshot: &Snapshot,
    folder: &RelativePath,
    action: SyncAction,
    direction: SyncDirection,
) -> SyncAction {
    match action {
        // `IgnoreLocalChange` on a known folder means: deleted locally, not passed on.
        SyncAction::TrashRemote | SyncAction::IgnoreLocalChange
            if direction.allows_download() && remote_changed_below(snapshot, folder) =>
        {
            SyncAction::Download
        }
        SyncAction::TrashLocal if local_changed_below(snapshot, folder) => {
            if direction.allows_upload() {
                SyncAction::Upload
            } else {
                SyncAction::Nothing
            }
        }
        other => other,
    }
}

fn descendants<'a>(
    snapshot: &'a Snapshot,
    folder: &'a RelativePath,
) -> impl Iterator<Item = RelativePath> + 'a {
    snapshot
        .all_paths()
        .into_iter()
        .filter(move |path| path != folder && path.starts_with(folder))
}

fn remote_changed_below(snapshot: &Snapshot, folder: &RelativePath) -> bool {
    descendants(snapshot, folder).any(|path| {
        remote_change(snapshot.base.get(&path), snapshot.remote.get(&path)) == Change::Modified
    })
}

fn local_changed_below(snapshot: &Snapshot, folder: &RelativePath) -> bool {
    descendants(snapshot, folder).any(|path| {
        local_change(snapshot.base.get(&path), snapshot.local.get(&path)) == Change::Modified
    })
}

/// All present sides must have the same kind (file/folder).
fn kinds_agree(local: Option<&LocalKind>, remote: Option<&Node>, base: Option<&BaseEntry>) -> bool {
    let kinds = [
        local.map(|kind| matches!(kind, LocalKind::Folder)),
        remote.map(Node::is_folder),
        base.map(BaseEntry::is_folder),
    ];
    let mut present = kinds.into_iter().flatten();
    match present.next() {
        Some(first) => present.all(|other| other == first),
        None => true,
    }
}

/// Locally, size and modification time are enough as a change marker.
fn same_local_state(known: &Fingerprint, current: &Fingerprint) -> bool {
    known.size == current.size && known.modified == current.modified
}

pub(crate) fn local_change(base: Option<&BaseEntry>, local: Option<&LocalKind>) -> Change {
    match (base, local) {
        (None, None) => Change::Unchanged,
        (Some(_), None) => Change::Deleted,
        (None, Some(_)) => Change::Modified,
        (Some(_), Some(LocalKind::Folder)) => Change::Unchanged,
        (Some(base), Some(LocalKind::File(current))) => match &base.local {
            Some(known) if same_local_state(known, current) => Change::Unchanged,
            _ => Change::Modified,
        },
    }
}

pub(crate) fn remote_change(base: Option<&BaseEntry>, remote: Option<&Node>) -> Change {
    match (base, remote) {
        (None, None) => Change::Unchanged,
        (Some(_), None) => Change::Deleted,
        (None, Some(_)) => Change::Modified,
        (Some(base), Some(node)) => {
            let same_revision = match &node.kind {
                NodeKind::Folder => true,
                NodeKind::File { revision, .. } => base.remote_revision.as_ref() == Some(revision),
            };
            if base.remote_id == node.id && same_revision {
                Change::Unchanged
            } else {
                Change::Modified
            }
        }
    }
}

/// Do both sides have the same content after their changes?
///
/// Folders count as equal. For files the size must match and then the
/// SHA-1 checksum (if Proton knows one), otherwise the modification time.
/// When in doubt `false`: then a conflict arises, but never data loss.
fn contents_identical<L: LocalStorage>(
    local_storage: &L,
    path: &RelativePath,
    local: Option<&LocalKind>,
    remote: Option<&Node>,
) -> bool {
    match (local, remote.map(|node| (node, node.fingerprint()))) {
        (Some(LocalKind::Folder), Some((node, None))) => node.is_folder(),
        (Some(LocalKind::File(local_fp)), Some((_, Some(remote_fp)))) => {
            if local_fp.size != remote_fp.size {
                return false;
            }
            match &remote_fp.content_hash {
                Some(remote_hash) => local_storage
                    .content_hash(path)
                    .is_ok_and(|local_hash| &local_hash == remote_hash),
                None => local_fp.modified == remote_fp.modified,
            }
        }
        _ => false,
    }
}
