//! Ausführung eines Plans.
//!
//! Jeder Schritt schreibt die Basis erst, nachdem er vollständig gelungen ist.
//! Schlägt ein Schritt fehl, wird er im Bericht vermerkt und der nächste Lauf
//! versucht ihn erneut. Nur fehlende Anmeldung bricht den ganzen Lauf ab.

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

/// Fehler eines einzelnen Schritts.
#[derive(Debug)]
enum StepError {
    Remote(RemoteError),
    Local(LocalError),
    State(crate::ports::StateError),
    /// Der Schritt passt nicht mehr zum aktuellen Stand und wird beim nächsten Lauf neu geplant.
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

/// Wie die Ausführung mit dem zwischengespeicherten entfernten Baum umgeht.
pub(crate) struct RemoteView<'c> {
    /// Eigene Änderungen in Proton werden hier sofort nachgetragen.
    pub cache: &'c mut RemoteCache,
    /// Vor lokalem Löschen prüfen, ob der Knoten in Proton wirklich fehlt.
    /// Nötig, wenn der Baum aus Ereignissen stammt statt frisch gelesen.
    pub verify_deletions: bool,
}

/// Führt alle Schritte aus und zählt die Ergebnisse.
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
    }
}

fn file_name(path: &RelativePath) -> Result<NodeName, StepError> {
    path.file_name().ok_or(StepError::Outdated(
        "Wurzelordner kann nicht abgeglichen werden",
    ))
}

/// Kennung des entfernten Elternordners von `path`.
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
            .ok_or(StepError::Outdated("Elternordner fehlt in Proton")),
        None => Err(StepError::Outdated("Wurzelordner hat keinen Elternordner")),
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

/// Verhindert, dass eine lokale Änderung nach dem Scan überschrieben oder gelöscht wird.
fn ensure_local_unchanged<L: LocalStorage>(local: &L, step: &PlannedStep) -> Result<(), StepError> {
    let current = local.entry(&step.path)?.map(|entry| entry.kind);
    if current == step.local {
        Ok(())
    } else {
        Err(StepError::Outdated("lokal während des Laufs geändert"))
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
        (None, _) => return Err(StepError::Outdated("lokale Datei fehlt")),
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

/// Lädt eine lokale Datei hoch: als neue Revision, wenn es sie entfernt schon gibt.
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
        .ok_or(StepError::Outdated("entfernte Datei fehlt"))?;
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

/// Lädt eine entfernte Datei nach `path`; erst am Ende wird sie sichtbar.
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
        .ok_or(StepError::Outdated("entfernter Knoten fehlt"))?;
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

/// Lokal gelöscht wird nur, was in Proton nachweislich fehlt. Ist der Knoten
/// doch noch da, war der zwischengespeicherte Baum veraltet: Er wird verworfen,
/// und der nächste (vollständige) Lauf entscheidet neu.
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
                "in Proton noch vorhanden; der nächste vollständige Abgleich prüft neu",
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

/// Konflikt: immer beide Versionen behalten und den Konflikt vormerken.
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
        .ok_or(StepError::Outdated("Konfliktart unbekannt"))?;
    let now = context.clock.now();
    let conflict_copy = match kind {
        ConflictKind::BothModified => {
            // Lokale Version beiseitelegen, dann die entfernte an ihren Platz laden.
            ensure_local_unchanged(context.local, step)?;
            let copy_name = NodeName::new(conflict_copy_name(file_name(&step.path)?.as_str(), now))
                .map_err(|_| StepError::Outdated("Name der Konfliktkopie ist zu lang"))?;
            let copy_path = step.path.parent().unwrap_or_default().join(&copy_name);
            context.local.rename(&step.path, &copy_path)?;
            let without_local = PlannedStep {
                local: None,
                ..step.clone()
            };
            download(context, plan, &without_local).await?;
            Some(copy_path)
        }
        // Die entfernt gelöschte Datei wird mit der lokalen Version neu angelegt.
        ConflictKind::LocalModifiedRemoteDeleted => {
            let without_remote = PlannedStep {
                remote: None,
                ..step.clone()
            };
            upload(context, plan, view, &without_remote).await?;
            None
        }
        // Die lokal gelöschte Datei wird mit der entfernten Version neu angelegt.
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
