//! Ausführung des Foto-Plans. Jeder Schritt prüft zuerst, ob der Stand noch
//! der beim Planen ist; die Basis wird erst nach Erfolg geschrieben.

use protonsync_domain::node::{Fingerprint, NodeId};
use protonsync_domain::path::RelativePath;
use protonsync_domain::photo::Photo;
use protonsync_domain::photo::layout::media_type;
use protonsync_domain::trash::{TrashEntry, TrashOrigin};

use super::plan::{PhotoPlan, PhotoStep, PhotoStepKind};
use super::{PhotoIndex, PhotoSync, photos_owner};
use crate::ports::{
    BaseEntry, Clock, LocalError, LocalKind, LocalStorage, NewPhoto, PhotoLibrary, RemoteError,
    StateError, StateRepo,
};
use crate::sync::SkippedPath;

/// Ein fehlgeschlagener Schritt; er wird beim nächsten Lauf erneut versucht.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoFailure {
    /// Betroffener Pfad.
    pub path: RelativePath,
    /// Fehlermeldung.
    pub message: String,
}

/// Ergebnis eines Laufs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhotoReport {
    /// Heruntergeladene Fotos.
    pub downloaded: usize,
    /// Hochgeladene Fotos.
    pub uploaded: usize,
    /// Nur verknüpft, weil der Inhalt auf beiden Seiten gleich ist.
    pub linked: usize,
    /// In den lokalen Papierkorb gelegt.
    pub trashed_local: usize,
    /// In den Proton-Papierkorb gelegt.
    pub trashed_remote: usize,
    /// Wegen der Richtung nicht übertragene Änderungen.
    pub ignored: usize,
    /// Übersprungene Pfade.
    pub skipped: Vec<SkippedPath>,
    /// Fehlgeschlagene Schritte.
    pub failures: Vec<PhotoFailure>,
}

/// Warum ein einzelner Schritt nicht ausgeführt wurde.
#[derive(Debug)]
enum StepError {
    Remote(RemoteError),
    Local(LocalError),
    State(StateError),
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

impl From<StateError> for StepError {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}

impl std::fmt::Display for StepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Remote(error) => error.fmt(f),
            Self::Local(error) => error.fmt(f),
            Self::State(error) => error.fmt(f),
            Self::Outdated(reason) => f.write_str(reason),
        }
    }
}

pub(super) async fn apply<P, L, S, C>(
    sync: &PhotoSync<'_, P, L, S, C>,
    index: &mut PhotoIndex,
    plan: PhotoPlan,
) -> PhotoReport
where
    P: PhotoLibrary,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    let mut report = PhotoReport {
        ignored: plan.ignored,
        skipped: plan.skipped,
        ..PhotoReport::default()
    };
    for step in plan.steps {
        match execute(sync, index, &step).await {
            Ok(()) => count(&mut report, &step.kind),
            Err(error) => report.failures.push(PhotoFailure {
                path: step.path.clone(),
                message: error.to_string(),
            }),
        }
    }
    report
}

fn count(report: &mut PhotoReport, kind: &PhotoStepKind) {
    match kind {
        PhotoStepKind::Download(_) => report.downloaded += 1,
        PhotoStepKind::Link(_) => report.linked += 1,
        PhotoStepKind::Upload => report.uploaded += 1,
        PhotoStepKind::TrashLocal(_) => report.trashed_local += 1,
        PhotoStepKind::TrashRemote(_) => report.trashed_remote += 1,
        PhotoStepKind::Forget => {}
    }
}

async fn execute<P, L, S, C>(
    sync: &PhotoSync<'_, P, L, S, C>,
    index: &mut PhotoIndex,
    step: &PhotoStep,
) -> Result<(), StepError>
where
    P: PhotoLibrary,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    let owner = photos_owner();
    match &step.kind {
        PhotoStepKind::Download(photo) => {
            ensure_local_unchanged(sync.local, step)?;
            create_parents(sync.local, &step.path)?;
            let mut staging = sync.local.stage(&step.path)?;
            sync.library.download_to(&photo.id, &mut staging).await?;
            // Die Änderungszeit der Datei ist die Aufnahmezeit, wie bei Kamera-Dateien.
            let fingerprint = sync.local.commit(staging, &step.path, photo.captured)?;
            sync.state
                .save_base(&owner, &base(&step.path, photo, fingerprint))?;
        }
        PhotoStepKind::Link(photo) => {
            ensure_local_unchanged(sync.local, step)?;
            let fingerprint = step
                .local
                .clone()
                .ok_or(StepError::Outdated("Datei fehlt"))?;
            sync.state
                .save_base(&owner, &base(&step.path, photo, fingerprint))?;
        }
        PhotoStepKind::Upload => {
            let photo = upload(sync, step).await?;
            let fingerprint = step
                .local
                .clone()
                .ok_or(StepError::Outdated("Datei fehlt"))?;
            sync.state
                .save_base(&owner, &base(&step.path, &photo, fingerprint))?;
            index.insert(photo);
        }
        PhotoStepKind::TrashLocal(id) => {
            ensure_photo_gone(sync.library, index, id).await?;
            ensure_local_unchanged(sync.local, step)?;
            let stored_as = sync.local.move_to_trash(&step.path)?;
            record_trash(sync, &step.path, TrashOrigin::Local { stored_as })?;
        }
        PhotoStepKind::TrashRemote(photo) => {
            sync.library.trash(&photo.id).await?;
            index.remove(&photo.id);
            let origin = TrashOrigin::Remote {
                node: photo.id.clone(),
            };
            record_trash(sync, &step.path, origin)?;
        }
        PhotoStepKind::Forget => sync.state.remove_base_tree(&owner, &step.path)?,
    }
    Ok(())
}

