//! Wording of the notifications and the weekly report (German/English).
//!
//! Only plain text: it has to read well in an email as well as in a push message.

use std::fmt::Write as _;

use protonsync_api::{ConflictInfo, ConflictKindInfo, JobInfo, SyncOutcomeInfo, TrashInfo};
use protonsync_application::ports::Message;
use protonsync_client::Lang;
use protonsync_domain::time::Timestamp;

use super::tally::WeekTally;
use crate::local_time::LocalZone;

/// More conflicts than this are only counted in one message.
const MAX_LISTED_CONFLICTS: usize = 20;
/// Prefix of every subject, so that filters can catch the messages.
const SUBJECT_PREFIX: &str = "protonsync";

/// Local time as `YYYY-MM-DD HH:MM`.
pub(crate) fn format_time(zone: &LocalZone, at: Timestamp) -> String {
    zone.to_local(at).map_or_else(
        || at.unix_seconds().to_string(),
        |local| {
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}",
                local.year(),
                local.month(),
                local.day(),
                local.hour(),
                local.minute()
            )
        },
    )
}

fn subject(text: &str) -> String {
    format!("{SUBJECT_PREFIX}: {text}")
}

fn conflict_kind(lang: Lang, kind: ConflictKindInfo) -> &'static str {
    match kind {
        ConflictKindInfo::BothModified => {
            lang.pick("beide Seiten geändert", "changed on both sides")
        }
        ConflictKindInfo::LocalModifiedRemoteDeleted => lang.pick(
            "lokal geändert, in Proton gelöscht",
            "changed locally, deleted in Proton",
        ),
        ConflictKindInfo::LocalDeletedRemoteModified => lang.pick(
            "lokal gelöscht, in Proton geändert",
            "deleted locally, changed in Proton",
        ),
        ConflictKindInfo::LocalModifiedRemoteProtected => lang.pick(
            "lokal geändert, Proton-Version geschützt",
            "changed locally, Proton version protected",
        ),
    }
}

fn conflict_line(lang: Lang, conflict: &ConflictInfo) -> String {
    let mut line = format!(
        "- {}: {} ({})",
        conflict.pair,
        conflict.path,
        conflict_kind(lang, conflict.kind)
    );
    if let Some(copy) = &conflict.conflict_copy {
        let _ = write!(
            line,
            "\n  {} {copy}",
            lang.pick("Deine Version:", "Your version:")
        );
    }
    line
}

/// One message for several new conflicts.
pub(crate) fn conflicts(lang: Lang, conflicts: &[ConflictInfo]) -> Message {
    let count = conflicts.len();
    let title = if count == 1 {
        lang.pick("Neuer Konflikt", "New conflict").to_owned()
    } else {
        format!("{count} {}", lang.pick("neue Konflikte", "new conflicts"))
    };
    let mut body = lang
        .pick(
            "Diese Dateien warten auf deine Entscheidung. Beide Versionen sind erhalten.\n\n",
            "These files are waiting for your decision. Both versions are kept.\n\n",
        )
        .to_owned();
    for conflict in conflicts.iter().take(MAX_LISTED_CONFLICTS) {
        body.push_str(&conflict_line(lang, conflict));
        body.push('\n');
    }
    if count > MAX_LISTED_CONFLICTS {
        let _ = writeln!(
            body,
            "{} {}",
            lang.pick("… und weitere:", "… and more:"),
            count - MAX_LISTED_CONFLICTS
        );
    }
    body.push_str(lang.pick(
        "\nEntscheiden: protonsync conflicts (oder Tray-App/Web-Oberfläche).",
        "\nDecide: protonsync conflicts (or the tray app/web interface).",
    ));
    Message {
        subject: subject(&title),
        body,
    }
}

/// A sync pair (or the photo sync) failed.
pub(crate) fn sync_failed(lang: Lang, what: &str, message: &str) -> Message {
    Message {
        subject: subject(&format!(
            "{} {what}",
            lang.pick("Abgleich fehlgeschlagen:", "Sync failed:")
        )),
        body: format!(
            "{what}: {message}\n\n{}",
            lang.pick(
                "Der Daemon versucht es weiter. Diese Meldung kommt erst wieder, nachdem ein Abgleich geklappt hat.",
                "The daemon keeps trying. This message comes again only after a sync has worked.",
            )
        ),
    }
}

