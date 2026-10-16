//! Textausgabe der Antworten für Menschen (für Maschinen gibt es `--json`).

use std::fmt::Write;

use protonsync_api::{
    ConflictInfo, ConflictKindInfo, Direction, PairSyncResult, StatusInfo, StepAction,
    SyncOutcomeInfo, SyncSummary, TrashInfo, TrashSide,
};

use crate::i18n::{Lang, format_date};

pub mod files;
pub mod jobs;
pub mod mounts;
pub mod photos;

fn direction(lang: Lang, direction: Direction) -> &'static str {
    match direction {
        Direction::TwoWay => lang.pick("zwei Wege", "two-way"),
        Direction::DownloadOnly => lang.pick("nur herunterladen", "download only"),
        Direction::UploadOnly => lang.pick("nur hochladen", "upload only"),
    }
}

fn action(lang: Lang, action: StepAction) -> &'static str {
    match action {
        StepAction::Upload => lang.pick("hochladen", "upload"),
        StepAction::Download => lang.pick("herunterladen", "download"),
        StepAction::TrashRemote => lang.pick("in Proton-Papierkorb", "to Proton trash"),
        StepAction::TrashLocal => lang.pick("in lokalen Papierkorb", "to local trash"),
        StepAction::Conflict => lang.pick("Konflikt (beide behalten)", "conflict (keep both)"),
        StepAction::IgnoreLocalChange => {
            lang.pick("lokale Änderung bleibt lokal", "local change stays local")
        }
        StepAction::IgnoreRemoteChange => lang.pick(
            "entfernte Änderung bleibt entfernt",
            "remote change stays remote",
        ),
        StepAction::RecordConverged => lang.pick("bereits gleich", "already identical"),
    }
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
    }
}

fn side(lang: Lang, side: TrashSide) -> &'static str {
    match side {
        TrashSide::Local => lang.pick("lokal", "local"),
        TrashSide::Remote => "Proton",
    }
}

/// Gesamtzustand.
#[must_use]
pub fn status(lang: Lang, status: &StatusInfo) -> String {
    let mut out = String::new();
    let account = status.account.as_deref().map_or_else(
        || lang.pick("nicht angemeldet", "not logged in").to_owned(),
        ToOwned::to_owned,
    );
    let _ = writeln!(out, "{}: {account}", lang.pick("Konto", "Account"));
    if status.login_pending {
        let _ = writeln!(
            out,
            "{}",
            lang.pick(
                "Anmeldung begonnen, aber nicht abgeschlossen",
                "Login started but not finished"
            )
        );
    }
    if status.dry_run {
        let _ = writeln!(
            out,
            "{}",
            lang.pick(
                "Probelauf aktiv: automatische Abgleiche planen nur (dry_run = false schaltet ab).",
                "Dry run active: automatic syncs only plan (set dry_run = false to change).",
            )
        );
    }
    if status.pairs.is_empty() {
        let _ = writeln!(
            out,
            "{}",
            lang.pick(
                "Keine Sync-Paare konfiguriert.",
                "No sync pairs configured."
            )
        );
    }
    for pair in &status.pairs {
        let remote = if pair.remote_path.is_empty() {
            "/"
        } else {
            &pair.remote_path
        };
        let _ = writeln!(
            out,
            "\n[{}] {} <-> {} ({})",
            pair.id,
            pair.local_root,
            remote,
            direction(lang, pair.direction)
        );
        if pair.syncing {
            let _ = writeln!(out, "  {}", lang.pick("Abgleich läuft", "Sync running"));
        }
        let last = pair.last_sync.as_ref().map_or_else(
            || lang.pick("noch keiner", "none yet").to_owned(),
            |result| outcome_line(lang, result),
        );
        let _ = writeln!(
            out,
            "  {}: {last}",
            lang.pick("Letzter Abgleich", "Last sync")
        );
        let _ = writeln!(
            out,
            "  {}: {}   {}: {}",
            lang.pick("Offene Konflikte", "Open conflicts"),
            pair.open_conflicts,
            lang.pick("Papierkorb", "Trash"),
            pair.trash_entries
        );
    }
    out
}

