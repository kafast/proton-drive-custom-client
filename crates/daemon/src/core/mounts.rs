//! FUSE drives in the daemon core: connect, mount, maintain, pin, free
//! (placeholder mode, ADR 0010).
//!
//! A drive only exists while the user is logged in: after login the core
//! builds the virtual drive and has the [`DriveMounter`] mount it; on logout
//! it is unmounted again. The core itself does not know FUSE; mounting is
//! replaceable (FUSE in the daemon, a fake in tests).

use std::any::Any;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use protonsync_adapter_fs::{FsContentCache, FsLocal};
use protonsync_api::{ApiError, CacheInfo, ErrorCode, FreeResult, MountInfo, PinChoice, PinInfo};
use protonsync_application::drive::{DriveSettings, VirtualDrive};
use protonsync_application::ports::{
    Account, AccountError, Clock, LocalStorage, RemoteStorage, TrashId,
};
use protonsync_application::use_cases::TrashService;
use protonsync_domain::path::RelativePath;
use protonsync_domain::pin::{EvictionPolicy, PinState};
use protonsync_domain::sync_pair::SyncPairId;
use tokio::sync::RwLock;

use super::{Core, DaemonState, not_logged_in};
use crate::convert;
use crate::mount_config::MountConfig;

/// The virtual drive as the core builds it.
pub type DaemonDrive<R, S, C> = VirtualDrive<Arc<R>, FsContentCache, Arc<S>, Arc<S>, Arc<C>>;

/// Keeps a drive mounted; when it is dropped, the drive is unmounted.
pub type MountGuard = Box<dyn Any + Send + Sync>;

/// Mounts a drive into the file system (via FUSE in the daemon).
pub trait DriveMounter<D>: Send + Sync {
    /// Mounts `drive` at `mountpoint`.
    ///
    /// # Errors
    /// An error message if mounting fails (e.g. FUSE is missing).
    fn mount(&self, mountpoint: &Path, drive: Arc<D>) -> Result<MountGuard, String>;
}

/// A shared [`DriveMounter`] for the drives of the core.
pub type SharedMounter<R, S, C> = Arc<dyn DriveMounter<DaemonDrive<R, S, C>>>;

/// A drive during wiring.
pub struct MountSetup {
    /// Configuration.
    pub config: MountConfig,
    /// Cache (folder in the data directory).
    pub cache: FsContentCache,
}

/// Connected drive (only when logged in).
struct MountConnection<R, S, C> {
    remote: Arc<R>,
    drive: Arc<DaemonDrive<R, S, C>>,
    /// `None` if mounting failed; pinning and freeing still work.
    guard: Option<MountGuard>,
}

pub(super) struct MountRuntime<R, S, C> {
    pub(super) config: MountConfig,
    /// Identifier in the state and owner in the trash, e.g. `mount:home`.
    pub(super) owner: SyncPairId,
    cache: FsContentCache,
    /// Only for trash entries with local content; a drive has none of those
    /// (deleted items are in the Proton trash).
    local: FsLocal,
    connection: RwLock<Option<MountConnection<R, S, C>>>,
}

impl<R, S, C> MountRuntime<R, S, C> {
    pub(super) fn new(setup: MountSetup) -> Self {
        Self {
            owner: SyncPairId::new(format!(
                "{}{}",
                protonsync_api::MOUNT_TRASH_PREFIX,
                setup.config.id
            )),
            local: FsLocal::new_unchecked(&setup.config.mountpoint),
            config: setup.config,
            cache: setup.cache,
            connection: RwLock::new(None),
        }
    }
}

fn unknown_mount_path(path: &Path) -> ApiError {
    ApiError::new(
        ErrorCode::NotFound,
        format!("{} is not in any drive", path.display()),
    )
}

