//! Zeitplan: wann ein Sync-Paar abgeglichen wird.
//!
//! - beim Start einmal,
//! - nach lokalen Änderungen (Signal vom Watcher, mit Wartezeit zum Bündeln),
//! - wenn Proton neue Ereignisse meldet (Abfrage im festen Abstand),
//! - als Sicherheitsnetz im langen Abstand vollständig.
//!
//! Dazu leert eine eigene Aufgabe täglich abgelaufene Papierkorbeinträge.

use std::sync::Arc;
use std::time::Duration;

use protonsync_application::ports::{Account, Clock, LocalStorage, StateRepo};
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep};

use crate::config::Timing;
use crate::core::Core;

/// Abstand der Papierkorb-Leerung.
const PURGE_INTERVAL: Duration = Duration::from_hours(24);
/// Erste Papierkorb-Leerung nach dem Start (nicht sofort, um den Start zu entlasten).
const FIRST_PURGE_DELAY: Duration = Duration::from_mins(5);

/// Läuft, bis `shutdown` auf `true` wechselt.
pub async fn run<A, L, S, C>(
    core: Arc<Core<A, L, S, C>>,
    timing: Timing,
    shutdown: watch::Receiver<bool>,
) where
    A: Account + 'static,
    A::Remote: 'static,
    L: LocalStorage + 'static,
    S: StateRepo + 'static,
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
    S: StateRepo,
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
                true
            }
            _ = poll.tick() => core.remote_changed(index).await,
            _ = full.tick() => true,
        };
        if run {
            core.scheduled_sync(index).await;
        }
    }
}

async fn purge_loop<A, L, S, C>(core: Arc<Core<A, L, S, C>>, mut shutdown: watch::Receiver<bool>)
where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: StateRepo,
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
