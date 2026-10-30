//! Port for the free space of the local disks.

use std::io;
use std::path::Path;

use protonsync_domain::disk::DiskSpace;

/// Reads size and free space of the file system a folder lies on.
pub trait DiskSpaceProbe: Send + Sync {
    /// Space of the file system that holds `path`.
    ///
    /// # Errors
    /// The I/O error, e.g. if `path` does not exist or the platform cannot tell.
    fn space(&self, path: &Path) -> io::Result<DiskSpace>;
}
