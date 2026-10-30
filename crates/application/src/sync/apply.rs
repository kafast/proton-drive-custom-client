//! Execution of a plan.
//!
//! Each step writes the base only after it has fully succeeded.
//! If a step fails, it is noted in the report and the next run
//! tries it again. Only a missing login aborts the whole run.

use protonsync_domain::archive::{ARCHIVE_FOLDER, archived_name};
use protonsync_domain::conflict::{Conflict, ConflictKind, SyncAction, conflict_copy_name};
use protonsync_domain::node::{Fingerprint, Node, NodeId, NodeKind, NodeName};
use protonsync_domain::path::RelativePath;
use protonsync_domain::trash::{TrashEntry, TrashOrigin};

use super::plan::{PlannedStep, SyncPlan};
use super::remote_index::{RemoteCache, RemoteMutation};
use super::{StepFailure, SyncContext, SyncError, SyncReport};
use crate::ports::{
    BaseEntry, Clock, LocalError, LocalKind, LocalStorage, RemoteError, RemoteStorage, StateRepo,
};

/// Error of a single step.
#[derive(Debug)]
enum StepError {
    Remote(RemoteError),
    Local(LocalError),
    State(crate::ports::StateError),
    /// The step no longer matches the current state and is planned again in the next run.
    Outdated(&'static str),
}

impl From<RemoteError> for StepError {
    fn from(error: RemoteError) -> Self {
        Self::Remote(error)
    }
}

impl From<LocalError> for StepError {
    fn from(error: LocalError) -> Self {
        Self::Local(error)
    }
}

impl From<crate::ports::StateError> for StepError {
    fn from(error: crate::ports::StateError) -> Self {
        Self::State(error)
    }
}

impl std::fmt::Display for StepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Remote(error) => write!(f, "{error}"),
            Self::Local(error) => write!(f, "{error}"),
            Self::State(error) => write!(f, "{error}"),
            Self::Outdated(reason) => f.write_str(reason),
        }
    }
}

/// How the execution deals with the cached remote tree.
pub(crate) struct RemoteView<'c> {
    /// Own changes in Proton are recorded here immediately.
    pub cache: &'c mut RemoteCache,
    /// Before deleting locally, check whether the node is really missing in Proton.
    /// Needed when the tree comes from events instead of a fresh read.
    pub verify_deletions: bool,
}

/// Executes all steps and counts the results.
pub(crate) async fn apply_plan<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    mut plan: SyncPlan,
    view: &mut RemoteView<'_>,
) -> Result<SyncReport, SyncError>
where
    R: RemoteStorage,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    let mut report = SyncReport {
        skipped: plan.skipped.clone(),
        ..SyncReport::default()
    };
    let steps = std::mem::take(&mut plan.steps);
    for step in &steps {
        match apply_step(context, &mut plan, view, step).await {
            Ok(()) => report.count(step.action),
            Err(StepError::Remote(RemoteError::Unauthorized)) => {
                return Err(SyncError::Remote(RemoteError::Unauthorized));
            }
            Err(error) => report.failures.push(StepFailure {
                path: step.path.clone(),
                action: step.action,
                message: error.to_string(),
            }),
        }
    }
    Ok(report)
}

async fn apply_step<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    plan: &mut SyncPlan,
    view: &mut RemoteView<'_>,
    step: &PlannedStep,
) -> Result<(), StepError>
where
    R: RemoteStorage,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    match step.action {
        SyncAction::Nothing | SyncAction::IgnoreLocalChange | SyncAction::IgnoreRemoteChange => {
            Ok(())
        }
        SyncAction::Upload => upload(context, plan, view, step).await,
        SyncAction::Download => download(context, plan, step).await,
        SyncAction::TrashRemote => trash_remote(context, view, step).await,
        SyncAction::TrashLocal => trash_local(context, view, step).await,
        SyncAction::RecordConverged => record_converged(context, step),
        SyncAction::Conflict => conflict(context, plan, view, step).await,
        SyncAction::ArchiveAndDownload => archive_and_download(context, plan, step).await,
        SyncAction::KeepLocalCopy => keep_local_copy(context, step),
        SyncAction::ArchiveLocal => archive_local(context, view, step).await,
        SyncAction::ArchiveRemote => archive_remote(context, plan, view, step).await,
        SyncAction::ArchiveRemoteAndUpload => {
            archive_remote(context, plan, view, step).await?;
            let without_remote = PlannedStep {
                remote: None,
                ..step.clone()
            };
            upload(context, plan, view, &without_remote).await
        }
    }
}

