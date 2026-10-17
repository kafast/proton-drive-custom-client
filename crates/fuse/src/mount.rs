//! Mounting and unmounting the drive.

use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use fuser::{BackgroundSession, Config, MountOption};
use protonsync_application::drive::VirtualDrive;
use protonsync_application::ports::{Clock, ContentCache, DriveRepo, RemoteStorage, StateRepo};
use tokio::runtime::Handle;

use crate::filesystem::DriveFilesystem;
use crate::ops::{DriveOps, Owner};

/// Name in `mount` and `df`.
const FILESYSTEM_NAME: &str = "protonsync";
/// Threads that accept kernel requests. The actual work runs in tokio; more
/// threads only help with accepting requests quickly.
const REQUEST_THREADS: usize = 2;

/// Error while mounting or unmounting.
#[derive(Debug, thiserror::Error)]
pub enum MountError {
    /// The mount point is missing or is not a folder.
    #[error("mount point {0} is not a folder")]
    NotAFolder(PathBuf),
    /// The kernel or `fusermount3` refused (is the package `fuse3` missing, for example?).
    #[error("FUSE error at {path}: {source}")]
    Fuse {
        /// Mount point.
        path: PathBuf,
        /// Cause.
        source: io::Error,
    },
}

/// A mounted drive. Dropping it unmounts the drive.
pub struct MountHandle {
    session: BackgroundSession,
    mountpoint: PathBuf,
}

impl std::fmt::Debug for MountHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MountHandle")
            .field("mountpoint", &self.mountpoint)
            .finish_non_exhaustive()
    }
}

impl MountHandle {
    /// The mount point.
    #[must_use]
    pub fn mountpoint(&self) -> &Path {
        &self.mountpoint
    }

    /// Unmounts and waits until all requests have finished.
    ///
    /// # Errors
    /// [`MountError::Fuse`] if unmounting fails (e.g. because a program is
    /// still working in the drive).
    pub fn unmount(self) -> Result<(), MountError> {
        let path = self.mountpoint;
        self.session
            .umount_and_join()
            .map_err(|source| MountError::Fuse { path, source })
    }
}

/// The mount options: only the own user, no device or SUID files, the kernel
/// checks permissions based on the attributes.
fn config() -> Config {
    let mut config = Config::default();
    config.mount_options = vec![
        MountOption::FSName(FILESYSTEM_NAME.into()),
        MountOption::Subtype(FILESYSTEM_NAME.into()),
        MountOption::NoDev,
        MountOption::NoSuid,
        MountOption::NoAtime,
        MountOption::DefaultPermissions,
    ];
    config.n_threads = Some(REQUEST_THREADS);
    config
}

/// Cleans up a stale mount (the daemon crashed, the kernel reports
/// "Transport endpoint is not connected"). If that fails, the subsequent mount
/// reports the error.
fn clear_stale_mount(mountpoint: &Path) {
    let stale = matches!(
        std::fs::metadata(mountpoint),
        Err(error) if error.raw_os_error() == Some(libc::ENOTCONN)
    );
    if !stale {
        return;
    }
    tracing::warn!(at = %mountpoint.display(), "stale mount, unmounting");
    // `fusermount3` for normal users, `umount` as root (without fusermount3).
    for (program, args) in [("fusermount3", ["-u", "-z"]), ("umount", ["-l", "--"])] {
        let unmounted = Command::new(program)
            .args(args)
            .arg(mountpoint)
            .status()
            .is_ok_and(|status| status.success());
        if unmounted {
            return;
        }
    }
}

/// Mounts `drive` at `mountpoint`. Requests run on `runtime`.
/// The folder is created if needed, and a stale mount is cleaned up.
/// The owner of all entries becomes the owner of the mount point.
///
/// # Errors
/// [`MountError`] if the folder is missing or FUSE is not available.
pub fn mount<R, C, D, S, K>(
    drive: Arc<VirtualDrive<R, C, D, S, K>>,
    mountpoint: &Path,
    runtime: Handle,
) -> Result<MountHandle, MountError>
where
    R: RemoteStorage + 'static,
    C: ContentCache + 'static,
    D: DriveRepo + 'static,
    S: StateRepo + 'static,
    K: Clock + 'static,
{
    clear_stale_mount(mountpoint);
    // If the folder is missing we create it; if that fails, the check below reports it.
    let _ = std::fs::create_dir_all(mountpoint);
    let metadata = std::fs::metadata(mountpoint)
        .ok()
        .filter(std::fs::Metadata::is_dir)
        .ok_or_else(|| MountError::NotAFolder(mountpoint.to_path_buf()))?;
    let owner = Owner {
        uid: metadata.uid(),
        gid: metadata.gid(),
    };
    let ops = Arc::new(DriveOps::new(drive, owner));
    let filesystem = DriveFilesystem::new(ops, runtime);
    let session = fuser::spawn_mount(filesystem, mountpoint, &config()).map_err(|source| {
        MountError::Fuse {
            path: mountpoint.to_path_buf(),
            source,
        }
    })?;
    Ok(MountHandle {
        session,
        mountpoint: mountpoint.to_path_buf(),
    })
}
