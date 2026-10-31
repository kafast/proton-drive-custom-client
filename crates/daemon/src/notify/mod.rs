//! Notifications by email and webhook, the weekly report (ADR 0017) and the
//! daily digest of the change log (ADR 0024).
//!
//! The notifier is one more listener on the daemon events, like the tray app:
//! [`Notifications`] decides from each event whether and what to report,
//! [`Dispatcher`] hands the message to every channel that wants the topic, and
//! [`run`] ties both to the event stream and the weekly and daily schedules.
//!
//! So that nobody is flooded:
//! - conflicts arriving within [`CONFLICT_BATCH_WINDOW`] go out as one message,
//! - a failing sync pair is reported once, and again only after it worked in between,
//! - an expired login is reported once until a sync works again,
//! - a disk is reported only when its level changes (almost full, full, ok again).

mod tally;
pub mod texts;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use protonsync_api::{
    ChangeInfo, ConflictInfo, Event, PhotoOutcomeInfo, Request, Response, SyncOutcomeInfo,
};
use protonsync_application::ports::{Clock, Message, Notifier};
use protonsync_client::Lang;
use protonsync_domain::job::CronSchedule;
use protonsync_domain::time::{SECONDS_PER_DAY, Timestamp};
use protonsync_web::Backend;
use tokio::sync::{broadcast, watch};
use tokio::time::Instant;

pub use tally::{JobTally, PairTally, PhotoTally, WeekTally};
pub use texts::ReportSnapshot;

use crate::core::MAX_CHANGES_PER_QUERY;
use crate::local_time::LocalZone;
use crate::notify_config::Topics;

/// Conflicts arriving this soon after each other are collected into one message.
pub const CONFLICT_BATCH_WINDOW: Duration = Duration::from_secs(60);
/// Waiting times before the second and third attempt of a temporary failure.
pub const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(10), Duration::from_secs(60)];
/// Name for the photo sync in failure messages.
const PHOTOS_NAME: &str = "photos";

/// What a message is about; every channel can switch each topic on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topic {
    /// New conflicts.
    Conflicts,
    /// A sync failed.
    SyncFailure,
    /// An upload job gave up.
    JobFailure,
    /// The login has to be renewed.
    LoginRequired,
    /// A disk is almost full, full, or has space again.
    DiskSpace,
    /// Weekly summary.
    WeeklyReport,
    /// Daily digest of the change log.
    DailyChanges,
}

impl Topics {
    /// Does this channel want messages about `topic`?
    #[must_use]
    pub const fn wants(&self, topic: Topic) -> bool {
        match topic {
            Topic::Conflicts => self.conflicts,
            Topic::SyncFailure => self.sync_failures,
            Topic::JobFailure => self.job_failures,
            Topic::LoginRequired => self.login_required,
            Topic::DiskSpace => self.disk_space,
            Topic::WeeklyReport => self.weekly_report,
            Topic::DailyChanges => self.daily_changes,
        }
    }
}

/// A message ready to be sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    /// Topic (decides which channels get it).
    pub topic: Topic,
    /// The message.
    pub message: Message,
}

/// Decides from the events what is reported (without sending anything itself).
pub struct Notifications {
    lang: Lang,
    zone: LocalZone,
    pending_conflicts: Vec<ConflictInfo>,
    /// Pairs (and the photo sync) whose failure has already been reported.
    failing: BTreeSet<String>,
    login_reported: bool,
    tally: WeekTally,
    /// Start of the period of the next daily digest; `None` before the first one.
    changes_since: Option<Timestamp>,
}

impl Notifications {
    /// Starts with empty counters at `since` (usually the start of the daemon).
    #[must_use]
    pub fn new(lang: Lang, zone: LocalZone, since: Timestamp) -> Self {
        Self {
            lang,
            zone,
            pending_conflicts: Vec::new(),
            failing: BTreeSet::new(),
            login_reported: false,
            tally: WeekTally::new(since),
            changes_since: None,
        }
    }

