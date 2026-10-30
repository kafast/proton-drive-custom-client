//! Schedule: when a sync pair is synced.
//!
//! - once at startup,
//! - after local changes (signal from the watcher, with a delay to batch them),
//! - when Proton reports new events (polled at a fixed interval),
//! - as a safety net at a long interval with a fully read remote tree.
//!
//! All other syncs use the cached tree and only catch up on the Proton
//! events.
//!
//! In addition, a separate task purges expired trash entries daily, and
//! another one checks the free disk space (right at startup, then at the
//! configured interval).
//!
//! FUSE drives are maintained at the event poll interval (apply Proton events,
//! retry pending uploads, load pinned files); every hour space is freed
//! according to the eviction policy.
//!
//! Upload jobs each run in their own task:
//! - with a cron schedule at the next matching local time; if the last run
//!   missed a due time (e.g. because the machine was off), one catch-up run
//!   starts right away (ADR 0008),
//! - "on new file" at startup, after changes in the source folder (watcher)
//!   and every five minutes as a safety net for network drives on which
//!   inotify reports nothing. Proton is only asked when matching files in the
//!   source folder have changed.
//!
//! The photo sync runs like a sync pair (startup, watcher, event poll), but
//! reads the library fully as a safety net only once a day, because that is
//! expensive with many photos (ADR 0012).
//!
//! If a run fails, retries follow after 1, 2, 4, … minutes (at most one
//! hour) until `max_attempts` is reached.

use std::sync::Arc;
use std::time::Duration;

use protonsync_application::ports::{Account, Clock, LocalStorage};
use protonsync_domain::job::{CronSchedule, JobTrigger};
use protonsync_domain::time::Timestamp;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep};

use crate::config::Timing;
use crate::core::{Core, DaemonState, JobAttempt};

/// Interval of the trash purge.
const PURGE_INTERVAL: Duration = Duration::from_hours(24);
/// First trash purge after startup (not immediately, to keep startup light).
const FIRST_PURGE_DELAY: Duration = Duration::from_mins(5);
/// Interval at which the photo library is read again in full.
const PHOTO_FULL_INTERVAL: Duration = Duration::from_hours(24);
/// Interval of freeing space in the drives.
const EVICTION_INTERVAL: Duration = Duration::from_hours(1);
/// Interval at which "on new file" jobs check the source folder without a watcher signal.
const NEW_FILE_POLL: Duration = Duration::from_mins(5);
/// Waiting time after changes in the source folder before checking.
const SOURCE_DEBOUNCE: Duration = Duration::from_secs(5);
/// Margin after the settle time, so that the file safely counts as settled.
const SETTLE_MARGIN: Duration = Duration::from_secs(1);
/// First waiting time before a retry; doubles with each attempt.
const RETRY_BASE: Duration = Duration::from_mins(1);
/// Longest waiting time before a retry.
const RETRY_MAX: Duration = Duration::from_hours(1);
/// Long waits until a cron time are slept in steps of this size and checked
/// against the clock, so that suspend and clock changes do not lead to late
/// runs.
const WALL_CLOCK_CHECK: Duration = Duration::from_mins(1);

/// What to do after a trigger.
enum Run {
    /// Sync with the cached remote tree.
    Cached,
    /// Sync with a fully read tree.
    Full,
    /// Nothing.
    Skip,
}

/// Runs until `shutdown` changes to `true`.
pub async fn run<A, L, S, C>(
    core: Arc<Core<A, L, S, C>>,
    timing: Timing,
    shutdown: watch::Receiver<bool>,
) where
    A: Account + 'static,
    A::Remote: 'static,
    L: LocalStorage + 'static,
    S: DaemonState,
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
    for index in 0..core.mount_count() {
        tasks.spawn(mount_loop(
            Arc::clone(&core),
            index,
            timing.event_poll,
            shutdown.clone(),
        ));
    }
    if core.photo_trigger().is_some() {
        tasks.spawn(photo_loop(Arc::clone(&core), timing, shutdown.clone()));
    }
    if core.mount_count() > 0 {
        tasks.spawn(eviction_loop(Arc::clone(&core), shutdown.clone()));
    }
    if let Some(every) = core.disk_check_interval() {
        tasks.spawn(disk_loop(Arc::clone(&core), every, shutdown.clone()));
    }
    tasks.spawn(purge_loop(Arc::clone(&core), shutdown));
    while tasks.join_next().await.is_some() {}
}

async fn mount_loop<A, L, S, C>(
    core: Arc<Core<A, L, S, C>>,
    index: usize,
    every: Duration,
    mut shutdown: watch::Receiver<bool>,
) where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: DaemonState,
    C: Clock + 'static,
{
    let mut ticks = interval_at(Instant::now() + every, every);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = ticks.tick() => core.maintain_mount(index).await,
        }
    }
}