fn file_name(path: &RelativePath) -> Result<NodeName, StepError> {
    path.file_name()
        .ok_or(StepError::Outdated("root folder cannot be synced"))
}

/// Identifier of the remote parent folder of `path`.
fn remote_parent<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    plan: &SyncPlan,
    path: &RelativePath,
) -> Result<NodeId, StepError> {
    match path.parent() {
        Some(parent) if parent.is_root() => Ok(context.pair.remote_root.clone()),
        Some(parent) => plan
            .remote_ids
            .get(&parent)
            .cloned()
            .ok_or(StepError::Outdated("parent folder missing in Proton")),
        None => Err(StepError::Outdated("root folder has no parent folder")),
    }
}

fn base_for(path: &RelativePath, node: &Node, local: Option<Fingerprint>) -> BaseEntry {
    let remote_revision = match &node.kind {
        NodeKind::File { revision, .. } => Some(revision.clone()),
        NodeKind::Folder => None,
    };
    BaseEntry {
        path: path.clone(),
        remote_id: node.id.clone(),
        remote_revision,
        local,
    }
}

/// Prevents a local change made after the scan from being overwritten or deleted.
fn ensure_local_unchanged<L: LocalStorage>(local: &L, step: &PlannedStep) -> Result<(), StepError> {
    let current = local.entry(&step.path)?.map(|entry| entry.kind);
    if current == step.local {
        Ok(())
    } else {
        Err(StepError::Outdated("changed locally during the run"))
    }
}

async fn upload<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    plan: &mut SyncPlan,
    view: &mut RemoteView<'_>,
    step: &PlannedStep,
) -> Result<(), StepError>
where
    R: RemoteStorage,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    let node = match (&step.local, &step.remote) {
        (Some(LocalKind::Folder), Some(existing)) => existing.clone(),
        (Some(LocalKind::Folder), None) => {
            let parent = remote_parent(context, plan, &step.path)?;
            context
                .remote
                .create_folder(&parent, &file_name(&step.path)?)
                .await?
        }
        (Some(LocalKind::File(fingerprint)), existing) => {
            upload_file(context, plan, &step.path, fingerprint, existing.as_ref()).await?
        }
        (None, _) => return Err(StepError::Outdated("local file missing")),
    };
    let local = match &step.local {
        Some(LocalKind::File(fingerprint)) => Some(fingerprint.clone()),
        _ => None,
    };
    plan.remote_ids.insert(step.path.clone(), node.id.clone());
    context
        .state
        .save_base(&context.pair.id, &base_for(&step.path, &node, local))?;
    view.cache.record(RemoteMutation::Stored {
        path: step.path.clone(),
        node,
    });
    Ok(())
}

/// Uploads a local file: as a new revision if it already exists remotely.
async fn upload_file<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    plan: &SyncPlan,
    path: &RelativePath,
    fingerprint: &Fingerprint,
    existing: Option<&Node>,
) -> Result<Node, StepError>
where
    R: RemoteStorage,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    let reader = context.local.open(path)?;
    let node = if let Some(node) = existing {
        context
            .remote
            .upload_revision(&node.id, reader, fingerprint.size, fingerprint.modified)
            .await?
    } else {
        let parent = remote_parent(context, plan, path)?;
        context
            .remote
            .upload_file(
                &parent,
                &file_name(path)?,
                reader,
                fingerprint.size,
                fingerprint.modified,
            )
            .await?
    };
    Ok(node)
}

async fn download<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    plan: &mut SyncPlan,
    step: &PlannedStep,
) -> Result<(), StepError>
where
    R: RemoteStorage,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    let node = step
        .remote
        .as_ref()
        .ok_or(StepError::Outdated("remote file missing"))?;
    let local = if node.is_folder() {
        context.local.create_folder(&step.path)?;
        None
    } else {
        ensure_local_unchanged(context.local, step)?;
        Some(download_file(context, &step.path, node).await?)
    };
    plan.remote_ids.insert(step.path.clone(), node.id.clone());
    context
        .state
        .save_base(&context.pair.id, &base_for(&step.path, node, local))?;
    Ok(())
}

