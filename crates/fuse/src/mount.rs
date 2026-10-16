//! Einhängen und Aushängen des Laufwerks.

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

/// Name in `mount` und `df`.
const FILESYSTEM_NAME: &str = "protonsync";
/// Threads, die Kernel-Anfragen annehmen. Die eigentliche Arbeit läuft in
/// tokio; mehr Threads helfen nur beim schnellen Annehmen.
const REQUEST_THREADS: usize = 2;

/// Fehler beim Einhängen oder Aushängen.
#[derive(Debug, thiserror::Error)]
pub enum MountError {
    /// Der Einhängepunkt fehlt oder ist kein Ordner.
    #[error("Einhängepunkt {0} ist kein Ordner")]
    NotAFolder(PathBuf),
    /// Der Kernel bzw. `fusermount3` hat abgelehnt (fehlt z. B. das Paket `fuse3`?).
    #[error("FUSE-Fehler bei {path}: {source}")]
    Fuse {
        /// Einhängepunkt.
        path: PathBuf,
        /// Ursache.
        source: io::Error,
    },
}

/// Ein eingehängtes Laufwerk. Wird es fallen gelassen, wird ausgehängt.
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
    /// Der Einhängepunkt.
    #[must_use]
    pub fn mountpoint(&self) -> &Path {
        &self.mountpoint
    }

    /// Hängt aus und wartet, bis alle Anfragen beendet sind.
    ///
    /// # Errors
    /// [`MountError::Fuse`], wenn das Aushängen scheitert (z. B. weil noch ein
    /// Programm im Laufwerk arbeitet).
    pub fn unmount(self) -> Result<(), MountError> {
        let path = self.mountpoint;
        self.session
            .umount_and_join()
            .map_err(|source| MountError::Fuse { path, source })
    }
}

/// Die Einhängeoptionen: nur der eigene Benutzer, keine Geräte- oder
/// SUID-Dateien, Rechte prüft der Kernel anhand der Attribute.
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

/// Räumt einen verwaisten Mount auf (der Daemon ist abgestürzt, der Kernel
/// meldet „Transport endpoint is not connected“). Scheitert das, meldet das
/// anschließende Einhängen den Fehler.
fn clear_stale_mount(mountpoint: &Path) {
    let stale = matches!(
        std::fs::metadata(mountpoint),
        Err(error) if error.raw_os_error() == Some(libc::ENOTCONN)
    );
    if !stale {
        return;
    }
    tracing::warn!(at = %mountpoint.display(), "Verwaister Mount, hänge aus");
    // `fusermount3` für normale Benutzer, `umount` als root (ohne fusermount3).
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

/// Hängt `drive` unter `mountpoint` ein. Anfragen laufen auf `runtime`.
/// Der Ordner wird bei Bedarf angelegt, ein verwaister Mount aufgeräumt.
/// Besitzer aller Einträge wird der Besitzer des Einhängepunkts.
///
/// # Errors
/// [`MountError`], wenn der Ordner fehlt oder FUSE nicht verfügbar ist.
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
    // Fehlt der Ordner, legen wir ihn an; scheitert das, meldet es die Prüfung danach.
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