    /// Handles an event. Conflicts are only collected (see [`Self::take_conflicts`]).
    pub fn on_event(&mut self, event: &Event) -> Option<Outgoing> {
        self.tally.record(event);
        match event {
            Event::ConflictDetected(conflict) => {
                self.pending_conflicts.push(conflict.clone());
                None
            }
            Event::SyncFinished(result) => match &result.outcome {
                SyncOutcomeInfo::Failed { message } => self.failed(&result.pair, message),
                SyncOutcomeInfo::Applied(_) | SyncOutcomeInfo::Planned { .. } => {
                    self.recovered(&result.pair);
                    None
                }
            },
            Event::PhotoSyncFinished(run) => match &run.outcome {
                PhotoOutcomeInfo::Failed { message } => self.failed(PHOTOS_NAME, message),
                PhotoOutcomeInfo::Applied(_) | PhotoOutcomeInfo::Planned(_) => {
                    self.recovered(PHOTOS_NAME);
                    None
                }
            },
            Event::LoginRequired { message } => {
                if self.login_reported {
                    return None;
                }
                self.login_reported = true;
                Some(Outgoing {
                    topic: Topic::LoginRequired,
                    message: texts::login_required(self.lang, message),
                })
            }
            Event::JobGaveUp {
                job,
                message,
                attempts,
            } => Some(Outgoing {
                topic: Topic::JobFailure,
                message: texts::job_gave_up(self.lang, job, message, *attempts),
            }),
            Event::DiskSpaceChanged(info) => Some(Outgoing {
                topic: Topic::DiskSpace,
                message: texts::disk_space(self.lang, info),
            }),
            Event::SyncStarted { .. }
            | Event::JobStarted { .. }
            | Event::JobFinished(_)
            | Event::PhotoSyncStarted => None,
        }
    }

    fn failed(&mut self, what: &str, message: &str) -> Option<Outgoing> {
        // Without a login every run fails; the login message says enough.
        if self.login_reported || !self.failing.insert(what.to_owned()) {
            return None;
        }
        Some(Outgoing {
            topic: Topic::SyncFailure,
            message: texts::sync_failed(self.lang, what, message),
        })
    }

    fn recovered(&mut self, what: &str) {
        self.failing.remove(what);
        self.login_reported = false;
    }

    /// Are conflicts waiting to be sent?
    #[must_use]
    pub fn has_pending_conflicts(&self) -> bool {
        !self.pending_conflicts.is_empty()
    }

    /// One message for all collected conflicts.
    pub fn take_conflicts(&mut self) -> Option<Outgoing> {
        if self.pending_conflicts.is_empty() {
            return None;
        }
        let conflicts = std::mem::take(&mut self.pending_conflicts);
        Some(Outgoing {
            topic: Topic::Conflicts,
            message: texts::conflicts(self.lang, &conflicts),
        })
    }

    /// The weekly report; afterwards the counters start again at `now`.
    pub fn weekly_report(&mut self, snapshot: &ReportSnapshot, now: Timestamp) -> Outgoing {
        let message = texts::weekly_report(self.lang, &self.zone, &self.tally, snapshot, now);
        self.tally = WeekTally::new(now);
        Outgoing {
            topic: Topic::WeeklyReport,
            message,
        }
    }
}

impl Notifications {
    /// Start of the period the next daily digest covers: the previous digest,
    /// or one day back for the first one.
    #[must_use]
    pub fn changes_since(&self, now: Timestamp) -> Timestamp {
        self.changes_since.unwrap_or_else(|| {
            Timestamp::from_unix_seconds(now.unix_seconds().saturating_sub(SECONDS_PER_DAY))
        })
    }

    /// The daily digest of `changes` (newest first, as the API delivers them);
    /// the next digest starts at `now`.
    pub fn daily_changes(
        &mut self,
        changes: &[ChangeInfo],
        keep_days: u32,
        now: Timestamp,
    ) -> Outgoing {
        let digest = texts::ChangeDigest {
            since: self.changes_since(now),
            until: now,
            changes,
            keep_days,
            truncated: changes.len()
                >= usize::try_from(MAX_CHANGES_PER_QUERY).unwrap_or(usize::MAX),
        };
        let message = texts::daily_changes(self.lang, &self.zone, &digest);
        self.changes_since = Some(now);
        Outgoing {
            topic: Topic::DailyChanges,
            message,
        }
    }
}

