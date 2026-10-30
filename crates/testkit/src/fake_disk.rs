//! Disk space stand-in: every folder gets the space set for it in the test.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use protonsync_application::ports::DiskSpaceProbe;
use protonsync_domain::disk::DiskSpace;

/// Answers with the space set via [`FakeDiskSpace::set`]; unknown folders
/// give [`io::ErrorKind::NotFound`].
#[derive(Debug, Default)]
pub struct FakeDiskSpace {
    spaces: Mutex<HashMap<PathBuf, DiskSpace>>,
}

impl FakeDiskSpace {
    /// An empty fake.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `path` lies on disk `volume` with `available_bytes` of `total_bytes` free.
    pub fn set(
        &self,
        path: impl Into<PathBuf>,
        volume: u64,
        available_bytes: u64,
        total_bytes: u64,
    ) {
        if let Ok(mut spaces) = self.spaces.lock() {
            spaces.insert(
                path.into(),
                DiskSpace {
                    volume: Some(volume),
                    available_bytes,
                    total_bytes,
                },
            );
        }
    }
}

impl DiskSpaceProbe for FakeDiskSpace {
    fn space(&self, path: &Path) -> io::Result<DiskSpace> {
        self.spaces
            .lock()
            .ok()
            .and_then(|spaces| spaces.get(path).copied())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no space set for this folder"))
    }
}
