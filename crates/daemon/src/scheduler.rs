//! Zeitplan: wann ein Sync-Paar abgeglichen wird.
//!
//! - beim Start einmal,
//! - nach lokalen Änderungen (Signal vom Watcher, mit Wartezeit zum Bündeln),
//! - wenn Proton neue Ereignisse meldet (Abfrage im festen Abstand),
//! - als Sicherheitsnetz im langen Abstand mit vollständig gelesenem entferntem Baum.
//!
//! Alle anderen Abgleiche nutzen den zwischengespeicherten Baum und holen nur
//! die Proton-Ereignisse nach.
//!
//! Dazu leert eine eigene Aufgabe täglich abgelaufene Papierkorbeinträge.
//!
//! Upload-Jobs laufen je in einer eigenen Aufgabe:
//! - mit Cron-Zeitplan zur nächsten passenden Ortszeit (verpasste Termine,
//!   etwa weil der Rechner aus war, werden nicht nachgeholt),
//! - „bei neuer Datei“ beim Start, nach Änderungen im Quellordner (Watcher)
//!   und alle fünf Minuten als Sicherheitsnetz für Netzlaufwerke, auf denen
//!   inotify nichts meldet. Proton wird nur gefragt, wenn sich passende Dateien
//!   im Quellordner geändert haben.
//!
//! Scheitert ein Lauf, folgen Wiederholungen nach 1, 2, 4, … Minuten (höchstens
//! eine Stunde), bis `max_attempts` erreicht ist.

use std::sync::Arc;
use std::time::Duration;

use protonsync_application::ports::{Account, Clock, JobLog, LocalStorage, StateRepo};
use protonsync_domain::job::{CronSchedule, JobTrigger};
use protonsync_domain::time::Timestamp;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep};

use crate::config::Timing;
use crate::core::{Core, JobAttempt};

/// Abstand der Papierkorb-Leerung.
const PURGE_INTERVAL: Duration = Duration::from_hours(24);
/// Erste Papierkorb-Leerung nach dem Start (nicht sofort, um den Start zu entlasten).
const FIRST_PURGE_DELAY: Duration = Duration::from_mins(5);
/// Abstand, in dem Jobs „bei neuer Datei“ den Quellordner ohne Watcher-Signal prüfen.
const NEW_FILE_POLL: Duration = Duration::from_mins(5);
/// Wartezeit nach Änderungen im Quellordner, bevor geprüft wird.
const SOURCE_DEBOUNCE: Duration = Duration::from_secs(5);
/// Spielraum nach der Ruhezeit, damit die Datei sicher als ruhig gilt.
const SETTLE_MARGIN: Duration = Duration::from_secs(1);
/// Erste Wartezeit vor einer Wiederholung; verdoppelt sich je Versuch.
const RETRY_BASE: Duration = Duration::from_mins(1);
/// Längste Wartezeit vor einer Wiederholung.
const RETRY_MAX: Duration = Duration::from_hours(1);
/// Lange Wartezeiten bis zu einem Cron-Termin werden in diesen Schritten
/// geschlafen und an der Uhr geprüft, damit Ruhezustand und Uhrstellen nicht
/// zu verspäteten Läufen führen.
const WALL_CLOCK_CHECK: Duration = Duration::from_mins(1);

/// Was nach einem Anlass zu tun ist.
enum Run {
    /// Abgleich mit zwischengespeichertem entferntem Baum.
    Cached,
    /// Abgleich mit vollständig gelesenem Baum.
    Full,
    /// Nichts.
    Skip,
}

/// Läuft, bis `shutdown` auf `true` wechselt.
pub async fn run<A, L, S, C>(
    core: Arc<Core<A, L, S, C>>,
    timing: Timing,
    shutdown: watch::Receiver<bool>,
) where
    A: Account + 'static,
    A::Remote: 'static,
    L: LocalStorage + 'static,
    S: StateRepo + JobLog + 'static,
    C: Clock + 'static,
{
    let mut tasks = JoinSet::new();
    for index in 0..core.pair_count() {
        tasks.spawn(pair_loop(
            Arc::clone(&core),
            index,
            timing,
            shutdown.clone(),
        ));
    }
    for index in 0..core.job_count() {
        tasks.spawn(job_loop(Arc::clone(&core), index, shutdown.clone()));
    }
    tasks.spawn(purge_loop(Arc::clone(&core), shutdown));
    while tasks.join_next().await.is_some() {}
}

async fn pair_loop<A, L, S, C>(
    core: Arc<Core<A, L, S, C>>,
    index: usize,
    timing: Timing,
    mut shutdown: watch::Receiver<bool>,
) where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: StateRepo + JobLog,
    C: Clock,
{
    let Some(trigger) = core.trigger(index) else {
        return;
    };
    core.scheduled_sync(index).await;

    let now = Instant::now();
    let mut poll = interval_at(now + timing.event_poll, timing.event_poll);
    let mut full = interval_at(now + timing.full_sync, timing.full_sync);
    // Nach einer Pause (z. B. Ruhezustand) nicht alle verpassten Takte nachholen.
    poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
    full.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        let run = tokio::select! {
            _ = shutdown.changed() => break,
            () = trigger.notified() => {
                // Weitere Änderungen in der Wartezeit verlängern sie, damit z. B.
                // ein Kopiervorgang mit vielen Dateien nur einen Abgleich auslöst.
                loop {
                    tokio::select! {
                        () = trigger.notified() => {}
                        () = sleep(timing.local_debounce) => break,
                    }
                }
                Run::Cached
            }
            _ = poll.tick() => if core.remote_changed(index).await { Run::Cached } else { Run::Skip },
            _ = full.tick() => Run::Full,
        };
        match run {
            Run::Cached => drop(core.scheduled_sync(index).await),
            Run::Full => drop(core.scheduled_full_sync(index).await),
            Run::Skip => {}
        }
    }
}

