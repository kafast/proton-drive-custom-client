//! Textausgabe für Fotos und Duplikate.

use std::fmt::Write;

use protonsync_api::{
    DuplicateGroupInfo, DuplicateReasonInfo, PhotoCounts, PhotoInfo, PhotoOutcomeInfo,
    PhotoRunInfo, PhotoStatus, TrashedPhotoInfo,
};

use super::direction;
use crate::i18n::{Lang, format_bytes, format_date};

/// Zustand des Foto-Abgleichs.
#[must_use]
pub fn status(lang: Lang, status: &PhotoStatus) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{}: {} ({})",
        lang.pick("Fotoordner", "Photo folder"),
        status.folder,
        direction(lang, status.direction)
    );
    let count = status.photos.map_or_else(
        || lang.pick("noch nicht gelesen", "not read yet").to_owned(),
        |count| count.to_string(),
    );
    let _ = writeln!(
        out,
        "{}: {count}",
        lang.pick("Fotos in Proton", "Photos in Proton")
    );
    if status.syncing {
        let _ = writeln!(out, "{}", lang.pick("Abgleich läuft", "Sync running"));
    }
    let last = status.last_run.as_ref().map_or_else(
        || lang.pick("noch keiner", "none yet").to_owned(),
        |run| run_line(lang, run),
    );
    let _ = writeln!(
        out,
        "{}: {last}",
        lang.pick("Letzter Abgleich", "Last sync")
    );
    let _ = writeln!(
        out,
        "{}: {}",
        lang.pick("Papierkorb", "Trash"),
        status.trash_entries
    );
    out
}

/// Wie [`counts_line`], aber als Vorhaben formuliert (Probelauf).
fn planned_line(lang: Lang, counts: &PhotoCounts) -> String {
    format!(
        "{} {}, {} {}, {} {}, {} {}",
        counts.downloaded,
        lang.pick("herunterzuladen", "to download"),
        counts.uploaded,
        lang.pick("hochzuladen", "to upload"),
        counts.linked,
        lang.pick("zu verknüpfen", "to link"),
        counts.trashed_local + counts.trashed_remote,
        lang.pick("in Papierkörbe", "to trash"),
    )
}

fn run_line(lang: Lang, run: &PhotoRunInfo) -> String {
    let date = format_date(run.finished_at);
    match &run.outcome {
        PhotoOutcomeInfo::Planned(counts) => format!(
            "{date}, {}: {}",
            lang.pick("Probelauf", "dry run"),
            planned_line(lang, counts)
        ),
        PhotoOutcomeInfo::Applied(counts) => format!("{date}, {}", counts_line(lang, counts)),
        PhotoOutcomeInfo::Failed { message } => format!(
            "{date}, {}: {message}",
            lang.pick("fehlgeschlagen", "failed")
        ),
    }
}

fn counts_line(lang: Lang, counts: &PhotoCounts) -> String {
    if counts == &PhotoCounts::default() {
        return lang.pick("nichts zu tun", "nothing to do").to_owned();
    }
    format!(
        "{} {}, {} {}, {} {}, {} {}, {} {}",
        counts.downloaded,
        lang.pick("heruntergeladen", "downloaded"),
        counts.uploaded,
        lang.pick("hochgeladen", "uploaded"),
        counts.linked,
        lang.pick("verknüpft", "linked"),
        counts.trashed_local + counts.trashed_remote,
        lang.pick("in Papierkörbe", "trashed"),
        counts.failures.len(),
        lang.pick("Fehler", "errors"),
    )
}

/// Ergebnis von `photos sync`.
#[must_use]
pub fn run(lang: Lang, run: &PhotoRunInfo) -> String {
    let mut out = String::new();
    let counts = match &run.outcome {
        PhotoOutcomeInfo::Failed { message } => {
            let _ = writeln!(out, "{}: {message}", lang.pick("Fehlgeschlagen", "Failed"));
            return out;
        }
        PhotoOutcomeInfo::Planned(counts) => {
            let _ = writeln!(
                out,
                "{}",
                lang.pick(
                    "Probelauf, nichts wurde geändert. Mit --apply ausführen:",
                    "Dry run, nothing was changed. Run with --apply to execute:",
                )
            );
            let _ = writeln!(out, "  {}", planned_line(lang, counts));
            counts
        }
        PhotoOutcomeInfo::Applied(counts) => {
            let _ = writeln!(out, "{}", counts_line(lang, counts));
            counts
        }
    };
    if counts.ignored > 0 {
        let _ = writeln!(
            out,
            "  {}: {}",
            lang.pick(
                "Wegen der Richtung nicht übertragen",
                "Not transferred (direction)"
            ),
            counts.ignored
        );
    }
    for failure in &counts.failures {
        let _ = writeln!(
            out,
            "  {} {}: {}",
            lang.pick("Fehler", "error"),
            failure.path,
            failure.message
        );
    }
    for skip in &counts.skipped {
        let _ = writeln!(
            out,
            "  {} {}: {}",
            lang.pick("übersprungen", "skipped"),
            skip.path,
            skip.reason
        );
    }
    out
}

fn reason(lang: Lang, reason: DuplicateReasonInfo) -> &'static str {
    match reason {
        DuplicateReasonInfo::SameContent => lang.pick("gleicher Inhalt", "same content"),
        DuplicateReasonInfo::SameNameAndCaptureTime => lang.pick(
            "gleicher Name und Aufnahmezeit (Inhalt verschieden)",
            "same name and capture time (content differs)",
        ),
    }
}

