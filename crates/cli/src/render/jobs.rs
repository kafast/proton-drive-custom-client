//! Text output for upload jobs.

use std::fmt::Write;

use jiff::tz::TimeZone;
use protonsync_api::{JobDetails, JobInfo, JobOutcomeInfo, JobRunInfo, JobVerification};

use crate::i18n::{Lang, format_date_time};

/// Overview of all jobs.
#[must_use]
pub fn jobs(lang: Lang, zone: &TimeZone, jobs: &[JobInfo]) -> String {
    if jobs.is_empty() {
        return format!(
            "{}\n",
            lang.pick(
                "Keine Upload-Jobs konfiguriert ([[job]] in der Konfiguration).",
                "No upload jobs configured ([[job]] in the configuration)."
            )
        );
    }
    let mut out = String::new();
    for job in jobs {
        let _ = writeln!(
            out,
            "[{}] {}/{} -> {}",
            job.id, job.source, job.pattern, job.target
        );
        let schedule = if job.schedule == "on_new_file" {
            lang.pick("bei neuer Datei", "on new file").to_owned()
        } else {
            format!("cron \"{}\"", job.schedule)
        };
        let _ = writeln!(
            out,
            "  {}: {schedule}, {}: {}, {}: {}",
            lang.pick("Zeitplan", "Schedule"),
            lang.pick("Aufbewahrung", "Retention"),
            retention(lang, job),
            lang.pick("Prüfung", "Verification"),
            match job.verify {
                JobVerification::Size => lang.pick("Größe", "size"),
                JobVerification::Hash => lang.pick("Größe und SHA-1", "size and SHA-1"),
            }
        );
        if job.running {
            let _ = writeln!(out, "  {}", lang.pick("Läuft gerade", "Running now"));
        }
        let last = job.last_run.as_ref().map_or_else(
            || lang.pick("noch keiner", "none yet").to_owned(),
            |run| run_line(lang, zone, run),
        );
        let _ = writeln!(out, "  {}: {last}", lang.pick("Letzter Lauf", "Last run"));
        if let Some(next) = job.next_run {
            let _ = writeln!(
                out,
                "  {}: {}",
                lang.pick("Nächster Lauf", "Next run"),
                format_date_time(next, zone)
            );
        }
    }
    out
}

fn retention(lang: Lang, job: &JobInfo) -> String {
    let mut parts = Vec::new();
    if let Some(keep) = job.keep_last {
        parts.push(format!("{} {keep}", lang.pick("die neuesten", "newest")));
    }
    if let Some(days) = job.max_age_days {
        parts.push(format!(
            "{} {days} {}",
            lang.pick("höchstens", "at most"),
            lang.pick("Tage", "days")
        ));
    }
    if parts.is_empty() {
        lang.pick("alles behalten", "keep everything").to_owned()
    } else {
        parts.join(lang.pick(" und ", " and "))
    }
}

fn run_line(lang: Lang, zone: &TimeZone, run: &JobRunInfo) -> String {
    format!(
        "{} {}",
        format_date_time(run.finished_at, zone),
        outcome(lang, &run.outcome)
    )
}

fn outcome(lang: Lang, outcome: &JobOutcomeInfo) -> String {
    match outcome {
        JobOutcomeInfo::Planned { uploads, expired } => format!(
            "{}: {uploads} {}, {expired} {}",
            lang.pick("Probelauf", "dry run"),
            lang.pick("hochzuladen", "to upload"),
            lang.pick("abgelaufen", "expired")
        ),
        JobOutcomeInfo::Succeeded {
            uploaded,
            unchanged,
            expired,
        } => format!(
            "{}: {uploaded} {}, {unchanged} {}, {expired} {}",
            lang.pick("erfolgreich", "succeeded"),
            lang.pick("hochgeladen", "uploaded"),
            lang.pick("unverändert", "unchanged"),
            lang.pick("in den Papierkorb", "to trash")
        ),
        JobOutcomeInfo::Failed { message, attempt } => match lang {
            Lang::De => format!("FEHLER ({attempt}. Versuch): {message}"),
            Lang::En => format!("FAILED (attempt {attempt}): {message}"),
        },
    }
}