async fn upload<P, L, S, C>(
    sync: &PhotoSync<'_, P, L, S, C>,
    step: &PhotoStep,
) -> Result<Photo, StepError>
where
    P: PhotoLibrary,
    L: LocalStorage,
{
    ensure_local_unchanged(sync.local, step)?;
    let fingerprint = step
        .local
        .as_ref()
        .ok_or(StepError::Outdated("Datei fehlt"))?;
    let name = step
        .path
        .file_name()
        .ok_or(StepError::Outdated("kein Dateiname"))?;
    let media_type = media_type(&name).ok_or(StepError::Outdated("keine Mediendatei"))?;
    let new = NewPhoto {
        name,
        media_type: media_type.to_owned(),
        size: fingerprint.size,
        // Vorerst die Änderungszeit; EXIF folgt später (ADR 0012).
        captured: fingerprint.modified,
    };
    let content = sync.local.open(&step.path)?;
    Ok(sync.library.upload(&new, content).await?)
}

fn record_trash<P, L, S, C>(
    sync: &PhotoSync<'_, P, L, S, C>,
    path: &RelativePath,
    origin: TrashOrigin,
) -> Result<(), StepError>
where
    S: StateRepo,
    C: Clock,
{
    let owner = photos_owner();
    let entry = TrashEntry {
        original_path: path.clone(),
        origin,
        trashed_at: sync.clock.now(),
    };
    sync.state.add_trash(&owner, &entry)?;
    sync.state.remove_base_tree(&owner, path)?;
    Ok(())
}

fn base(path: &RelativePath, photo: &Photo, local: Fingerprint) -> BaseEntry {
    BaseEntry {
        path: path.clone(),
        remote_id: photo.id.clone(),
        remote_revision: Some(photo.revision.clone()),
        local: Some(local),
    }
}

/// Verhindert, dass eine lokale Änderung nach dem Planen überschrieben oder gelöscht wird.
fn ensure_local_unchanged<L: LocalStorage>(local: &L, step: &PhotoStep) -> Result<(), StepError> {
    let current = match local.entry(&step.path)?.map(|entry| entry.kind) {
        Some(LocalKind::File(fingerprint)) => Some(fingerprint),
        Some(LocalKind::Folder) => return Err(StepError::Outdated("Ordner im Weg")),
        None => None,
    };
    let unchanged = match (&current, &step.local) {
        (None, None) => true,
        (Some(current), Some(planned)) => {
            current.size == planned.size && current.modified == planned.modified
        }
        _ => false,
    };
    if unchanged {
        Ok(())
    } else {
        Err(StepError::Outdated("lokal während des Laufs geändert"))
    }
}

/// Lokal gelöscht wird nur, was in Proton nachweislich fehlt. Ist das Foto
/// doch noch da, war der Index veraltet; der nächste Lauf liest alles neu.
async fn ensure_photo_gone<P: PhotoLibrary>(
    library: &P,
    index: &mut PhotoIndex,
    id: &NodeId,
) -> Result<(), StepError> {
    match library.photo(id).await {
        Err(RemoteError::NotFound(_)) => Ok(()),
        Err(error) => Err(error.into()),
        Ok(_) => {
            index.invalidate();
            Err(StepError::Outdated(
                "in Proton noch vorhanden; der nächste Lauf liest die Bibliothek neu",
            ))
        }
    }
}

/// Legt fehlende Monatsordner an.
fn create_parents<L: LocalStorage>(local: &L, path: &RelativePath) -> Result<(), LocalError> {
    let mut missing = Vec::new();
    let mut parent = path.parent();
    while let Some(folder) = parent.filter(|folder| !folder.is_root()) {
        if local.entry(&folder)?.is_none() {
            missing.push(folder.clone());
        }
        parent = folder.parent();
    }
    for folder in missing.iter().rev() {
        local.create_folder(folder)?;
    }
    Ok(())
}
