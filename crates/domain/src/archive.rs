//! Version archive of the backup mode (direction `backup_only`).
//!
//! Before a changed file from Proton replaces the local copy, the local copy is
//! moved into the folder [`ARCHIVE_FOLDER`] next to it. Its name gets the point in
//! time as a prefix, so any number of versions of a file can pile up and they
//! sort chronologically, e.g. `_archiv/2026-10-28_23-06-40_report.pdf`.

use crate::job::LocalDateTime;
use crate::time::Timestamp;

/// Name of the archive folder inside each folder of a backup pair.
///
/// It is part of the visible folder layout and stays German, like the
/// conflict copy marker.
pub const ARCHIVE_FOLDER: &str = "_archiv";

const SECONDS_PER_MINUTE: i64 = 60;

/// Name of an archived version: UTC time of archiving as prefix,
/// `YYYY-MM-DD_HH-MM-SS_<original>`.
///
/// `attempt` is 0 for the first try; if that name is taken (two versions within the
/// same second), attempts 2, 3, … add ` (2)`, ` (3)`, … before the extension.
/// `None` if the point in time cannot be represented as a calendar date.
#[must_use]
pub fn archived_name(original: &str, archived_at: Timestamp, attempt: u32) -> Option<String> {
    let time = LocalDateTime::from_unix_utc(archived_at)?;
    let second = archived_at.unix_seconds().rem_euclid(SECONDS_PER_MINUTE);
    let prefix = format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        time.year(),
        time.month(),
        time.day(),
        time.hour(),
        time.minute(),
        second
    );
    let name = if attempt < 2 {
        original.to_owned()
    } else {
        with_counter(original, attempt)
    };
    Some(format!("{prefix}_{name}"))
}

/// `report.pdf` → `report (2).pdf`; a leading dot is not an extension.
fn with_counter(original: &str, counter: u32) -> String {
    match original.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => format!("{stem} ({counter}).{extension}"),
        _ => format!("{original} ({counter})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-10-28 23:06:40 UTC.
    const AT: Timestamp = Timestamp::from_unix_seconds(1_793_228_800);

    #[test]
    fn prefix_is_the_utc_time_down_to_the_second() {
        assert_eq!(
            archived_name("report.pdf", AT, 0).as_deref(),
            Some("2026-10-28_23-06-40_report.pdf")
        );
    }

    #[test]
    fn later_attempts_get_a_counter_before_the_extension() {
        assert_eq!(
            archived_name("report.pdf", AT, 2).as_deref(),
            Some("2026-10-28_23-06-40_report (2).pdf")
        );
        assert_eq!(
            archived_name(".bashrc", AT, 3).as_deref(),
            Some("2026-10-28_23-06-40_.bashrc (3)")
        );
    }

    #[test]
    fn names_sort_chronologically() {
        let earlier = archived_name("a", Timestamp::from_unix_seconds(1_700_000_000), 0);
        let later = archived_name("a", AT, 0);
        assert!(earlier < later);
    }
}