/// Result of a run that was just executed.
#[must_use]
pub fn job_run(lang: Lang, zone: &TimeZone, run: &JobRunInfo, details: &JobDetails) -> String {
    let mut out = format!("[{}] {}\n", run.job, run_line(lang, zone, run));
    let planned = matches!(run.outcome, JobOutcomeInfo::Planned { .. });
    let list = |out: &mut String, label: &str, names: &[String]| {
        for name in names {
            let _ = writeln!(out, "  {label}: {name}");
        }
    };
    list(
        &mut out,
        if planned {
            lang.pick("hochladen", "upload")
        } else {
            lang.pick("hochgeladen", "uploaded")
        },
        &details.uploads,
    );
    list(
        &mut out,
        lang.pick("wartet (Ruhezeit)", "waiting (settle time)"),
        &details.waiting,
    );
    list(
        &mut out,
        if planned {
            lang.pick("in den Papierkorb", "to trash")
        } else {
            lang.pick("im Papierkorb", "trashed")
        },
        &details.expired,
    );
    for failure in &details.failures {
        let _ = writeln!(
            out,
            "  {}: {}: {}",
            lang.pick("Fehler", "Error"),
            failure.path,
            failure.message
        );
    }
    if planned {
        let _ = writeln!(
            out,
            "{}",
            lang.pick(
                "Nur geplant. Mit --apply wirklich ausführen.",
                "Planned only. Use --apply to run it."
            )
        );
    }
    out
}

/// History of a job.
#[must_use]
pub fn history(lang: Lang, zone: &TimeZone, runs: &[JobRunInfo]) -> String {
    if runs.is_empty() {
        return format!("{}\n", lang.pick("Noch keine Läufe.", "No runs yet."));
    }
    runs.iter()
        .map(|run| run_line(lang, zone, run) + "\n")
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use protonsync_api::FailureInfo;

    const AT: i64 = 1_760_000_000;

    fn run(outcome: JobOutcomeInfo) -> JobRunInfo {
        JobRunInfo {
            job: "ha".into(),
            started_at: AT - 60,
            finished_at: AT,
            outcome,
        }
    }

    fn job_samples() -> Vec<JobInfo> {
        vec![
            JobInfo {
                id: "ha".into(),
                source: "/mnt/ha".into(),
                pattern: "*.tar".into(),
                target: "Backups/HA".into(),
                schedule: "0 3 * * *".into(),
                keep_last: Some(14),
                max_age_days: Some(90),
                verify: JobVerification::Hash,
                running: false,
                last_run: Some(run(JobOutcomeInfo::Succeeded {
                    uploaded: 1,
                    unchanged: 13,
                    expired: 1,
                })),
                next_run: Some(AT + 3600),
            },
            JobInfo {
                id: "fotos".into(),
                source: "/srv/fotos".into(),
                pattern: "*".into(),
                target: "Fotos".into(),
                schedule: "on_new_file".into(),
                keep_last: None,
                max_age_days: None,
                verify: JobVerification::Size,
                running: true,
                last_run: None,
                next_run: None,
            },
        ]
    }

    #[test]
    fn job_list_snapshots() {
        let utc = TimeZone::UTC;
        insta::assert_snapshot!("jobs_de", jobs(Lang::De, &utc, &job_samples()));
        insta::assert_snapshot!("jobs_en", jobs(Lang::En, &utc, &job_samples()));
        insta::assert_snapshot!("jobs_none_de", jobs(Lang::De, &utc, &[]));
    }

    #[test]
    fn run_and_history_snapshots() {
        let utc = TimeZone::UTC;
        let planned = run(JobOutcomeInfo::Planned {
            uploads: 1,
            expired: 1,
        });
        let details = JobDetails {
            uploads: vec!["b.tar".into()],
            waiting: vec!["c.tar".into()],
            expired: vec!["a.tar".into()],
            failures: Vec::new(),
        };
        insta::assert_snapshot!(
            "job_run_planned_de",
            job_run(Lang::De, &utc, &planned, &details)
        );
        let failed = run(JobOutcomeInfo::Failed {
            message: "1 of 2 files failed".into(),
            attempt: 2,
        });
        let details = JobDetails {
            uploads: vec!["b.tar".into()],
            failures: vec![FailureInfo {
                path: "c.tar".into(),
                message: "network".into(),
            }],
            ..JobDetails::default()
        };
        insta::assert_snapshot!(
            "job_run_failed_en",
            job_run(Lang::En, &utc, &failed, &details)
        );
        insta::assert_snapshot!(
            "job_history_de",
            history(Lang::De, &utc, &[failed, planned])
        );
        insta::assert_snapshot!("job_history_none_en", history(Lang::En, &utc, &[]));
    }
}
