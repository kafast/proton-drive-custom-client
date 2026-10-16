//! Zustand einzelner Pfade für Dateimanager (ADR 0013). Alles kommt aus dem
//! lokalen Zustand; Proton wird nie gefragt, damit das Anzeigen eines Ordners
//! schnell bleibt und kein Netz braucht.

use std::path::Path;

use protonsync_api::{ApiError, ErrorCode, PathState, PathStatusInfo, Request, Response};
use protonsync_application::drive::DriveContent;
use protonsync_application::ports::{Account, Clock, LocalKind, LocalStorage};
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncPairId;

use super::mounts::relative_to;
use super::{Core, DaemonState};
use crate::convert;

/// Interne Ordner des lokalen Speichers (Papierkorb, Zwischenablage) beginnen so.
const INTERNAL_PREFIX: &str = ".protonsync";

impl<A, L, S, C> Core<A, L, S, C>
where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: DaemonState,
    C: Clock + 'static,
{
    /// Beantwortet die Anfragen zu Laufwerken und Pfaden.
    pub(super) async fn dispatch_files(&self, request: Request) -> Result<Response, ApiError> {
        match request {
            Request::Mounts => Ok(Response::Mounts {
                mounts: self.mount_infos().await?,
            }),
            Request::Pin { path, state } => {
                self.pin(&path, state).await?;
                Ok(Response::Done)
            }
            Request::Free { path, apply } => Ok(Response::Freed {
                results: self.free(path.as_deref(), !apply).await?,
            }),
            Request::PathStatus { paths } => Ok(Response::PathStatus {
                entries: self.path_status(&paths).await?,
            }),
            _ => Err(ApiError::new(
                ErrorCode::InvalidRequest,
                "Keine Anfrage zu Laufwerken",
            )),
        }
    }

    /// Zustand der Pfade, in der Reihenfolge der Anfrage.
    pub(super) async fn path_status(
        &self,
        paths: &[String],
    ) -> Result<Vec<PathStatusInfo>, ApiError> {
        let mut entries = Vec::with_capacity(paths.len());
        for path in paths {
            let (state, in_drive, pinned) = self.one_path_status(Path::new(path)).await?;
            entries.push(PathStatusInfo {
                path: path.clone(),
                state,
                in_drive,
                pinned,
            });
        }
        Ok(entries)
    }

    /// Zustand, ob im Laufwerk, ob angeheftet.
    async fn one_path_status(&self, path: &Path) -> Result<(PathState, bool, bool), ApiError> {
        for (index, mount) in self.mounts.iter().enumerate() {
            let Some(relative) = relative_to(&mount.config.mountpoint, path) else {
                continue;
            };
            // Ohne Anmeldung ist das Laufwerk nicht eingehängt: nur Platzhalter.
            let Some(drive) = self.drive(index).await else {
                return Ok((PathState::OnlineOnly, true, false));
            };
            let status = drive
                .path_status(&relative)
                .map_err(|e| convert::drive_error(&e))?;
            return Ok((drive_state(status.content), true, status.pinned));
        }
        let locals = self
            .pairs
            .iter()
            .map(|pair| (pair.config.local.as_path(), &pair.local, &pair.id))
            .chain(
                self.photos
                    .iter()
                    .map(|photos| (photos.config.folder.as_path(), &photos.local, &photos.owner)),
            );
        for (root, local, owner) in locals {
            if let Some(relative) = relative_to(root, path) {
                return Ok((self.local_state(local, owner, &relative)?, false, false));
            }
        }
        Ok((PathState::Outside, false, false))
    }

    /// Zustand in einem Sync- oder Fotoordner: verglichen mit dem Stand des
    /// letzten Abgleichs.
    fn local_state(
        &self,
        local: &L,
        owner: &SyncPairId,
        path: &RelativePath,
    ) -> Result<PathState, ApiError> {
        if path.is_root() {
            return Ok(PathState::Folder);
        }
        if path.as_str().starts_with(INTERNAL_PREFIX) {
            return Ok(PathState::Outside);
        }
        let conflicts = self
            .state
            .conflicts(owner)
            .map_err(|e| convert::state_api_error(&e))?;
        // Konfliktkopie und Original gehören beide zum offenen Konflikt.
        if conflicts.iter().any(|(_, conflict)| {
            &conflict.relative_path == path || conflict.conflict_copy.as_ref() == Some(path)
        }) {
            return Ok(PathState::Conflict);
        }
        let current = match local.entry(path) {
            Ok(Some(entry)) => entry.kind,
            // Gelöscht oder nicht lesbar: Der nächste Abgleich kümmert sich darum.
            Ok(None) | Err(_) => return Ok(PathState::Pending),
        };
        let LocalKind::File(current) = current else {
            return Ok(PathState::Folder);
        };
        let base = self
            .state
            .base_entry(owner, path)
            .map_err(|e| convert::state_api_error(&e))?;
        let unchanged = base
            .and_then(|base| base.local)
            .is_some_and(|known| known.size == current.size && known.modified == current.modified);
        Ok(if unchanged {
            PathState::Synced
        } else {
            PathState::Pending
        })
    }
}

fn drive_state(content: DriveContent) -> PathState {
    match content {
        DriveContent::Folder => PathState::Folder,
        DriveContent::OnlineOnly => PathState::OnlineOnly,
        DriveContent::Partial => PathState::Partial,
        DriveContent::Local => PathState::Local,
        DriveContent::PendingUpload => PathState::PendingUpload,
    }
}