/// An upload job gave up.
pub(crate) fn job_gave_up(lang: Lang, job: &str, message: &str, attempts: u32) -> Message {
    Message {
        subject: subject(&format!(
            "{} {job}",
            lang.pick("Upload-Job fehlgeschlagen:", "Upload job failed:")
        )),
        body: format!(
            "{job}: {message}\n{}: {attempts}\n\n{}",
            lang.pick("Versuche", "Attempts"),
            lang.pick(
                "Verlauf: protonsync jobs history",
                "History: protonsync jobs history",
            )
        ),
    }
}

/// The login has to be renewed.
pub(crate) fn login_required(lang: Lang, message: &str) -> Message {
    Message {
        subject: subject(lang.pick("Anmeldung erforderlich", "Login required")),
        body: format!(
            "{message}\n\n{}",
            lang.pick(
                "Bis zur neuen Anmeldung wird nichts synchronisiert. Anmelden: protonsync login",
                "Nothing is synced until you log in again. Log in: protonsync login",
            )
        ),
    }
}

/// A disk is almost full, full, or has space again.
pub(crate) fn disk_space(lang: Lang, info: &protonsync_api::DiskSpaceInfo) -> Message {
    let (title, body) = protonsync_client::i18n::disk_space_message(lang, info);
    Message {
        subject: subject(&title),
        body,
    }
}

/// What the weekly report shows besides the counters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReportSnapshot {
    /// Logged-in account, `None` if logged out.
    pub account: Option<String>,
    /// Automatic syncs only plan.
    pub dry_run: bool,
    /// Last result per sync pair.
    pub last_syncs: Vec<(String, Option<protonsync_api::PairSyncResult>)>,
    /// Open conflicts.
    pub conflicts: Vec<ConflictInfo>,
    /// Trash of all owners.
    pub trash: Vec<TrashInfo>,
    /// Upload jobs.
    pub jobs: Vec<JobInfo>,
}

/// The weekly summary.
pub(crate) fn weekly_report(
    lang: Lang,
    zone: &LocalZone,
    tally: &WeekTally,
    snapshot: &ReportSnapshot,
    now: Timestamp,
) -> Message {
    let mut body = String::new();
    let _ = writeln!(
        body,
        "{} {} {} {}",
        lang.pick("Zeitraum:", "Period:"),
        format_time(zone, tally.since),
        lang.pick("bis", "to"),
        format_time(zone, now)
    );
    match &snapshot.account {
        Some(account) => {
            let _ = writeln!(body, "{} {account}", lang.pick("Angemeldet:", "Logged in:"));
        }
        None => body.push_str(lang.pick(
            "Nicht angemeldet: es wird nichts synchronisiert!\n",
            "Not logged in: nothing is synced!\n",
        )),
    }
    if snapshot.dry_run {
        body.push_str(lang.pick(
            "Probelauf aktiv (dry_run): Änderungen werden nur geplant.\n",
            "Dry run active (dry_run): changes are only planned.\n",
        ));
    }
    pairs_section(&mut body, lang, zone, tally, snapshot);
    conflicts_section(&mut body, lang, snapshot);
    trash_section(&mut body, lang, zone, tally, snapshot);
    jobs_section(&mut body, lang, zone, tally, snapshot);
    photos_section(&mut body, lang, tally);
    Message {
        subject: subject(lang.pick("Wochenbericht", "Weekly report")),
        body,
    }
}