/// A channel together with the topics it wants.
pub struct Route<N> {
    /// The channel.
    pub notifier: N,
    /// What it gets.
    pub topics: Topics,
}

/// Hands messages to the channels.
pub struct Dispatcher<N> {
    routes: Vec<Route<N>>,
    retry_delays: Vec<Duration>,
}

impl<N: Notifier> Dispatcher<N> {
    /// Dispatcher with the usual waiting times between attempts ([`RETRY_DELAYS`]).
    #[must_use]
    pub fn new(routes: Vec<Route<N>>) -> Self {
        Self::with_retry_delays(routes, RETRY_DELAYS.to_vec())
    }

    /// Dispatcher with own waiting times (one more attempt per entry).
    #[must_use]
    pub fn with_retry_delays(routes: Vec<Route<N>>, retry_delays: Vec<Duration>) -> Self {
        Self {
            routes,
            retry_delays,
        }
    }

    /// Does any channel want this topic?
    #[must_use]
    pub fn wants(&self, topic: Topic) -> bool {
        self.routes.iter().any(|route| route.topics.wants(topic))
    }

    /// Sends `outgoing` to every channel that wants it. Failures are logged:
    /// a notification must never stop the sync.
    pub async fn deliver(&self, outgoing: &Outgoing) {
        for route in &self.routes {
            if route.topics.wants(outgoing.topic) {
                self.send_with_retries(&route.notifier, &outgoing.message)
                    .await;
            }
        }
    }

    async fn send_with_retries(&self, notifier: &N, message: &Message) {
        let mut delays = self.retry_delays.iter();
        loop {
            match notifier.send(message).await {
                Ok(()) => {
                    tracing::info!(channel = notifier.channel(), subject = %message.subject, "notification sent");
                    return;
                }
                Err(error) => match delays.next() {
                    Some(delay) if error.is_temporary() => {
                        tracing::info!(channel = notifier.channel(), %error, "notification failed, trying again");
                        tokio::time::sleep(*delay).await;
                    }
                    _ => {
                        tracing::warn!(channel = notifier.channel(), %error, "notification not sent");
                        return;
                    }
                },
            }
        }
    }
}

/// Collects the current state for the weekly report through the daemon API.
pub async fn report_snapshot<B: Backend>(backend: &B) -> ReportSnapshot {
    let mut snapshot = ReportSnapshot::default();
    if let Response::Status(status) = backend.handle(Request::Status).await {
        snapshot.account = status.account;
        snapshot.dry_run = status.dry_run;
        snapshot.last_syncs = status
            .pairs
            .into_iter()
            .map(|pair| (pair.id, pair.last_sync))
            .collect();
    }
    if let Response::Conflicts { conflicts } = backend.handle(Request::Conflicts).await {
        snapshot.conflicts = conflicts;
    }
    if let Response::Trash { entries } = backend.handle(Request::Trash).await {
        snapshot.trash = entries;
    }
    if let Response::Jobs { jobs } = backend.handle(Request::Jobs).await {
        snapshot.jobs = jobs;
    }
    snapshot
}

/// Reads the changes since `since` through the daemon API; `None` if the
/// daemon cannot answer (the digest is then skipped, not sent empty).
pub async fn read_changes<B: Backend>(
    backend: &B,
    since: Timestamp,
) -> Option<(Vec<ChangeInfo>, u32)> {
    let request = Request::Changes {
        since: Some(since.unix_seconds()),
        pair: None,
        limit: MAX_CHANGES_PER_QUERY,
    };
    match backend.handle(request).await {
        Response::Changes { changes, keep_days } => Some((changes, keep_days)),
        other => {
            tracing::warn!(response = ?other, "change log not readable, daily digest skipped");
            None
        }
    }
}

