//! Mounting the drives via FUSE (only with the feature `fuse` on Linux).

use std::path::Path;
use std::sync::Arc;

use protonsync_application::ports::{Clock, RemoteStorage};

use crate::core::{DaemonDrive, DaemonState, DriveMounter, MountGuard};

/// Mounts drives via FUSE; requests run on the daemon's tokio runtime.
#[derive(Debug, Clone, Copy, Default)]
pub struct FuseMounter;

impl<R, S, C> DriveMounter<DaemonDrive<R, S, C>> for FuseMounter
where
    R: RemoteStorage + 'static,
    S: DaemonState,
    C: Clock + 'static,
{
    fn mount(
        &self,
        mountpoint: &Path,
        drive: Arc<DaemonDrive<R, S, C>>,
    ) -> Result<MountGuard, String> {
        // Called from a tokio task; there is always a runtime there.
        let runtime = tokio::runtime::Handle::try_current().map_err(|e| e.to_string())?;
        let handle =
            protonsync_fuse::mount(drive, mountpoint, runtime).map_err(|e| e.to_string())?;
        Ok(Box::new(handle))
    }
}