fn pairs_section(
    body: &mut String,
    lang: Lang,
    zone: &LocalZone,
    tally: &WeekTally,
    snapshot: &ReportSnapshot,
) {
    if snapshot.last_syncs.is_empty() {
        return;
    }
    let _ = write!(body, "\n{}\n", lang.pick("Sync-Paare", "Sync pairs"));
    for (pair, last) in &snapshot.last_syncs {
        let counts = tally.pairs.get(pair).cloned().unwrap_or_default();
        let _ = writeln!(
            body,
            "- {pair}: {} {}, {} {}, {} {}, {} {}, {} {}",
            counts.uploaded,
            lang.pick("hochgeladen", "uploaded"),
            counts.downloaded,
            lang.pick("heruntergeladen", "downloaded"),
            counts.trashed_remote + counts.trashed_local,
            lang.pick("in den Papierkorb", "to the trash"),
            counts.conflicts,
            lang.pick("Konflikte", "conflicts"),
            counts.failed_runs + counts.step_failures,
            lang.pick("Fehler", "errors"),
        );
        if let Some(last) = last {
            let state = match &last.outcome {
                SyncOutcomeInfo::Failed { message } => {
                    format!("{} ({message})", lang.pick("fehlgeschlagen", "failed"))
                }
                SyncOutcomeInfo::Planned { .. } => lang.pick("nur geplant", "planned only").into(),
                SyncOutcomeInfo::Applied(_) => lang.pick("erfolgreich", "succeeded").into(),
            };
            let _ = writeln!(
                body,
                "  {} {}, {state}",
                lang.pick("Letzter Abgleich:", "Last sync:"),
                format_time(zone, Timestamp::from_unix_seconds(last.finished_at)),
            );
        }
    }
}

fn conflicts_section(body: &mut String, lang: Lang, snapshot: &ReportSnapshot) {
    let _ = write!(
        body,
        "\n{} {}\n",
        lang.pick("Offene Konflikte:", "Open conflicts:"),
        snapshot.conflicts.len()
    );
    for conflict in snapshot.conflicts.iter().take(MAX_LISTED_CONFLICTS) {
        body.push_str(&conflict_line(lang, conflict));
        body.push('\n');
    }
}

fn trash_section(
    body: &mut String,
    lang: Lang,
    zone: &LocalZone,
    tally: &WeekTally,
    snapshot: &ReportSnapshot,
) {
    let new = snapshot
        .trash
        .iter()
        .filter(|entry| entry.trashed_at >= tally.since.unix_seconds())
        .count();
    let _ = write!(
        body,
        "\n{} {} ({} {new})\n",
        lang.pick("Papierkorb:", "Trash:"),
        snapshot.trash.len(),
        lang.pick("neu in diesem Zeitraum:", "new in this period:"),
    );
    if let Some(next) = snapshot.trash.iter().map(|entry| entry.expires_at).min() {
        let _ = writeln!(
            body,
            "  {} {}",
            lang.pick("Nächste endgültige Löschung:", "Next permanent deletion:"),
            format_time(zone, Timestamp::from_unix_seconds(next))
        );
    }
}

fn jobs_section(
    body: &mut String,
    lang: Lang,
    zone: &LocalZone,
    tally: &WeekTally,
    snapshot: &ReportSnapshot,
) {
    if snapshot.jobs.is_empty() {
        return;
    }
    let _ = write!(body, "\n{}\n", lang.pick("Upload-Jobs", "Upload jobs"));
    for job in &snapshot.jobs {
        let counts = tally.jobs.get(&job.id).cloned().unwrap_or_default();
        let _ = write!(
            body,
            "- {}: {} {}, {} {}, {} {}",
            job.id,
            counts.succeeded,
            lang.pick("Läufe erfolgreich", "runs succeeded"),
            counts.failed,
            lang.pick("fehlgeschlagen", "failed"),
            counts.uploaded,
            lang.pick("Dateien hochgeladen", "files uploaded"),
        );
        if let Some(next) = job.next_run {
            let _ = write!(
                body,
                "; {} {}",
                lang.pick("nächster Lauf", "next run"),
                format_time(zone, Timestamp::from_unix_seconds(next))
            );
        }
        body.push('\n');
    }
}

fn photos_section(body: &mut String, lang: Lang, tally: &WeekTally) {
    let Some(photos) = &tally.photos else {
        return;
    };
    let _ = write!(
        body,
        "\n{} {} {}, {} {}, {} {}\n",
        lang.pick("Fotos:", "Photos:"),
        photos.downloaded,
        lang.pick("heruntergeladen", "downloaded"),
        photos.uploaded,
        lang.pick("hochgeladen", "uploaded"),
        photos.failed_runs,
        lang.pick("fehlgeschlagene Läufe", "failed runs"),
    );
}
