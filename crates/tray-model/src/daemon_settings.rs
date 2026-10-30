//! The daemon's editable settings in the tray app (ADR 0021): the view for
//! QML and the changes that QML's controls trigger. Every control applies at
//! once; there is no separate "Save" button.

use protonsync_api::{Direction, PairSettingsChange, SettingsChange, SettingsInfo};
use protonsync_client::Lang;
use serde::Serialize;

/// The pair option `protect_remote`.
pub const PROTECT_REMOTE: &str = "protect_remote";
/// The pair option `archive_versions`.
pub const ARCHIVE_VERSIONS: &str = "archive_versions";

/// All directions in the order of the drop-down list.
const DIRECTIONS: [Direction; 4] = [
    Direction::TwoWay,
    Direction::DownloadOnly,
    Direction::UploadOnly,
    Direction::BackupOnly,
];

/// The daemon's settings as QML shows them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingsView {
    /// Whether the daemon can save changes.
    pub editable: bool,
    /// Path of the configuration file.
    pub config_path: String,
    /// Only plan automatic syncs.
    pub dry_run: bool,
    /// Days after which trash entries expire.
    pub trash_retention_days: u32,
    /// Choices for the direction, in this order.
    pub directions: Vec<DirectionOption>,
    /// Sync pairs.
    pub pairs: Vec<PairSettingsView>,
}

/// One choice in the drop-down list of directions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DirectionOption {
    /// Value for [`direction_change`].
    pub key: &'static str,
    /// Label.
    pub label: &'static str,
}

/// Settings of one sync pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PairSettingsView {
    /// Sync pair.
    pub id: String,
    /// Index into [`SettingsView::directions`].
    pub direction_index: usize,
    /// Local deletions and replacements are not passed on to Proton.
    pub protect_remote: bool,
    /// Deleted and replaced versions are moved into `_archiv`.
    pub archive_versions: bool,
}

/// Value of a direction (as in the configuration file).
fn direction_key(direction: Direction) -> &'static str {
    match direction {
        Direction::TwoWay => "two_way",
        Direction::DownloadOnly => "download_only",
        Direction::UploadOnly => "upload_only",
        Direction::BackupOnly => "backup_only",
    }
}

fn direction_label(lang: Lang, direction: Direction) -> &'static str {
    match direction {
        Direction::TwoWay => lang.pick("Zwei Wege", "Two-way"),
        Direction::DownloadOnly => lang.pick("Nur herunterladen", "Download only"),
        Direction::UploadOnly => lang.pick("Nur hochladen", "Upload only"),
        Direction::BackupOnly => lang.pick(
            "Nur Sicherung (mit Versionsarchiv)",
            "Backup only (with version archive)",
        ),
    }
}

/// The view of `info` in `lang`.
#[must_use]
pub fn settings_view(info: &SettingsInfo, lang: Lang) -> SettingsView {
    SettingsView {
        editable: info.editable,
        config_path: info.config_path.clone(),
        dry_run: info.dry_run,
        trash_retention_days: info.trash_retention_days,
        directions: DIRECTIONS
            .into_iter()
            .map(|direction| DirectionOption {
                key: direction_key(direction),
                label: direction_label(lang, direction),
            })
            .collect(),
        pairs: info
            .pairs
            .iter()
            .map(|pair| PairSettingsView {
                id: pair.id.clone(),
                direction_index: DIRECTIONS
                    .iter()
                    .position(|direction| *direction == pair.direction)
                    .unwrap_or_default(),
                protect_remote: pair.protect_remote,
                archive_versions: pair.archive_versions,
            })
            .collect(),
    }
}

/// Switch the dry run on or off.
#[must_use]
pub fn dry_run_change(dry_run: bool) -> SettingsChange {
    SettingsChange {
        dry_run: Some(dry_run),
        ..SettingsChange::default()
    }
}

/// New retention in the trash; `None` for less than one day.
#[must_use]
pub fn retention_change(days: i64) -> Option<SettingsChange> {
    let days = u32::try_from(days).ok().filter(|days| *days > 0)?;
    Some(SettingsChange {
        trash_retention_days: Some(days),
        ..SettingsChange::default()
    })
}

/// New direction of a pair; `key` from [`DirectionOption::key`].
#[must_use]
pub fn direction_change(pair: &str, key: &str) -> Option<SettingsChange> {
    let direction = DIRECTIONS
        .into_iter()
        .find(|direction| direction_key(*direction) == key)?;
    Some(pair_change(PairSettingsChange {
        direction: Some(direction),
        ..PairSettingsChange::new(pair)
    }))
}

/// Switch a pair option ([`PROTECT_REMOTE`], [`ARCHIVE_VERSIONS`]).
#[must_use]
pub fn option_change(pair: &str, option: &str, enabled: bool) -> Option<SettingsChange> {
    let mut change = PairSettingsChange::new(pair);
    match option {
        PROTECT_REMOTE => change.protect_remote = Some(enabled),
        ARCHIVE_VERSIONS => change.archive_versions = Some(enabled),
        _ => return None,
    }
    Some(pair_change(change))
}

fn pair_change(pair: PairSettingsChange) -> SettingsChange {
    SettingsChange {
        pairs: vec![pair],
        ..SettingsChange::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protonsync_api::PairSettings;

    fn info() -> SettingsInfo {
        SettingsInfo {
            config_path: "/c.toml".into(),
            editable: true,
            dry_run: true,
            trash_retention_days: 30,
            pairs: vec![PairSettings {
                id: "main".into(),
                direction: Direction::BackupOnly,
                protect_remote: true,
                archive_versions: false,
            }],
        }
    }

    #[test]
    fn the_view_points_at_the_current_direction() {
        let view = settings_view(&info(), Lang::De);
        let pair = &view.pairs[0];
        assert_eq!(view.directions[pair.direction_index].key, "backup_only");
        assert_eq!(
            view.directions[0].label, "Zwei Wege",
            "labels in the language"
        );
        assert!(pair.protect_remote);
    }

    #[test]
    fn controls_become_changes() {
        assert_eq!(dry_run_change(false).dry_run, Some(false));
        assert_eq!(retention_change(14).unwrap().trash_retention_days, Some(14));
        assert_eq!(retention_change(0), None);
        assert_eq!(retention_change(-3), None);
        let direction = direction_change("main", "download_only").unwrap();
        assert_eq!(direction.pairs[0].direction, Some(Direction::DownloadOnly));
        assert_eq!(direction_change("main", "sideways"), None);
        let archive = option_change("main", ARCHIVE_VERSIONS, true).unwrap();
        assert_eq!(archive.pairs[0].archive_versions, Some(true));
        assert_eq!(archive.pairs[0].protect_remote, None);
        let protect = option_change("main", PROTECT_REMOTE, false).unwrap();
        assert_eq!(protect.pairs[0].protect_remote, Some(false));
        assert_eq!(option_change("main", "unknown", true), None);
    }
}