/// Everything [`run`] needs besides the channels.
pub struct NotifyLoop<B, C> {
    /// The daemon (for the weekly report).
    pub backend: Arc<B>,
    /// Clock.
    pub clock: Arc<C>,
    /// Time zone of the schedule.
    pub zone: LocalZone,
    /// When the weekly report goes out.
    pub report_schedule: CronSchedule,
    /// When the daily change digest goes out.
    pub daily_changes_schedule: CronSchedule,
    /// Decision logic with the counters.
    pub notifications: Notifications,
}

/// Listens to the events until `shutdown` and sends notifications.
pub async fn run<B, C, N>(
    mut setup: NotifyLoop<B, C>,
    mut events: broadcast::Receiver<Event>,
    dispatcher: Arc<Dispatcher<N>>,
    mut shutdown: watch::Receiver<bool>,
) where
    B: Backend,
    C: Clock + 'static,
    N: Notifier + 'static,
{
    let mut conflict_deadline: Option<Instant> = None;
    loop {
        let report_in = time_until(&setup, &setup.report_schedule);
        let digest_in = time_until(&setup, &setup.daily_changes_schedule);
        tokio::select! {
            _ = shutdown.changed() => break,
            received = events.recv() => match received {
                Ok(event) => {
                    if let Some(outgoing) = setup.notifications.on_event(&event) {
                        spawn_delivery(&dispatcher, outgoing);
                    }
                    if setup.notifications.has_pending_conflicts() && conflict_deadline.is_none() {
                        conflict_deadline = Some(Instant::now() + CONFLICT_BATCH_WINDOW);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "notifications missed events");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            () = sleep_until_option(conflict_deadline) => {
                conflict_deadline = None;
                if let Some(outgoing) = setup.notifications.take_conflicts() {
                    spawn_delivery(&dispatcher, outgoing);
                }
            }
            () = tokio::time::sleep(report_in) => {
                // Without a channel for the report the counters still restart weekly.
                let snapshot = report_snapshot(setup.backend.as_ref()).await;
                let outgoing = setup.notifications.weekly_report(&snapshot, setup.clock.now());
                if dispatcher.wants(Topic::WeeklyReport) {
                    spawn_delivery(&dispatcher, outgoing);
                }
            }
            () = tokio::time::sleep(digest_in) => {
                if dispatcher.wants(Topic::DailyChanges) {
                    send_daily_changes(&mut setup, &dispatcher).await;
                }
            }
        }
    }
    // Conflicts collected right before shutting down are not lost.
    if let Some(outgoing) = setup.notifications.take_conflicts() {
        dispatcher.deliver(&outgoing).await;
    }
}

async fn send_daily_changes<B, C, N>(setup: &mut NotifyLoop<B, C>, dispatcher: &Arc<Dispatcher<N>>)
where
    B: Backend,
    C: Clock + 'static,
    N: Notifier + 'static,
{
    let now = setup.clock.now();
    let since = setup.notifications.changes_since(now);
    if let Some((changes, keep_days)) = read_changes(setup.backend.as_ref(), since).await {
        let outgoing = setup.notifications.daily_changes(&changes, keep_days, now);
        spawn_delivery(dispatcher, outgoing);
    }
}

fn time_until<B, C: Clock>(setup: &NotifyLoop<B, C>, schedule: &CronSchedule) -> Duration {
    let now = setup.clock.now();
    let next = setup.zone.next_run(schedule, now);
    let seconds = next.map_or(i64::MAX, |next| next.unix_seconds() - now.unix_seconds());
    Duration::from_secs(u64::try_from(seconds).unwrap_or(0))
}

async fn sleep_until_option(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

fn spawn_delivery<N: Notifier + 'static>(dispatcher: &Arc<Dispatcher<N>>, outgoing: Outgoing) {
    let dispatcher = Arc::clone(dispatcher);
    tokio::spawn(async move { dispatcher.deliver(&outgoing).await });
}
