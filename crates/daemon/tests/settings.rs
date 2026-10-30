//! Changing settings at runtime via the API (ADR 0021), against fakes.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::{Arc, Mutex};

use common::{Fixture, PAIR, T0, fixture_with, logged_in};
use protonsync_api::{
    Direction, ErrorCode, PairSettingsChange, Request, Response, SettingsChange, SettingsInfo,
    SyncOutcomeInfo,
};
use protonsync_application::ports::RemoteStorage;
use protonsync_daemon::core::SettingsFile;
use protonsync_testkit::FakeAccount;

/// Records saved changes; can be told to refuse them.
#[derive(Default)]
struct FakeFile {
    saved: Mutex<Vec<SettingsChange>>,
    refuse: bool,
}

impl SettingsFile for FakeFile {
    fn path(&self) -> String {
        "/home/k/.config/protonsync/config.toml".into()
    }

    fn writable(&self) -> bool {
        !self.refuse
    }

    fn save(&self, change: &SettingsChange) -> Result<(), String> {
        if self.refuse {
            return Err("read-only file system".into());
        }
        self.saved.lock().unwrap().push(change.clone());
        Ok(())
    }
}

fn with_file(f: Fixture, file: &Arc<FakeFile>) -> Fixture {
    let file: Arc<dyn SettingsFile> = Arc::clone(file) as Arc<dyn SettingsFile>;
    Fixture {
        core: f.core.with_settings_file(file),
        ..f
    }
}

async fn settings(f: &Fixture) -> SettingsInfo {
    match f.core.handle(Request::Settings).await {
        Response::Settings(info) => info,
        other => panic!("unexpected: {other:?}"),
    }
}

async fn change(f: &Fixture, change: SettingsChange) -> Response {
    f.core.handle(Request::ChangeSettings { change }).await
}

fn error_code(response: Response) -> ErrorCode {
    match response {
        Response::Error(error) => error.code,
        other => panic!("expected an error: {other:?}"),
    }
}

fn pair_change(change: impl FnOnce(&mut PairSettingsChange)) -> SettingsChange {
    let mut pair = PairSettingsChange::new(PAIR);
    change(&mut pair);
    SettingsChange {
        pairs: vec![pair],
        ..SettingsChange::default()
    }
}

async fn remote_file_count(f: &Fixture) -> usize {
    let root = f.remote.root_folder().await.unwrap();
    f.remote.list_children(&root.id).await.unwrap().len()
}

#[tokio::test]
async fn settings_show_the_current_values() {
    let file = Arc::new(FakeFile::default());
    let f = with_file(fixture_with(FakeAccount::new, true), &file);
    let info = settings(&f).await;
    assert!(info.editable);
    assert!(info.dry_run);
    assert_eq!(info.trash_retention_days, 30);
    assert_eq!(info.config_path, "/home/k/.config/protonsync/config.toml");
    assert_eq!(info.pairs.len(), 1);
    assert_eq!(info.pairs[0].direction, Direction::TwoWay);
    assert!(!info.pairs[0].protect_remote);
}

#[tokio::test]
async fn without_a_file_nothing_can_be_changed() {
    let f = fixture_with(FakeAccount::new, true);
    assert!(!settings(&f).await.editable);
    let response = change(
        &f,
        SettingsChange {
            dry_run: Some(false),
            ..SettingsChange::default()
        },
    )
    .await;
    assert_eq!(error_code(response), ErrorCode::Unavailable);
    assert!(settings(&f).await.dry_run);
}