/// Downloads a remote file to `path`; it only becomes visible at the end.
async fn download_file<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    path: &RelativePath,
    node: &Node,
) -> Result<Fingerprint, StepError>
where
    R: RemoteStorage,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    let modified = node
        .fingerprint()
        .map_or_else(|| context.clock.now(), |remote| remote.modified);
    let mut staging = context.local.stage(path)?;
    context.remote.download_to(&node.id, &mut staging).await?;
    Ok(context.local.commit(staging, path, modified)?)
}

async fn trash_remote<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    view: &mut RemoteView<'_>,
    step: &PlannedStep,
) -> Result<(), StepError>
where
    R: RemoteStorage,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    let node = step
        .remote
        .as_ref()
        .ok_or(StepError::Outdated("remote node missing"))?;
    context.remote.trash(&node.id).await?;
    view.cache.record(RemoteMutation::Trashed {
        path: step.path.clone(),
    });
    let entry = TrashEntry {
        original_path: step.path.clone(),
        origin: TrashOrigin::Remote {
            node: node.id.clone(),
        },
        trashed_at: context.clock.now(),
    };
    context.state.add_trash(&context.pair.id, &entry)?;
    context
        .state
        .remove_base_tree(&context.pair.id, &step.path)?;
    Ok(())
}

async fn trash_local<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    view: &mut RemoteView<'_>,
    step: &PlannedStep,
) -> Result<(), StepError>
where
    R: RemoteStorage,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    if view.verify_deletions {
        ensure_remote_gone(context, view, step).await?;
    }
    ensure_local_unchanged(context.local, step)?;
    let stored_as = context.local.move_to_trash(&step.path)?;
    let entry = TrashEntry {
        original_path: step.path.clone(),
        origin: TrashOrigin::Local { stored_as },
        trashed_at: context.clock.now(),
    };
    context.state.add_trash(&context.pair.id, &entry)?;
    context
        .state
        .remove_base_tree(&context.pair.id, &step.path)?;
    Ok(())
}

/// Only what is provably missing in Proton is deleted locally. If the node
/// is still there, the cached tree was outdated: it is discarded,
/// and the next (full) run decides again.
async fn ensure_remote_gone<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    view: &mut RemoteView<'_>,
    step: &PlannedStep,
) -> Result<(), StepError>
where
    R: RemoteStorage,
{
    let Some(id) = &step.base_remote else {
        return Ok(());
    };
    match context.remote.node(id).await {
        Err(RemoteError::NotFound(_)) => Ok(()),
        Err(error) => Err(error.into()),
        Ok(_) => {
            view.cache.invalidate();
            Err(StepError::Outdated(
                "still present in Proton; the next full sync checks again",
            ))
        }
    }
}

fn record_converged<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    step: &PlannedStep,
) -> Result<(), StepError>
where
    S: StateRepo,
{
    match (&step.local, &step.remote) {
        (Some(local), Some(node)) => {
            let fingerprint = match local {
                LocalKind::File(fingerprint) => Some(fingerprint.clone()),
                LocalKind::Folder => None,
            };
            context
                .state
                .save_base(&context.pair.id, &base_for(&step.path, node, fingerprint))?;
        }
        _ => context
            .state
            .remove_base_tree(&context.pair.id, &step.path)?,
    }
    Ok(())
}

/// Upper bound for the counter in archive names; reaching it means something is badly wrong.
const MAX_ARCHIVE_ATTEMPTS: u32 = 100;

/// Backup mode: move the local file into the version archive next to it,
/// then download the remote version into its place.
async fn archive_and_download<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    plan: &mut SyncPlan,
    step: &PlannedStep,
) -> Result<(), StepError>
where
    R: RemoteStorage,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    ensure_local_unchanged(context.local, step)?;
    archive_locally(context, &step.path)?;
    let without_local = PlannedStep {
        local: None,
        ..step.clone()
    };
    download(context, plan, &without_local).await
}

fn archive_folder_name() -> Result<NodeName, StepError> {
    NodeName::new(ARCHIVE_FOLDER).map_err(|_| StepError::Outdated("invalid archive name"))
}

/// Moves the local `path` (file or folder) into the archive folder next to it.
fn archive_locally<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    path: &RelativePath,
) -> Result<(), StepError>
where
    L: LocalStorage,
    C: Clock,
{
    let archive = path
        .parent()
        .unwrap_or_default()
        .join(&archive_folder_name()?);
    context.local.create_folder(&archive)?;
    move_into_archive(context, path, &archive)
}