fn outcome_line(lang: Lang, result: &PairSyncResult) -> String {
    let date = format_date(result.finished_at);
    match &result.outcome {
        SyncOutcomeInfo::Planned { steps, .. } => format!(
            "{date}, {} ({} {})",
            lang.pick("Probelauf", "dry run"),
            steps.len(),
            lang.pick("Schritte geplant", "steps planned")
        ),
        SyncOutcomeInfo::Applied(summary) => format!("{date}, {}", summary_line(lang, summary)),
        SyncOutcomeInfo::Failed { message } => {
            format!(
                "{date}, {}: {message}",
                lang.pick("fehlgeschlagen", "failed")
            )
        }
    }
}

fn summary_line(lang: Lang, summary: &SyncSummary) -> String {
    format!(
        "{} {}, {} {}, {} {}, {} {}, {} {}",
        summary.uploaded,
        lang.pick("hochgeladen", "uploaded"),
        summary.downloaded,
        lang.pick("heruntergeladen", "downloaded"),
        summary.trashed_remote + summary.trashed_local,
        lang.pick("in Papierkörbe", "trashed"),
        summary.conflicts,
        lang.pick("Konflikte", "conflicts"),
        summary.failures.len(),
        lang.pick("Fehler", "errors"),
    )
}

/// Ergebnis von `sync`.
#[must_use]
pub fn sync_results(lang: Lang, results: &[PairSyncResult]) -> String {
    let mut out = String::new();
    for result in results {
        let _ = writeln!(out, "[{}]", result.pair);
        match &result.outcome {
            SyncOutcomeInfo::Planned { steps, skipped } => {
                if steps.is_empty() && skipped.is_empty() {
                    let _ = writeln!(out, "  {}", lang.pick("Nichts zu tun.", "Nothing to do."));
                } else {
                    let _ = writeln!(
                        out,
                        "  {}",
                        lang.pick(
                            "Probelauf, nichts wurde geändert. Mit --apply ausführen:",
                            "Dry run, nothing was changed. Run with --apply to execute:",
                        )
                    );
                }
                for step in steps {
                    let _ = writeln!(out, "  {:<36} {}", action(lang, step.action), step.path);
                }
                for skip in skipped {
                    let _ = writeln!(
                        out,
                        "  {} {}: {}",
                        lang.pick("übersprungen", "skipped"),
                        skip.path,
                        skip.reason
                    );
                }
            }
            SyncOutcomeInfo::Applied(summary) => {
                let _ = writeln!(out, "  {}", summary_line(lang, summary));
                for failure in &summary.failures {
                    let _ = writeln!(
                        out,
                        "  {} {}: {}",
                        lang.pick("Fehler", "error"),
                        failure.path,
                        failure.message
                    );
                }
                for skip in &summary.skipped {
                    let _ = writeln!(
                        out,
                        "  {} {}: {}",
                        lang.pick("übersprungen", "skipped"),
                        skip.path,
                        skip.reason
                    );
                }
            }
            SyncOutcomeInfo::Failed { message } => {
                let _ = writeln!(
                    out,
                    "  {}: {message}",
                    lang.pick("Fehlgeschlagen", "Failed")
                );
            }
        }
    }
    out
}

/// Offene Konflikte.
#[must_use]
pub fn conflicts(lang: Lang, conflicts: &[ConflictInfo]) -> String {
    if conflicts.is_empty() {
        return format!(
            "{}\n",
            lang.pick("Keine offenen Konflikte.", "No open conflicts.")
        );
    }
    let mut out = String::new();
    for conflict in conflicts {
        let _ = writeln!(
            out,
            "[{}] #{} {} ({}, {})",
            conflict.pair,
            conflict.id,
            conflict.path,
            conflict_kind(lang, conflict.kind),
            format_date(conflict.detected_at)
        );
        if let Some(copy) = &conflict.conflict_copy {
            let _ = writeln!(
                out,
                "    {}: {copy}",
                lang.pick("Konfliktkopie", "Conflict copy")
            );
        }
    }
    let _ = writeln!(
        out,
        "\n{}",
        lang.pick(
            "Entscheiden: protonsync conflicts resolve <PAAR> <ID> keep-local|keep-remote|keep-both",
            "Decide: protonsync conflicts resolve <PAIR> <ID> keep-local|keep-remote|keep-both",
        )
    );
    out
}