async fn purge_loop<A, L, S, C>(core: Arc<Core<A, L, S, C>>, mut shutdown: watch::Receiver<bool>)
where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: StateRepo + JobLog,
    C: Clock,
{
    let mut ticks = interval_at(Instant::now() + FIRST_PURGE_DELAY, PURGE_INTERVAL);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = ticks.tick() => core.scheduled_purge().await,
        }
    }
}

async fn job_loop<A, L, S, C>(
    core: Arc<Core<A, L, S, C>>,
    index: usize,
    shutdown: watch::Receiver<bool>,
) where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: StateRepo + JobLog,
    C: Clock,
{
    let Some(job) = core.job(index) else {
        return;
    };
    match job.trigger.clone() {
        JobTrigger::Schedule(schedule) => cron_loop(&core, index, &schedule, shutdown).await,
        JobTrigger::OnNewFile => new_file_loop(&core, index, shutdown).await,
    }
}

async fn cron_loop<A, L, S, C>(
    core: &Core<A, L, S, C>,
    index: usize,
    schedule: &CronSchedule,
    mut shutdown: watch::Receiver<bool>,
) where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: StateRepo + JobLog,
    C: Clock,
{
    loop {
        let Some(next) = core.next_job_run(index) else {
            tracing::warn!(
                schedule = schedule.expression(),
                "Zeitplan ohne nächsten Termin"
            );
            return;
        };
        if !sleep_until_wall_clock(core, next, &mut shutdown).await {
            return;
        }
        if run_with_retries(core, index, false, &mut shutdown)
            .await
            .is_none()
        {
            return;
        }
    }
}

async fn new_file_loop<A, L, S, C>(
    core: &Core<A, L, S, C>,
    index: usize,
    mut shutdown: watch::Receiver<bool>,
) where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: StateRepo + JobLog,
    C: Clock,
{
    let (Some(job), Some(trigger)) = (core.job(index), core.job_trigger(index)) else {
        return;
    };
    let settle = Duration::from_secs(u64::from(job.settle_seconds)) + SETTLE_MARGIN;
    // Beim Start prüfen: Dateien können entstanden sein, während der Daemon aus war.
    let mut only_if_changed = true;
    loop {
        let recheck = match run_with_retries(core, index, only_if_changed, &mut shutdown).await {
            None => return,
            // Noch nicht zur Ruhe gekommene Dateien nach der Ruhezeit erneut versuchen.
            Some(JobAttempt::Done { waiting: true }) => Some(Instant::now() + settle),
            Some(_) => None,
        };
        only_if_changed = tokio::select! {
            _ = shutdown.changed() => return,
            () = trigger.notified() => {
                loop {
                    tokio::select! {
                        () = trigger.notified() => {}
                        () = sleep(SOURCE_DEBOUNCE) => break,
                    }
                }
                true
            }
            () = sleep(NEW_FILE_POLL) => true,
            () = tokio::time::sleep_until(recheck.unwrap_or_else(Instant::now)), if recheck.is_some() => false,
        };
    }
}

/// Führt einen Job aus und wiederholt ihn bei Fehlern mit wachsendem Abstand.
/// `None`, wenn währenddessen der Daemon beendet wird.
async fn run_with_retries<A, L, S, C>(
    core: &Core<A, L, S, C>,
    index: usize,
    only_if_changed: bool,
    shutdown: &mut watch::Receiver<bool>,
) -> Option<JobAttempt>
where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: StateRepo + JobLog,
    C: Clock,
{
    let max_attempts = core.job(index)?.max_attempts;
    let mut attempt = 1;
    loop {
        let result = core
            .scheduled_job(index, attempt, attempt == 1 && only_if_changed)
            .await;
        match result {
            JobAttempt::Failed { .. } if attempt < max_attempts => {
                if !wait(retry_delay(attempt), shutdown).await {
                    return None;
                }
                attempt += 1;
            }
            other => return Some(other),
        }
    }
}

/// Wartezeit vor dem Versuch `attempt + 1`.
fn retry_delay(attempt: u32) -> Duration {
    let factor = 2u32.saturating_pow(attempt.saturating_sub(1));
    RETRY_BASE.saturating_mul(factor).min(RETRY_MAX)
}

/// Schläft bis zur Uhrzeit `target`; `false`, wenn der Daemon beendet wird.
async fn sleep_until_wall_clock<A, L, S, C>(
    core: &Core<A, L, S, C>,
    target: Timestamp,
    shutdown: &mut watch::Receiver<bool>,
) -> bool
where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: StateRepo + JobLog,
    C: Clock,
{
    loop {
        let remaining = target.unix_seconds() - core.now().unix_seconds();
        let Ok(remaining) = u64::try_from(remaining) else {
            return true;
        };
        if remaining == 0 {
            return true;
        }
        if !wait(
            Duration::from_secs(remaining).min(WALL_CLOCK_CHECK),
            shutdown,
        )
        .await
        {
            return false;
        }
    }
}

/// Wartet `duration`; `false`, wenn der Daemon währenddessen beendet wird.
async fn wait(duration: Duration, shutdown: &mut watch::Receiver<bool>) -> bool {
    tokio::select! {
        _ = shutdown.changed() => false,
        () = sleep(duration) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_back_off_exponentially_up_to_an_hour() {
        let minutes: Vec<u64> = (1..=9).map(|a| retry_delay(a).as_secs() / 60).collect();
        assert_eq!(minutes, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(retry_delay(u32::MAX), RETRY_MAX);
    }
}