/// Renames `path` into `archive` under a free, time-prefixed name.
fn move_into_archive<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    path: &RelativePath,
    archive: &RelativePath,
) -> Result<(), StepError>
where
    L: LocalStorage,
    C: Clock,
{
    let original = file_name(path)?;
    let now = context.clock.now();
    // Attempt 0 has no counter; the counter starts at 2 like "report (2).pdf".
    for attempt in std::iter::once(0).chain(2..MAX_ARCHIVE_ATTEMPTS) {
        let name = archived_name(original.as_str(), now, attempt)
            .and_then(|name| NodeName::new(name).ok())
            .ok_or(StepError::Outdated(
                "name of the archived version is invalid",
            ))?;
        match context.local.rename(path, &archive.join(&name)) {
            Err(LocalError::AlreadyExists(_)) => {}
            other => return Ok(other?),
        }
    }
    Err(StepError::Outdated("no free name in the archive folder"))
}

/// Backup mode: deleted remotely; the local copy stays, only the base is dropped,
/// so the entry is no longer tracked.
fn keep_local_copy<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    step: &PlannedStep,
) -> Result<(), StepError>
where
    S: StateRepo,
{
    context
        .state
        .remove_base_tree(&context.pair.id, &step.path)?;
    Ok(())
}

/// Option `archive_versions`: deleted remotely; the local entry goes into the
/// local version archive instead of the trash and is no longer tracked.
async fn archive_local<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    view: &mut RemoteView<'_>,
    step: &PlannedStep,
) -> Result<(), StepError>
where
    R: RemoteStorage,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    if view.verify_deletions {
        ensure_remote_gone(context, view, step).await?;
    }
    ensure_local_unchanged(context.local, step)?;
    archive_locally(context, &step.path)?;
    context
        .state
        .remove_base_tree(&context.pair.id, &step.path)?;
    Ok(())
}

/// Option `archive_versions`: moves the remote entry of `step` into the archive
/// folder next to it in Proton, under a time-prefixed name. The base is
/// dropped; an upload into its place (if any) writes a new one.
async fn archive_remote<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    plan: &mut SyncPlan,
    view: &mut RemoteView<'_>,
    step: &PlannedStep,
) -> Result<(), StepError>
where
    R: RemoteStorage,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    let node = step
        .remote
        .as_ref()
        .ok_or(StepError::Outdated("remote node missing"))?;
    let parent_path = step.path.parent().unwrap_or_default();
    let archive = remote_archive_folder(context, plan, view, &parent_path, &step.path).await?;
    let archived = move_into_remote_archive(context, node, &archive).await?;
    view.cache.record(RemoteMutation::Trashed {
        path: step.path.clone(),
    });
    let archive_path = parent_path.join(&archive_folder_name()?);
    view.cache.record(RemoteMutation::Stored {
        path: archive_path.join(&archived.name),
        node: archived,
    });
    plan.remote_ids.remove(&step.path);
    context
        .state
        .remove_base_tree(&context.pair.id, &step.path)?;
    Ok(())
}

/// The archive folder in Proton inside `parent_path`; created if missing.
async fn remote_archive_folder<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    plan: &mut SyncPlan,
    view: &mut RemoteView<'_>,
    parent_path: &RelativePath,
    path: &RelativePath,
) -> Result<NodeId, StepError>
where
    R: RemoteStorage,
{
    let name = archive_folder_name()?;
    let archive_path = parent_path.join(&name);
    if let Some(id) = plan.remote_ids.get(&archive_path) {
        return Ok(id.clone());
    }
    let parent = remote_parent(context, plan, path)?;
    let folder = match find_remote_archive(context, &parent, &name).await? {
        Some(folder) => folder,
        None => match context.remote.create_folder(&parent, &name).await {
            Ok(folder) => folder,
            // Created by someone else in the meantime.
            Err(RemoteError::AlreadyExists(_)) => find_remote_archive(context, &parent, &name)
                .await?
                .ok_or(StepError::Outdated("archive folder in Proton vanished"))?,
            Err(error) => return Err(error.into()),
        },
    };
    plan.remote_ids
        .insert(archive_path.clone(), folder.id.clone());
    let id = folder.id.clone();
    view.cache.record(RemoteMutation::Stored {
        path: archive_path,
        node: folder,
    });
    Ok(id)
}

