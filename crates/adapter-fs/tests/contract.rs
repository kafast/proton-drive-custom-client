//! `FsLocal` passes the contract tests for `LocalStorage` in a temporary folder.

// Helper functions in test files may abort like tests.
#![allow(clippy::unwrap_used)]

use protonsync_adapter_fs::FsLocal;
use protonsync_application::ports::LocalStorage;
use protonsync_domain::path::RelativePath;

/// Adapter together with a temporary directory that lives until the test ends.
struct InTempDir {
    local: FsLocal,
    _dir: tempfile::TempDir,
}

impl std::ops::Deref for InTempDir {
    type Target = FsLocal;
    fn deref(&self) -> &FsLocal {
        &self.local
    }
}

fn in_temp_dir() -> InTempDir {
    let dir = tempfile::tempdir().unwrap();
    InTempDir {
        local: FsLocal::new(dir.path()).unwrap(),
        _dir: dir,
    }
}

protonsync_testkit::local_storage_contract_tests!(deref: in_temp_dir);

#[test]
fn internal_files_and_symlinks_are_not_scanned() {
    let local = in_temp_dir();
    std::fs::write(local.root().join(".protonsync-tmp-x"), b"x").unwrap();
    std::fs::write(local.root().join("real.txt"), b"x").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("real.txt", local.root().join("link")).unwrap();

    let paths: Vec<String> = local
        .scan()
        .unwrap()
        .into_iter()
        .map(|entry| entry.path.as_str().to_owned())
        .collect();
    assert_eq!(paths, ["real.txt"]);
}

#[test]
fn trashed_files_stay_out_of_the_scan() {
    let local = in_temp_dir();
    std::fs::write(local.root().join("gone.txt"), b"x").unwrap();
    local
        .move_to_trash(&RelativePath::parse("gone.txt").unwrap())
        .unwrap();
    assert_eq!(local.scan().unwrap(), Vec::new());
    assert!(local.root().join(".protonsync-trash").is_dir());
}