async fn disk_loop<A, L, S, C>(
    core: Arc<Core<A, L, S, C>>,
    every: Duration,
    mut shutdown: watch::Receiver<bool>,
) where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: DaemonState,
    C: Clock + 'static,
{
    // The first tick comes right away, so a full disk is reported at startup.
    let mut ticks = interval_at(Instant::now(), every);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = ticks.tick() => core.check_disk_space(),
        }
    }
}

async fn eviction_loop<A, L, S, C>(core: Arc<Core<A, L, S, C>>, mut shutdown: watch::Receiver<bool>)
where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: DaemonState,
    C: Clock + 'static,
{
    let mut ticks = interval_at(Instant::now() + EVICTION_INTERVAL, EVICTION_INTERVAL);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = ticks.tick() => core.scheduled_eviction().await,
        }
    }
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
    S: DaemonState,
    C: Clock + 'static,
{
    let Some(trigger) = core.trigger(index) else {
        return;
    };
    core.scheduled_sync(index).await;

    let now = Instant::now();
    let mut poll = interval_at(now + timing.event_poll, timing.event_poll);
    let mut full = interval_at(now + timing.full_sync, timing.full_sync);
    // After a pause (e.g. suspend) do not catch up on all missed ticks.
    poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
    full.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        let run = tokio::select! {
            _ = shutdown.changed() => break,
            () = trigger.notified() => {
                // Further changes during the wait extend it, so that e.g. copying
                // many files triggers only one sync.
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

async fn photo_loop<A, L, S, C>(
    core: Arc<Core<A, L, S, C>>,
    timing: Timing,
    mut shutdown: watch::Receiver<bool>,
) where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: DaemonState,
    C: Clock + 'static,
{
    let Some(trigger) = core.photo_trigger() else {
        return;
    };
    drop(core.scheduled_photo_sync(false).await);

    let now = Instant::now();
    let mut poll = interval_at(now + timing.event_poll, timing.event_poll);
    let mut full = interval_at(now + PHOTO_FULL_INTERVAL, PHOTO_FULL_INTERVAL);
    poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
    full.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        let run = tokio::select! {
            _ = shutdown.changed() => break,
            () = trigger.notified() => {
                loop {
                    tokio::select! {
                        () = trigger.notified() => {}
                        () = sleep(timing.local_debounce) => break,
                    }
                }
                Run::Cached
            }
            _ = poll.tick() => if core.photos_changed().await { Run::Cached } else { Run::Skip },
            _ = full.tick() => Run::Full,
        };
        match run {
            Run::Cached => drop(core.scheduled_photo_sync(false).await),
            Run::Full => drop(core.scheduled_photo_sync(true).await),
            Run::Skip => {}
        }
    }
}

async fn purge_loop<A, L, S, C>(core: Arc<Core<A, L, S, C>>, mut shutdown: watch::Receiver<bool>)
where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: DaemonState,
    C: Clock + 'static,
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
    S: DaemonState,
    C: Clock + 'static,
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
    S: DaemonState,
    C: Clock + 'static,
{
    if core.missed_job_run(index) {
        tracing::info!(
            schedule = schedule.expression(),
            "catching up on a missed run"
        );
        if run_with_retries(core, index, false, &mut shutdown)
            .await
            .is_none()
        {
            return;
        }
    }
    loop {
        let Some(next) = core.next_job_run(index) else {
            tracing::warn!(schedule = schedule.expression(), "schedule has no next run");
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
    S: DaemonState,
    C: Clock + 'static,
{
    let (Some(job), Some(trigger)) = (core.job(index), core.job_trigger(index)) else {
        return;
    };
    let settle = Duration::from_secs(u64::from(job.settle_seconds)) + SETTLE_MARGIN;
    // Check at startup: files may have appeared while the daemon was off.
    let mut only_if_changed = true;
    loop {
        let recheck = match run_with_retries(core, index, only_if_changed, &mut shutdown).await {
            None => return,
            // Retry files that have not settled yet after the settle time.
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

/// Runs a job and retries it on failure with a growing interval.
/// `None` if the daemon shuts down meanwhile.
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
    S: DaemonState,
    C: Clock + 'static,
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

/// Waiting time before attempt `attempt + 1`.
fn retry_delay(attempt: u32) -> Duration {
    let factor = 2u32.saturating_pow(attempt.saturating_sub(1));
    RETRY_BASE.saturating_mul(factor).min(RETRY_MAX)
}

/// Sleeps until the wall-clock time `target`; `false` if the daemon shuts down.
async fn sleep_until_wall_clock<A, L, S, C>(
    core: &Core<A, L, S, C>,
    target: Timestamp,
    shutdown: &mut watch::Receiver<bool>,
) -> bool
where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: DaemonState,
    C: Clock + 'static,
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

/// Waits `duration`; `false` if the daemon shuts down meanwhile.
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
