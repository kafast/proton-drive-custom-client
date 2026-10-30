//! Port `DiskSpaceProbe` on the real file system (`statvfs` on Unix).

use std::io;
use std::path::Path;

use protonsync_application::ports::DiskSpaceProbe;
use protonsync_domain::disk::DiskSpace;

/// Reads the free space with `statvfs`; the device number of the folder
/// identifies the disk.
#[derive(Debug, Clone, Copy, Default)]
pub struct FsDiskSpace;

impl DiskSpaceProbe for FsDiskSpace {
    #[cfg(unix)]
    fn space(&self, path: &Path) -> io::Result<DiskSpace> {
        use std::os::unix::fs::MetadataExt;

        let stats = rustix::fs::statvfs(path)?;
        // Blocks are counted in units of `f_frsize` (the fragment size).
        let block = stats.f_frsize;
        Ok(DiskSpace {
            volume: Some(std::fs::metadata(path)?.dev()),
            available_bytes: stats.f_bavail.saturating_mul(block),
            total_bytes: stats.f_blocks.saturating_mul(block),
        })
    }

    #[cfg(not(unix))]
    fn space(&self, _path: &Path) -> io::Result<DiskSpace> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "free space is only checked on Unix",
        ))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn reads_the_space_of_a_real_folder() {
        let folder = tempfile::tempdir().unwrap();
        let space = FsDiskSpace.space(folder.path()).unwrap();
        assert!(space.total_bytes > 0);
        assert!(space.available_bytes <= space.total_bytes);
        let again = FsDiskSpace.space(folder.path()).unwrap();
        assert_eq!(space.volume, again.volume);
    }

    #[test]
    fn missing_folders_are_an_error() {
        assert!(FsDiskSpace.space(Path::new("/does/not/exist")).is_err());
    }
}
