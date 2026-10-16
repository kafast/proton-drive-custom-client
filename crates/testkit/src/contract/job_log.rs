//! Contract-Tests für [`JobLog`].
//!
//! ```ignore
//! protonsync_testkit::job_log_contract_tests!(protonsync_testkit::InMemoryStateRepo::new);
//! ```

use protonsync_application::ports::{JOB_HISTORY_LIMIT, JobLog};
use protonsync_domain::job::{JobId, JobOutcome, JobRun};
use protonsync_domain::time::Timestamp;

fn job(id: &str) -> JobId {
    JobId::parse(id).unwrap()
}

fn run(started: i64, outcome: JobOutcome) -> JobRun {
    JobRun {
        started_at: Timestamp::from_unix_seconds(started),
        finished_at: Timestamp::from_unix_seconds(started + 5),
        outcome,
    }
}

/// Alle Ergebnisarten kommen unverändert zurück, neueste zuerst, je Job getrennt.
pub fn job_runs_roundtrip(log: &impl JobLog) {
    let runs = [
        run(
            1,
            JobOutcome::Planned {
                uploads: 2,
                expired: 1,
            },
        ),
        run(
            2,
            JobOutcome::Succeeded {
                uploaded: 3,
                unchanged: 4,
                expired: 5,
            },
        ),
        run(
            3,
            JobOutcome::Failed {
                message: "Netz weg".into(),
                attempt: 2,
            },
        ),
    ];
    for entry in &runs {
        log.record_job_run(&job("ha"), entry).unwrap();
    }
    log.record_job_run(&job("anderer"), &runs[0]).unwrap();

    let newest_first: Vec<JobRun> = runs.iter().rev().cloned().collect();
    assert_eq!(log.job_runs(&job("ha"), 10).unwrap(), newest_first);
    assert_eq!(log.job_runs(&job("ha"), 1).unwrap(), newest_first[..1]);
    assert_eq!(log.job_runs(&job("anderer"), 10).unwrap().len(), 1);
    assert_eq!(log.job_runs(&job("unbekannt"), 10).unwrap(), Vec::new());
}

/// Der Verlauf ist begrenzt; die ältesten Einträge fallen weg.
pub fn job_history_is_limited(log: &impl JobLog) {
    let total = i64::try_from(JOB_HISTORY_LIMIT).unwrap() + 3;
    for started in 0..total {
        log.record_job_run(
            &job("ha"),
            &run(
                started,
                JobOutcome::Planned {
                    uploads: 0,
                    expired: 0,
                },
            ),
        )
        .unwrap();
    }
    let kept = log.job_runs(&job("ha"), usize::MAX).unwrap();
    assert_eq!(kept.len(), JOB_HISTORY_LIMIT);
    assert_eq!(kept[0].started_at.unix_seconds(), total - 1);
    assert_eq!(kept.last().unwrap().started_at.unix_seconds(), 3);
}

/// Erzeugt für jeden Contract-Test eine `#[test]`-Funktion.
#[macro_export]
macro_rules! job_log_contract_tests {
    ($factory:expr) => {
        $crate::job_log_contract_tests!(@tests $factory; job_runs_roundtrip, job_history_is_limited);
    };
    (@tests $factory:expr; $($test:ident),+) => {
        $(
            #[test]
            fn $test() {
                let log = ($factory)();
                $crate::contract::job_log::$test(&log);
            }
        )+
    };
}