/// Papierkorbeinträge.
#[must_use]
pub fn trash(lang: Lang, entries: &[TrashInfo], empty: &str) -> String {
    if entries.is_empty() {
        return format!("{empty}\n");
    }
    let mut out = String::new();
    for entry in entries {
        let _ = writeln!(
            out,
            "[{}] #{} {} ({}) {} {}, {} {}",
            entry.pair,
            entry.id,
            entry.path,
            side(lang, entry.side),
            lang.pick("gelöscht", "deleted"),
            format_date(entry.trashed_at),
            lang.pick("endgültig ab", "purged from"),
            format_date(entry.expires_at)
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use protonsync_api::{FailureInfo, PairStatus, PlannedStepInfo, SkippedInfo};

    const DAY: i64 = 1_760_000_000;

    fn status_sample() -> StatusInfo {
        StatusInfo {
            daemon_version: "0.1.0".into(),
            account: Some("krischan".into()),
            login_pending: false,
            dry_run: true,
            pairs: vec![PairStatus {
                id: "main".into(),
                local_root: "/home/k/Proton".into(),
                remote_path: "protonsync".into(),
                direction: Direction::TwoWay,
                syncing: false,
                last_sync: Some(PairSyncResult {
                    pair: "main".into(),
                    finished_at: DAY,
                    outcome: SyncOutcomeInfo::Applied(SyncSummary {
                        uploaded: 2,
                        downloaded: 1,
                        ..SyncSummary::default()
                    }),
                }),
                open_conflicts: 1,
                trash_entries: 3,
            }],
        }
    }

    fn sync_sample() -> Vec<PairSyncResult> {
        vec![
            PairSyncResult {
                pair: "main".into(),
                finished_at: DAY,
                outcome: SyncOutcomeInfo::Planned {
                    steps: vec![
                        PlannedStepInfo {
                            path: "a.txt".into(),
                            action: StepAction::Upload,
                        },
                        PlannedStepInfo {
                            path: "alt".into(),
                            action: StepAction::TrashRemote,
                        },
                    ],
                    skipped: vec![SkippedInfo {
                        path: "x".into(),
                        reason: "Datei und Ordner".into(),
                    }],
                },
            },
            PairSyncResult {
                pair: "docs".into(),
                finished_at: DAY,
                outcome: SyncOutcomeInfo::Applied(SyncSummary {
                    conflicts: 1,
                    failures: vec![FailureInfo {
                        path: "b".into(),
                        message: "Netz".into(),
                    }],
                    ..SyncSummary::default()
                }),
            },
            PairSyncResult {
                pair: "fotos".into(),
                finished_at: DAY,
                outcome: SyncOutcomeInfo::Failed {
                    message: "Nicht angemeldet".into(),
                },
            },
        ]
    }

    fn conflict_sample() -> Vec<ConflictInfo> {
        vec![ConflictInfo {
            pair: "main".into(),
            id: 4,
            path: "Bericht.pdf".into(),
            kind: ConflictKindInfo::BothModified,
            conflict_copy: Some("Bericht (Konflikt 1760000000).pdf".into()),
            detected_at: DAY,
        }]
    }

    fn trash_sample() -> Vec<TrashInfo> {
        vec![TrashInfo {
            pair: "main".into(),
            id: 9,
            path: "alt.txt".into(),
            side: TrashSide::Remote,
            trashed_at: DAY,
            expires_at: DAY + 30 * 86_400,
        }]
    }

    #[test]
    fn status_snapshots() {
        insta::assert_snapshot!("status_de", status(Lang::De, &status_sample()));
        insta::assert_snapshot!("status_en", status(Lang::En, &status_sample()));
        let logged_out = StatusInfo {
            account: None,
            dry_run: false,
            pairs: Vec::new(),
            ..status_sample()
        };
        insta::assert_snapshot!("status_logged_out_de", status(Lang::De, &logged_out));
    }

    #[test]
    fn sync_snapshots() {
        insta::assert_snapshot!("sync_de", sync_results(Lang::De, &sync_sample()));
        insta::assert_snapshot!("sync_en", sync_results(Lang::En, &sync_sample()));
    }

    #[test]
    fn conflict_and_trash_snapshots() {
        insta::assert_snapshot!("conflicts_de", conflicts(Lang::De, &conflict_sample()));
        insta::assert_snapshot!("conflicts_none_en", conflicts(Lang::En, &[]));
        insta::assert_snapshot!("trash_de", trash(Lang::De, &trash_sample(), "leer"));
        insta::assert_snapshot!("trash_en", trash(Lang::En, &trash_sample(), "empty"));
    }
}