async fn find_remote_archive<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    parent: &NodeId,
    name: &NodeName,
) -> Result<Option<Node>, StepError>
where
    R: RemoteStorage,
{
    let existing = context
        .remote
        .list_children(parent)
        .await?
        .into_iter()
        .find(|child| &child.name == name);
    match existing {
        Some(node) if node.is_folder() => Ok(Some(node)),
        Some(_) => Err(StepError::Outdated(
            "a file named _archiv blocks the archive folder in Proton",
        )),
        None => Ok(None),
    }
}

/// Moves `node` into `archive` and gives it a free, time-prefixed name there.
///
/// Moving first and renaming afterwards means that a failure halfway leaves the
/// node inside the archive (which is not synced) rather than next to the original.
/// If the original name is already taken inside the archive, the node is renamed
/// in place first.
async fn move_into_remote_archive<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    node: &Node,
    archive: &NodeId,
) -> Result<Node, StepError>
where
    R: RemoteStorage,
    C: Clock,
{
    let now = context.clock.now();
    match context.remote.move_node(&node.id, archive).await {
        Ok(_) => rename_to_free_archive_name(context, &node.id, &node.name, now).await,
        Err(RemoteError::AlreadyExists(_)) => {
            rename_to_free_archive_name(context, &node.id, &node.name, now).await?;
            Ok(context.remote.move_node(&node.id, archive).await?)
        }
        Err(error) => Err(error.into()),
    }
}

async fn rename_to_free_archive_name<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    id: &NodeId,
    original: &NodeName,
    now: protonsync_domain::time::Timestamp,
) -> Result<Node, StepError>
where
    R: RemoteStorage,
{
    // Attempt 0 has no counter; the counter starts at 2 like "report (2).pdf".
    for attempt in std::iter::once(0).chain(2..MAX_ARCHIVE_ATTEMPTS) {
        let name = archived_name(original.as_str(), now, attempt)
            .and_then(|name| NodeName::new(name).ok())
            .ok_or(StepError::Outdated(
                "name of the archived version is invalid",
            ))?;
        match context.remote.rename(id, &name).await {
            Err(RemoteError::AlreadyExists(_)) => {}
            other => return Ok(other?),
        }
    }
    Err(StepError::Outdated("no free name in the archive folder"))
}

/// Conflict: always keep both versions and queue the conflict.
async fn conflict<R, L, S, C>(
    context: &SyncContext<'_, R, L, S, C>,
    plan: &mut SyncPlan,
    view: &mut RemoteView<'_>,
    step: &PlannedStep,
) -> Result<(), StepError>
where
    R: RemoteStorage,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    let kind = step
        .conflict
        .ok_or(StepError::Outdated("conflict kind unknown"))?;
    let now = context.clock.now();
    let conflict_copy = match kind {
        ConflictKind::BothModified | ConflictKind::LocalModifiedRemoteProtected => {
            // Set the local version aside, then download the remote one into its place.
            ensure_local_unchanged(context.local, step)?;
            let copy_name = NodeName::new(conflict_copy_name(file_name(&step.path)?.as_str(), now))
                .map_err(|_| StepError::Outdated("name of the conflict copy is too long"))?;
            let copy_path = step.path.parent().unwrap_or_default().join(&copy_name);
            context.local.rename(&step.path, &copy_path)?;
            let without_local = PlannedStep {
                local: None,
                ..step.clone()
            };
            download(context, plan, &without_local).await?;
            Some(copy_path)
        }
        // The file deleted remotely is recreated with the local version.
        ConflictKind::LocalModifiedRemoteDeleted if context.pair.direction.allows_upload() => {
            let without_remote = PlannedStep {
                remote: None,
                ..step.clone()
            };
            upload(context, plan, view, &without_remote).await?;
            None
        }
        // Without upload (`download_only`) nothing is recreated in Proton:
        // the local version stays as an untracked local file.
        ConflictKind::LocalModifiedRemoteDeleted => {
            context
                .state
                .remove_base_tree(&context.pair.id, &step.path)?;
            None
        }
        // The file deleted locally is recreated with the remote version.
        ConflictKind::LocalDeletedRemoteModified => {
            download(context, plan, step).await?;
            None
        }
    };
    let conflict = Conflict {
        relative_path: step.path.clone(),
        kind,
        conflict_copy,
        detected_at: now,
    };
    context.state.add_conflict(&context.pair.id, &conflict)?;
    Ok(())
}