fn photo_line(lang: Lang, photo: &PhotoInfo) -> String {
    let mut line = format!(
        "{} {} ({}, {} {})",
        photo.id,
        photo.name,
        format_bytes(photo.size),
        lang.pick("hochgeladen", "uploaded"),
        format_date(photo.uploaded)
    );
    if photo.favorite {
        line.push_str(lang.pick(", Favorit", ", favorite"));
    }
    if photo.albums > 0 {
        let _ = write!(line, ", {}: {}", lang.pick("Alben", "albums"), photo.albums);
    }
    line
}

/// Vorschlagsliste von `photos duplicates`.
#[must_use]
pub fn duplicates(lang: Lang, groups: &[DuplicateGroupInfo]) -> String {
    if groups.is_empty() {
        return format!(
            "{}\n",
            lang.pick("Keine Duplikate gefunden.", "No duplicates found.")
        );
    }
    let mut out = String::new();
    for group in groups {
        let _ = writeln!(
            out,
            "{}, {}:",
            format_date(group.captured),
            reason(lang, group.reason)
        );
        let _ = writeln!(
            out,
            "  {}   {}",
            lang.pick("behalten:", "keep:   "),
            photo_line(lang, &group.keep)
        );
        for other in &group.others {
            let _ = writeln!(
                out,
                "  {}   {}",
                lang.pick("doppelt: ", "copy:   "),
                photo_line(lang, other)
            );
        }
    }
    let _ = writeln!(
        out,
        "\n{}",
        lang.pick(
            "Nur Vorschläge, nichts wurde gelöscht. Löschen (über den Papierkorb):\n  protonsync photos duplicates trash KENNUNG… --apply",
            "Suggestions only, nothing was deleted. To delete (via the trash):\n  protonsync photos duplicates trash ID… --apply",
        )
    );
    out
}

/// Ergebnis von `photos duplicates trash`.
#[must_use]
pub fn trashed(lang: Lang, dry_run: bool, photos: &[TrashedPhotoInfo]) -> String {
    let mut out = String::new();
    let verb = if dry_run {
        lang.pick("Würde in den Papierkorb legen", "Would move to trash")
    } else {
        lang.pick("In den Papierkorb gelegt", "Moved to trash")
    };
    let _ = writeln!(out, "{verb}: {}", photos.len());
    for photo in photos {
        let _ = writeln!(out, "  {} {}", photo.id, photo.path);
    }
    if dry_run {
        let _ = writeln!(
            out,
            "{}",
            lang.pick(
                "Probelauf: Mit --apply wirklich löschen.",
                "Dry run: use --apply to actually delete."
            )
        );
    } else {
        let _ = writeln!(
            out,
            "{}",
            lang.pick(
                "Wiederherstellen: protonsync trash restore photos:library NR (Nummer aus `protonsync trash`)",
                "Restore: protonsync trash restore photos:library NO (number from `protonsync trash`)"
            )
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use protonsync_api::{Direction, FailureInfo, SkippedInfo};

    fn counts() -> PhotoCounts {
        PhotoCounts {
            downloaded: 3,
            linked: 1,
            ignored: 2,
            skipped: vec![SkippedInfo {
                path: "2026/10/b.jpg".into(),
                reason: "lokal bearbeitet".into(),
            }],
            failures: vec![FailureInfo {
                path: "2026/10/c.jpg".into(),
                message: "offline".into(),
            }],
            ..PhotoCounts::default()
        }
    }

    fn status_sample() -> PhotoStatus {
        PhotoStatus {
            folder: "/home/k/Bilder/Proton".into(),
            direction: Direction::DownloadOnly,
            syncing: false,
            photos: Some(1234),
            last_run: Some(PhotoRunInfo {
                finished_at: 1_760_000_000,
                outcome: PhotoOutcomeInfo::Applied(counts()),
            }),
            trash_entries: 2,
        }
    }

    fn photo(id: &str, favorite: bool, albums: usize) -> PhotoInfo {
        PhotoInfo {
            id: id.into(),
            name: "IMG_0001.jpg".into(),
            size: 2_400_000,
            uploaded: 1_759_000_000,
            favorite,
            albums,
        }
    }

    #[test]
    fn status_snapshots() {
        insta::assert_snapshot!("photo_status_de", status(Lang::De, &status_sample()));
        let fresh = PhotoStatus {
            photos: None,
            last_run: None,
            ..status_sample()
        };
        insta::assert_snapshot!("photo_status_fresh_en", status(Lang::En, &fresh));
    }

    #[test]
    fn run_snapshots() {
        let planned = PhotoRunInfo {
            finished_at: 0,
            outcome: PhotoOutcomeInfo::Planned(counts()),
        };
        insta::assert_snapshot!("photo_run_planned_de", run(Lang::De, &planned));
        let failed = PhotoRunInfo {
            finished_at: 0,
            outcome: PhotoOutcomeInfo::Failed {
                message: "Nicht angemeldet".into(),
            },
        };
        insta::assert_snapshot!("photo_run_failed_en", run(Lang::En, &failed));
    }

    #[test]
    fn duplicate_snapshots() {
        let groups = vec![DuplicateGroupInfo {
            reason: DuplicateReasonInfo::SameContent,
            captured: 1_758_000_000,
            keep: photo("vol~a", true, 1),
            others: vec![photo("vol~b", false, 0)],
        }];
        insta::assert_snapshot!("duplicates_de", duplicates(Lang::De, &groups));
        insta::assert_snapshot!("duplicates_none_en", duplicates(Lang::En, &[]));
        let trashed_photos = vec![TrashedPhotoInfo {
            id: "vol~b".into(),
            path: "2025/09/IMG_0001.jpg".into(),
        }];
        insta::assert_snapshot!(
            "duplicates_trash_dry_run_en",
            trashed(Lang::En, true, &trashed_photos)
        );
        insta::assert_snapshot!(
            "duplicates_trashed_de",
            trashed(Lang::De, false, &trashed_photos)
        );
    }
}
