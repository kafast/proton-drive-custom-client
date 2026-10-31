//! Text output for the change log.

use std::fmt::Write;

use jiff::tz::TimeZone;
use protonsync_api::ChangeInfo;
use protonsync_client::changes::{describe, side};

use crate::i18n::{Lang, format_date_time};

/// The change log, newest first, one line per change.
#[must_use]
pub fn changes(lang: Lang, zone: &TimeZone, changes: &[ChangeInfo], keep_days: u32) -> String {
    if keep_days == 0 {
        return format!(
            "{}\n",
            lang.pick(
                "Das Änderungsprotokoll ist ausgeschaltet (change_log_days = 0).",
                "The change log is switched off (change_log_days = 0)."
            )
        );
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} {keep_days} {}",
        lang.pick(
            "Änderungen der Abgleiche, neueste zuerst; aufbewahrt",
            "Changes made by syncs, newest first; kept for"
        ),
        lang.pick("Tage.", "days.")
    );
    if changes.is_empty() {
        let _ = writeln!(
            out,
            "{}",
            lang.pick("Keine Änderungen protokolliert.", "No changes logged.")
        );
        return out;
    }
    for change in changes {
        let _ = writeln!(
            out,
            "{}  [{}]  {:<6}  {}{}: {}",
            format_date_time(change.at, zone),
            change.pair,
            side(lang, change.side),
            change.path,
            if change.is_folder { "/" } else { "" },
            describe(lang, change)
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use protonsync_api::{ChangeKindInfo, ChangedSideInfo};

    const AT: i64 = 1_760_000_000;

    fn samples() -> Vec<ChangeInfo> {
        let change = |at, path: &str, side, kind, is_folder| ChangeInfo {
            pair: "main".into(),
            at,
            path: path.into(),
            side,
            kind,
            is_folder,
        };
        vec![
            change(
                AT,
                "Docs/report.pdf",
                ChangedSideInfo::Remote,
                ChangeKindInfo::Modified,
                false,
            ),
            change(
                AT - 60,
                "Docs",
                ChangedSideInfo::Remote,
                ChangeKindInfo::Created,
                true,
            ),
            change(
                AT - 120,
                "old.txt",
                ChangedSideInfo::Local,
                ChangeKindInfo::Trashed,
                false,
            ),
            change(
                AT - 180,
                "photo.jpg",
                ChangedSideInfo::Local,
                ChangeKindInfo::Created,
                false,
            ),
        ]
    }

    #[test]
    fn change_log_snapshots() {
        let utc = TimeZone::UTC;
        insta::assert_snapshot!("changes_de", changes(Lang::De, &utc, &samples(), 30));
        insta::assert_snapshot!("changes_en", changes(Lang::En, &utc, &samples(), 30));
        insta::assert_snapshot!("changes_none_en", changes(Lang::En, &utc, &[], 30));
        insta::assert_snapshot!("changes_off_de", changes(Lang::De, &utc, &[], 0));
    }
}