/// Path relative to the mount point (`.` and `..` are not allowed).
pub(super) fn relative_to(mountpoint: &Path, path: &Path) -> Option<RelativePath> {
    let rest = path.strip_prefix(mountpoint).ok()?;
    let mut segments = Vec::new();
    for component in rest.components() {
        match component {
            Component::Normal(segment) => segments.push(segment.to_str()?.to_owned()),
            _ => return None,
        }
    }
    if segments.is_empty() {
        return Some(RelativePath::root());
    }
    RelativePath::parse(&segments.join("/")).ok()
}

impl<A, L, S, C> Core<A, L, S, C>
where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: DaemonState,
    C: Clock + 'static,
{
    /// Adds FUSE drives. Without `mounter` they are only connected
    /// (pinning and freeing work, nothing is mounted).
    #[must_use]
    pub fn with_mounts(
        mut self,
        mounts: Vec<MountSetup>,
        policy: EvictionPolicy,
        mounter: Option<SharedMounter<A::Remote, S, C>>,
    ) -> Self {
        self.mounts = mounts.into_iter().map(MountRuntime::new).collect();
        self.eviction = Some(policy);
        self.mounter = mounter;
        self
    }

    /// Number of drives (indices `0..mount_count()`).
    #[must_use]
    pub fn mount_count(&self) -> usize {
        self.mounts.len()
    }

    /// Connects all drives to Proton and mounts them.
    pub(super) async fn connect_mounts(&self) {
        for mount in &self.mounts {
            let mut connection = mount.connection.write().await;
            // A drive that is already mounted stays as it is.
            if connection.is_some() {
                continue;
            }
            match self.connect_mount(mount).await {
                Ok(connected) => *connection = Some(connected),
                Err(error) => {
                    tracing::warn!(mount = %mount.config.id, %error, "drive not connected");
                }
            }
        }
    }

    async fn connect_mount(
        &self,
        mount: &MountRuntime<A::Remote, S, C>,
    ) -> Result<MountConnection<A::Remote, S, C>, AccountError> {
        let remote = Arc::new(self.account.open_remote(&mount.config.remote).await?);
        let root = remote.root_folder().await?;
        let settings = DriveSettings {
            id: mount.owner.clone(),
            default_pin: if mount.config.keep_local {
                PinState::Pinned
            } else {
                PinState::Unpinned
            },
        };
        let drive = VirtualDrive::new(
            Arc::clone(&remote),
            mount.cache.clone(),
            Arc::clone(&self.state),
            Arc::clone(&self.state),
            Arc::clone(&self.clock),
            settings,
            &root,
        )
        .map_err(|error| AccountError::Unavailable(error.to_string()))?;
        let drive = Arc::new(drive);
        let guard = self.mounter.as_ref().and_then(|mounter| {
            match mounter.mount(&mount.config.mountpoint, Arc::clone(&drive)) {
                Ok(guard) => {
                    tracing::info!(
                        mount = %mount.config.id,
                        at = %mount.config.mountpoint.display(),
                        "drive mounted"
                    );
                    Some(guard)
                }
                Err(message) => {
                    tracing::error!(mount = %mount.config.id, %message, "drive not mounted");
                    None
                }
            }
        });
        Ok(MountConnection {
            remote,
            drive,
            guard,
        })
    }

    /// Unmounts all drives (on logout and shutdown).
    pub async fn disconnect_mounts(&self) {
        for mount in &self.mounts {
            // Dropping unmounts.
            *mount.connection.write().await = None;
        }
    }

    pub(super) async fn drive(&self, index: usize) -> Option<Arc<DaemonDrive<A::Remote, S, C>>> {
        let mount = self.mounts.get(index)?;
        let connection = mount.connection.read().await;
        connection.as_ref().map(|c| Arc::clone(&c.drive))
    }

    pub(super) async fn mount_remote(&self, index: usize) -> Option<Arc<A::Remote>> {
        let mount = self.mounts.get(index)?;
        let connection = mount.connection.read().await;
        connection.as_ref().map(|c| Arc::clone(&c.remote))
    }

    /// Drive and relative path for an absolute path.
    fn locate(&self, path: &str) -> Result<(usize, RelativePath), ApiError> {
        let path = PathBuf::from(path);
        self.mounts
            .iter()
            .enumerate()
            .find_map(|(index, mount)| {
                relative_to(&mount.config.mountpoint, &path).map(|relative| (index, relative))
            })
            .ok_or_else(|| unknown_mount_path(&path))
    }

    pub(super) async fn mount_infos(&self) -> Result<Vec<MountInfo>, ApiError> {
        let mut infos = Vec::with_capacity(self.mounts.len());
        for mount in &self.mounts {
            let connection = mount.connection.read().await;
            let cache = match connection.as_ref() {
                Some(connected) => {
                    let usage = connected
                        .drive
                        .usage()
                        .map_err(|e| convert::drive_error(&e))?;
                    Some(CacheInfo {
                        files: u64::try_from(usage.files).unwrap_or(u64::MAX),
                        bytes: usage.bytes,
                        pending_uploads: u64::try_from(usage.pending_uploads).unwrap_or(u64::MAX),
                    })
                }
                None => None,
            };
            let pins = self
                .state
                .pin_rules(&mount.owner)
                .map_err(|e| convert::state_api_error(&e))?
                .iter()
                .map(|(path, state)| PinInfo {
                    path: path.as_str().to_owned(),
                    state: convert::pin_choice(state),
                })
                .collect();
            infos.push(MountInfo {
                id: mount.config.id.clone(),
                mountpoint: mount.config.mountpoint.display().to_string(),
                remote_path: mount.config.remote.as_str().to_owned(),
                mounted: connection.as_ref().is_some_and(|c| c.guard.is_some()),
                keep_local: mount.config.keep_local,
                cache,
                pins,
            });
        }
        Ok(infos)
    }

    /// Sets "always keep local" or "on demand only" for a path.
    /// Pinned content is loaded right away in the background.
    pub(super) async fn pin(&self, path: &str, choice: PinChoice) -> Result<(), ApiError> {
        let (index, relative) = self.locate(path)?;
        let drive = self.drive(index).await.ok_or_else(not_logged_in)?;
        // Report typos early: the path must exist in the drive.
        drive
            .resolve(&relative)
            .await
            .map_err(|e| convert::drive_error(&e))?;
        drive
            .set_pin(&relative, Some(convert::pin_state(choice)))
            .map_err(|e| convert::drive_error(&e))?;
        if choice == PinChoice::Pinned {
            tokio::spawn(async move {
                if let Err(error) = drive.prefetch_pinned().await {
                    tracing::warn!(%error, "pinned content not fully loaded");
                }
            });
        }
        Ok(())
    }

    /// Frees space: with `path` everything below it right away, otherwise
    /// according to the eviction policy on all connected drives.
    pub(super) async fn free(
        &self,
        path: Option<&str>,
        dry_run: bool,
    ) -> Result<Vec<FreeResult>, ApiError> {
        if let Some(path) = path {
            let (index, relative) = self.locate(path)?;
            let drive = self.drive(index).await.ok_or_else(not_logged_in)?;
            let report = drive
                .free(&relative, dry_run)
                .await
                .map_err(|e| convert::drive_error(&e))?;
            let id = &self.mounts[index].config.id;
            return Ok(vec![convert::free_result(id, dry_run, &report)]);
        }
        let mut results = Vec::new();
        let Some(policy) = self.eviction else {
            return Ok(results);
        };
        for (index, mount) in self.mounts.iter().enumerate() {
            let Some(drive) = self.drive(index).await else {
                continue;
            };
            let report = drive
                .evict(policy, dry_run)
                .await
                .map_err(|e| convert::drive_error(&e))?;
            results.push(convert::free_result(&mount.config.id, dry_run, &report));
        }
        Ok(results)
    }

    /// Regular maintenance of a drive: apply Proton events, retry pending
    /// uploads, load pinned content.
    pub async fn maintain_mount(&self, index: usize) {
        let (Some(mount), Some(drive)) = (self.mounts.get(index), self.drive(index).await) else {
            return;
        };
        let id = &mount.config.id;
        if let Err(error) = drive.refresh().await {
            tracing::warn!(mount = %id, %error, "events not applied");
        }
        for (path, error) in drive.upload_pending().await {
            tracing::warn!(mount = %id, %path, %error, "upload still pending");
        }
        match drive.prefetch_pinned().await {
            Ok(report) if report.files > 0 => {
                tracing::info!(mount = %id, files = report.files, bytes = report.bytes, "pinned content loaded");
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(mount = %id, %error, "pinned content not loaded"),
        }
    }

    /// Eviction from the schedule; respects the dry-run setting. In a dry run
    /// what would be freed is only logged quietly.
    pub async fn scheduled_eviction(&self) {
        let dry_run = self.settings.dry_run;
        match self.free(None, dry_run).await {
            Ok(results) => {
                for result in results.iter().filter(|r| !r.files.is_empty()) {
                    if dry_run {
                        tracing::debug!(
                            mount = %result.mount,
                            files = result.files.len(),
                            bytes = result.bytes,
                            "space would be freed (dry run)"
                        );
                    } else {
                        tracing::info!(
                            mount = %result.mount,
                            files = result.files.len(),
                            bytes = result.bytes,
                            "space freed"
                        );
                    }
                }
            }
            Err(error) => tracing::warn!(error = %error.message, "space not freed"),
        }
    }

    /// Restores a trash entry of a drive.
    pub(super) async fn restore_mount_trash(
        &self,
        index: usize,
        id: TrashId,
    ) -> Result<(), ApiError> {
        let mount = self.mounts.get(index).ok_or_else(not_logged_in)?;
        let remote = self.mount_remote(index).await.ok_or_else(not_logged_in)?;
        self.trash_service(mount, &remote)
            .restore(id)
            .await
            .map_err(|e| convert::use_case_error(&e))
    }

    /// Permanently deletes expired trash entries of a drive.
    pub(super) async fn purge_mount_trash(
        &self,
        index: usize,
        dry_run: bool,
    ) -> Result<Vec<protonsync_api::TrashInfo>, ApiError> {
        let (Some(mount), Some(remote)) = (self.mounts.get(index), self.mount_remote(index).await)
        else {
            return Ok(Vec::new());
        };
        let entries = self
            .trash_service(mount, &remote)
            .purge_expired(self.settings.retention, dry_run)
            .await
            .map_err(|e| convert::use_case_error(&e))?;
        Ok(entries
            .iter()
            .map(|(id, entry)| {
                convert::trash(mount.owner.as_str(), *id, entry, self.settings.retention)
            })
            .collect())
    }

    fn trash_service<'a>(
        &'a self,
        mount: &'a MountRuntime<A::Remote, S, C>,
        remote: &'a Arc<A::Remote>,
    ) -> TrashService<'a, Arc<A::Remote>, FsLocal, Arc<S>, Arc<C>> {
        TrashService {
            remote,
            local: &mount.local,
            state: &self.state,
            clock: &self.clock,
            pair: &mount.owner,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_made_relative_to_the_mountpoint() {
        let at = Path::new("/home/k/Drive");
        let relative = |path: &str| relative_to(at, Path::new(path)).map(|p| p.as_str().to_owned());
        assert_eq!(relative("/home/k/Drive"), Some(String::new()));
        assert_eq!(
            relative("/home/k/Drive/Photos/2026"),
            Some("Photos/2026".into())
        );
        assert_eq!(relative("/home/k/Drive/Photos/"), Some("Photos".into()));
        assert_eq!(relative("/home/k/Drive/../x"), None);
        assert_eq!(relative("/home/k/Other"), None);
        assert_eq!(relative("/home/k/DriveX"), None);
    }
}
