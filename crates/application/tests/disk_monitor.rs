//! The free-space check reports level changes per disk, not every check.

#![allow(clippy::unwrap_used)]

use protonsync_application::use_cases::{DiskMonitor, WatchedFolder};
use protonsync_domain::disk::{DiskLevel, DiskThresholds};
use protonsync_testkit::FakeDiskSpace;

const TOTAL: u64 = 1_000;

fn folders() -> Vec<WatchedFolder> {
    ["main", "photos", "data"]
        .into_iter()
        .map(|name| WatchedFolder {
            name: name.into(),
            path: format!("/{name}").into(),
        })
        .collect()
}

#[test]
fn reports_only_changes_and_groups_folders_by_disk() {
    let probe = FakeDiskSpace::new();
    probe.set("/main", 1, 500, TOTAL);
    probe.set("/photos", 1, 500, TOTAL);
    probe.set("/data", 2, 500, TOTAL);
    let mut monitor = DiskMonitor::new(DiskThresholds::default());
    let mut errors = 0;

    assert_eq!(monitor.check(&probe, &folders(), |_, _| errors += 1), []);

    probe.set("/main", 1, 50, TOTAL);
    probe.set("/photos", 1, 50, TOTAL);
    let changes = monitor.check(&probe, &folders(), |_, _| errors += 1);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].level, DiskLevel::Low);
    assert_eq!(changes[0].folders, ["main", "photos"]);
    assert_eq!(changes[0].path, std::path::Path::new("/main"));

    // Same level again: nothing to report.
    assert_eq!(monitor.check(&probe, &folders(), |_, _| errors += 1), []);

    probe.set("/main", 1, 5, TOTAL);
    probe.set("/photos", 1, 5, TOTAL);
    let changes = monitor.check(&probe, &folders(), |_, _| errors += 1);
    assert_eq!(changes[0].level, DiskLevel::Full);

    probe.set("/main", 1, 900, TOTAL);
    probe.set("/photos", 1, 900, TOTAL);
    let changes = monitor.check(&probe, &folders(), |_, _| errors += 1);
    assert_eq!(changes[0].level, DiskLevel::Ok);
    assert_eq!(errors, 0);
}

#[test]
fn folders_that_cannot_be_checked_are_passed_to_the_error_handler() {
    let probe = FakeDiskSpace::new();
    probe.set("/main", 1, 0, TOTAL);
    let mut monitor = DiskMonitor::new(DiskThresholds::default());
    let mut failed = Vec::new();

    let changes = monitor.check(&probe, &folders(), |folder, _| {
        failed.push(folder.name.clone());
    });

    assert_eq!(failed, ["photos", "data"]);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].level, DiskLevel::Full);
}