#[tokio::test]
async fn switching_off_the_dry_run_lets_automatic_syncs_apply() {
    let file = Arc::new(FakeFile::default());
    let f = with_file(
        fixture_with(|r| FakeAccount::new(r).with_stored_session(), true),
        &file,
    );
    f.core.startup().await;
    f.local.write("report.pdf", b"content", T0);
    let planned = f.core.scheduled_sync(0).await.unwrap();
    assert!(matches!(planned.outcome, SyncOutcomeInfo::Planned { .. }));

    let response = change(
        &f,
        SettingsChange {
            dry_run: Some(false),
            trash_retention_days: Some(7),
            ..SettingsChange::default()
        },
    )
    .await;
    let Response::Settings(info) = response else {
        panic!("unexpected: {response:?}");
    };
    assert!(!info.dry_run);
    assert_eq!(info.trash_retention_days, 7);
    assert_eq!(file.saved.lock().unwrap().len(), 1);

    let applied = f.core.scheduled_sync(0).await.unwrap();
    assert!(matches!(applied.outcome, SyncOutcomeInfo::Applied(_)));
    assert_eq!(remote_file_count(&f).await, 1);
    let Response::Status(status) = f.core.handle(Request::Status).await else {
        panic!("expected a status");
    };
    assert!(!status.dry_run);
}

#[tokio::test]
async fn a_new_direction_applies_to_the_next_sync() {
    let file = Arc::new(FakeFile::default());
    let f = with_file(logged_in().await, &file);
    let response = change(
        &f,
        pair_change(|pair| pair.direction = Some(Direction::DownloadOnly)),
    )
    .await;
    assert!(matches!(response, Response::Settings(_)), "{response:?}");

    f.local.write("report.pdf", b"content", T0);
    let response = f
        .core
        .handle(Request::Sync {
            pair: None,
            apply: true,
        })
        .await;
    let Response::Sync { results } = response else {
        panic!("unexpected: {response:?}");
    };
    let SyncOutcomeInfo::Applied(summary) = &results[0].outcome else {
        panic!("unexpected: {:?}", results[0].outcome);
    };
    assert_eq!(summary.uploaded, 0, "download only never uploads");
    assert_eq!(remote_file_count(&f).await, 0);
    let Response::Status(status) = f.core.handle(Request::Status).await else {
        panic!("expected a status");
    };
    assert_eq!(status.pairs[0].direction, Direction::DownloadOnly);
}

#[tokio::test]
async fn pair_options_are_reported_after_a_change() {
    let file = Arc::new(FakeFile::default());
    let f = with_file(logged_in().await, &file);
    change(
        &f,
        pair_change(|pair| {
            pair.protect_remote = Some(true);
            pair.archive_versions = Some(true);
        }),
    )
    .await;
    let info = settings(&f).await;
    assert!(info.pairs[0].protect_remote);
    assert!(info.pairs[0].archive_versions);
    assert_eq!(info.pairs[0].direction, Direction::TwoWay);
}

#[tokio::test]
async fn a_refused_save_changes_nothing() {
    let file = Arc::new(FakeFile {
        refuse: true,
        ..FakeFile::default()
    });
    let f = with_file(fixture_with(FakeAccount::new, true), &file);
    assert!(!settings(&f).await.editable);
    let response = change(
        &f,
        SettingsChange {
            dry_run: Some(false),
            ..pair_change(|pair| pair.protect_remote = Some(true))
        },
    )
    .await;
    assert_eq!(error_code(response), ErrorCode::Unavailable);
    let info = settings(&f).await;
    assert!(info.dry_run);
    assert!(!info.pairs[0].protect_remote);
}

#[tokio::test]
async fn invalid_changes_are_refused_before_saving() {
    let file = Arc::new(FakeFile::default());
    let f = with_file(fixture_with(FakeAccount::new, true), &file);
    assert_eq!(
        error_code(change(&f, SettingsChange::default()).await),
        ErrorCode::InvalidRequest
    );
    let unknown = SettingsChange {
        pairs: vec![PairSettingsChange {
            protect_remote: Some(true),
            ..PairSettingsChange::new("nope")
        }],
        ..SettingsChange::default()
    };
    assert_eq!(error_code(change(&f, unknown).await), ErrorCode::NotFound);
    let no_retention = SettingsChange {
        trash_retention_days: Some(0),
        ..SettingsChange::default()
    };
    assert_eq!(
        error_code(change(&f, no_retention).await),
        ErrorCode::InvalidRequest
    );
    assert_eq!(*file.saved.lock().unwrap(), Vec::<SettingsChange>::new());
}
