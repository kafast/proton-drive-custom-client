//! The business workflow of the daemon: answers API requests and runs
//! syncs. Generic over the ports, so that it can be tested with fakes.

use std::sync::Arc;

use protonsync_api::{
    ApiError, ErrorCode, Event, PairStatus, PairSyncResult, Request, Response, StatusInfo,
    SyncOutcomeInfo,
};
use protonsync_application::ports::{
    Account, AccountError, Clock, ConflictId, DriveRepo, JobLog, LocalStorage, RemoteError,
    RemoteStorage, StateRepo, TrashId,
};
use protonsync_application::sync::{RemoteCache, SyncContext, SyncError, SyncOutcome};
use protonsync_application::use_cases::{ConflictService, TrashService};
use protonsync_domain::pin::EvictionPolicy;
use protonsync_domain::sync_pair::{StorageMode, SyncPair, SyncPairId};
use protonsync_domain::trash::RetentionPeriod;
use tokio::sync::{Mutex, Notify, RwLock, broadcast};

use crate::config::PairConfig;
use crate::convert;
use crate::local_time::LocalZone;

mod files;
mod jobs;
mod mounts;
mod photos;
mod settings;

use jobs::JobRuntime;
pub use jobs::{JobAttempt, JobSetup};
use mounts::MountRuntime;
pub use mounts::{DaemonDrive, DriveMounter, MountGuard, MountSetup, SharedMounter};
use photos::PhotoRuntime;
pub use photos::PhotoSetup;
use settings::PairOptions;
pub use settings::SettingsFile;

/// What the core needs from the state store: sync state, job history and
/// the index of the drives. A shorthand for the three ports (like an
/// interface that extends others).
pub trait DaemonState: StateRepo + JobLog + DriveRepo + 'static {}

impl<T: StateRepo + JobLog + DriveRepo + 'static> DaemonState for T {}

/// How many events a slow subscriber may miss before it sees gaps.
const EVENT_BUFFER: usize = 64;

/// Settings the workflow needs.
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    /// Only plan automatic syncs and trash purges.
    pub dry_run: bool,
    /// Retention in the trash.
    pub retention: RetentionPeriod,
}

/// A sync pair during wiring: configuration and local storage.
pub struct PairSetup<L> {
    /// Configuration.
    pub config: PairConfig,
    /// Local storage of the pair.
    pub local: L,
}

/// Connection of a sync pair to Proton (only when logged in).
struct Connection<R> {
    remote: Arc<R>,
    pair: SyncPair,
    /// Cached remote tree; belongs to the connection, so that a new login
    /// always starts with a freshly read tree.
    cache: Mutex<RemoteCache>,
}

/// How a run reads the remote tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreeRead {
    /// Use the cache and only catch up on events.
    Cached,
    /// Read fully from Proton (safety net, manual sync).
    Full,
}

struct PairRuntime<R, L> {
    /// Configuration at startup; the editable options live in `options`.
    config: PairConfig,
    /// Options that can change at runtime (ADR 0021).
    options: std::sync::RwLock<PairOptions>,
    id: SyncPairId,
    local: L,
    connection: RwLock<Option<Arc<Connection<R>>>>,
    syncing: std::sync::atomic::AtomicBool,
    last_sync: std::sync::Mutex<Option<PairSyncResult>>,
    trigger: Notify,
}

/// The daemon core.
pub struct Core<A: Account, L, S, C> {
    account: A,
    // Shared (`Arc`), because the drives use the state and clock as well.
    state: Arc<S>,
    clock: Arc<C>,
    /// Can change at runtime (ADR 0021); read with [`Core::settings`].
    settings: std::sync::RwLock<Settings>,
    /// Where changed settings are saved; without it they cannot be changed.
    settings_file: Option<Arc<dyn SettingsFile>>,
    pairs: Vec<PairRuntime<A::Remote, L>>,
    jobs: Vec<JobRuntime<A::Remote, L>>,
    mounts: Vec<MountRuntime<A::Remote, S, C>>,
    /// Photo sync, if set up.
    photos: Option<PhotoRuntime<A::Photos, L>>,
    /// Eviction policy of the drives (only set with drives).
    eviction: Option<EvictionPolicy>,
    /// Mounts drives; without it they are only connected.
    mounter: Option<SharedMounter<A::Remote, S, C>>,
    /// Time zone for the cron schedules of the jobs and the month folders of the photos.
    zone: LocalZone,
    events: broadcast::Sender<Event>,
    // Only one sync at a time: spares Proton and avoids concurrent writes
    // to the state.
    sync_lock: Mutex<()>,
    // Likewise only one upload job at a time (independent of the syncs).
    job_lock: Mutex<()>,
}

/// Who a trash entry belongs to.
#[derive(Debug, Clone, Copy)]
enum TrashOwner {
    Pair(usize),
    Job(usize),
    Mount(usize),
    Photos,
}

fn not_logged_in() -> ApiError {
    ApiError::new(ErrorCode::NotLoggedIn, "not logged in")
}

fn unknown_pair(id: &str) -> ApiError {
    ApiError::new(ErrorCode::NotFound, format!("unknown sync pair {id}"))
}

impl<A, L, S, C> Core<A, L, S, C>
where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: DaemonState,
    C: Clock + 'static,
{
    /// Assembles the core.
    pub fn new(
        account: A,
        state: S,
        clock: C,
        settings: Settings,
        pairs: Vec<PairSetup<L>>,
    ) -> Self {
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        Self {
            account,
            state: Arc::new(state),
            clock: Arc::new(clock),
            settings: std::sync::RwLock::new(settings),
            settings_file: None,
            pairs: pairs
                .into_iter()
                .map(|setup| PairRuntime {
                    id: SyncPairId::new(setup.config.id.clone()),
                    options: std::sync::RwLock::new(PairOptions::from(&setup.config)),
                    config: setup.config,
                    local: setup.local,
                    connection: RwLock::new(None),
                    syncing: std::sync::atomic::AtomicBool::new(false),
                    last_sync: std::sync::Mutex::new(None),
                    trigger: Notify::new(),
                })
                .collect(),
            jobs: Vec::new(),
            mounts: Vec::new(),
            photos: None,
            eviction: None,
            mounter: None,
            zone: LocalZone::utc(),
            events,
            sync_lock: Mutex::new(()),
            job_lock: Mutex::new(()),
        }
    }

    /// Adds upload jobs; their schedules apply in the time zone `zone`.
    #[must_use]
    pub fn with_jobs(mut self, jobs: Vec<JobSetup<L>>, zone: LocalZone) -> Self {
        self.jobs = jobs.into_iter().map(JobRuntime::new).collect();
        self.zone = zone;
        self
    }

    /// Number of sync pairs (indices `0..pair_count()`).
    #[must_use]
    pub fn pair_count(&self) -> usize {
        self.pairs.len()
    }

    /// Signal that triggers a sync of the pair soon (e.g. on local changes).
    #[must_use]
    pub fn trigger(&self, index: usize) -> Option<&Notify> {
        self.pairs.get(index).map(|pair| &pair.trigger)
    }

    /// Subscribes to events.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    fn publish(&self, event: Event) {
        // An error only means: nobody is listening right now.
        let _ = self.events.send(event);
    }

    /// Resumes a stored session and connects the sync pairs.
    pub async fn startup(&self) {
        match self.account.resume().await {
            Ok(Some(user)) => {
                tracing::info!(%user, "session resumed");
                self.connect_all().await;
            }
            Ok(None) => tracing::info!("not logged in; run `protonsync login`"),
            Err(error) => {
                tracing::warn!(%error, "cannot resume the session");
                self.publish(Event::LoginRequired {
                    message: error.to_string(),
                });
            }
        }
    }

    async fn connect_all(&self) {
        for pair in &self.pairs {
            match self.connect(pair).await {
                Ok(connection) => *pair.connection.write().await = Some(Arc::new(connection)),
                Err(error) => {
                    tracing::warn!(pair = %pair.config.id, %error, "sync pair not connected");
                }
            }
        }
        self.connect_mounts().await;
    }

    async fn connect(
        &self,
        pair: &PairRuntime<A::Remote, L>,
    ) -> Result<Connection<A::Remote>, AccountError> {
        let remote = self.account.open_remote(&pair.config.remote).await?;
        let root = remote.root_folder().await?;
        let options = pair.options();
        Ok(Connection {
            remote: Arc::new(remote),
            pair: SyncPair {
                id: pair.id.clone(),
                local_root: pair.config.local.clone(),
                remote_root: root.id,
                direction: options.direction,
                storage_mode: StorageMode::FullLocal,
                protect_remote: options.protect_remote,
                archive_versions: options.archive_versions,
            },
            cache: Mutex::new(RemoteCache::new()),
        })
    }

    /// The pair's connection; connects again if the connection at login or
    /// startup failed (e.g. network down, remote folder could not be created).
    ///
    /// The error message names the real cause instead of "not logged in".
    async fn ensure_connection(
        &self,
        pair: &PairRuntime<A::Remote, L>,
    ) -> Result<Arc<Connection<A::Remote>>, String> {
        if let Some(connection) = pair.connection.read().await.clone() {
            return Ok(connection);
        }
        if self.account.username().await.is_none() {
            return Err(not_logged_in().message);
        }
        match self.connect(pair).await {
            Ok(connection) => {
                let connection = Arc::new(connection);
                *pair.connection.write().await = Some(Arc::clone(&connection));
                Ok(connection)
            }
            Err(error) => Err(format!(
                "cannot open the remote folder {:?}: {error}",
                pair.config.remote.to_string()
            )),
        }
    }

    async fn disconnect_all(&self) {
        for pair in &self.pairs {
            *pair.connection.write().await = None;
        }
        self.forget_job_remotes().await;
        self.forget_photo_library().await;
        self.disconnect_mounts().await;
    }

    async fn connection(&self, index: usize) -> Option<Arc<Connection<A::Remote>>> {
        let pair = self.pairs.get(index)?;
        pair.connection.read().await.clone()
    }

    fn find_pair(&self, id: &str) -> Result<usize, ApiError> {
        self.pairs
            .iter()
            .position(|pair| pair.config.id == id)
            .ok_or_else(|| unknown_pair(id))
    }

    /// Answers a request.
    pub async fn handle(&self, request: Request) -> Response {
        match self.dispatch(request).await {
            Ok(response) => response,
            Err(error) => Response::Error(error),
        }
    }

    async fn dispatch(&self, request: Request) -> Result<Response, ApiError> {
        match request {
            Request::Status => Ok(Response::Status(self.status().await?)),
            Request::LoginStart { username, password } => {
                let progress = self
                    .account
                    .login_start(&username, password.expose())
                    .await
                    .map_err(|e| convert::account_error(&e))?;
                Ok(Response::LoginStep(convert::login_step(progress)))
            }
            Request::LoginSecondFactor { code } => {
                let progress = self
                    .account
                    .login_second_factor(&code)
                    .await
                    .map_err(|e| convert::account_error(&e))?;
                Ok(Response::LoginStep(convert::login_step(progress)))
            }
            Request::LoginFinish { data_password } => {
                let username = self
                    .account
                    .login_finish(data_password.as_ref().map(protonsync_api::Password::expose))
                    .await
                    .map_err(|e| convert::account_error(&e))?;
                self.connect_all().await;
                for pair in &self.pairs {
                    pair.trigger.notify_one();
                }
                for job in &self.jobs {
                    job.trigger.notify_one();
                }
                if let Some(trigger) = self.photo_trigger() {
                    trigger.notify_one();
                }
                Ok(Response::LoggedIn { username })
            }
            Request::Logout => {
                self.disconnect_all().await;
                self.account
                    .logout()
                    .await
                    .map_err(|e| convert::account_error(&e))?;
                Ok(Response::Done)
            }
            Request::Sync { pair, apply } => {
                let results = self.sync_requested(pair.as_deref(), !apply).await?;
                Ok(Response::Sync { results })
            }
            Request::Conflicts => Ok(Response::Conflicts {
                conflicts: self.conflicts()?,
            }),
            Request::ResolveConflict {
                pair,
                id,
                resolution,
            } => {
                let index = self.find_pair(&pair)?;
                self.resolve_conflict(index, ConflictId(id), convert::resolution(resolution))?;
                Ok(Response::Done)
            }
            Request::Trash => Ok(Response::Trash {
                entries: self.trash()?,
            }),
            Request::RestoreTrash { pair, id } => {
                let owner = self.find_trash_owner(&pair)?;
                self.restore_trash(owner, TrashId(id)).await?;
                Ok(Response::Done)
            }
            Request::PurgeTrash { apply } => Ok(Response::Trash {
                entries: self.purge_trash(!apply).await?,
            }),
            // The server manages the subscription itself; only confirm here.
            Request::Subscribe => Ok(Response::Done),
            Request::Jobs => Ok(Response::Jobs {
                jobs: self.job_infos()?,
            }),
            Request::RunJob { job, apply } => {
                let index = self.find_job(&job)?;
                let (run, details) = self.run_job(index, !apply, 1).await?;
                Ok(Response::JobRun { run, details })
            }
            Request::JobHistory { job, limit } => Ok(Response::JobHistory {
                runs: self.job_history(&job, limit)?,
            }),
            request @ (Request::Mounts
            | Request::Pin { .. }
            | Request::Free { .. }
            | Request::PathStatus { .. }) => self.dispatch_files(request).await,
            Request::Settings => Ok(Response::Settings(self.settings_info())),
            Request::ChangeSettings { change } => {
                Ok(Response::Settings(self.change_settings(change).await?))
            }
            request @ (Request::Photos
            | Request::SyncPhotos { .. }
            | Request::PhotoDuplicates
            | Request::TrashPhotoDuplicates { .. }) => self.dispatch_photos(request).await,
        }
    }

    async fn status(&self) -> Result<StatusInfo, ApiError> {
        let mut pairs = Vec::with_capacity(self.pairs.len());
        for pair in &self.pairs {
            pairs.push(PairStatus {
                id: pair.config.id.clone(),
                local_root: pair.config.local.display().to_string(),
                remote_path: pair.config.remote.as_str().to_owned(),
                direction: convert::direction(pair.options().direction),
                syncing: pair.syncing.load(std::sync::atomic::Ordering::Relaxed),
                last_sync: pair.last_sync.lock().map(|g| g.clone()).unwrap_or_default(),
                open_conflicts: self
                    .state
                    .conflicts(&pair.id)
                    .map_err(|e| convert::state_api_error(&e))?
                    .len(),
                trash_entries: self
                    .state
                    .trash_entries(&pair.id)
                    .map_err(|e| convert::state_api_error(&e))?
                    .len(),
            });
        }
        Ok(StatusInfo {
            daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
            account: self.account.username().await,
            login_pending: self.account.login_pending().await,
            dry_run: self.settings().dry_run,
            pairs,
        })
    }

    async fn sync_requested(
        &self,
        filter: Option<&str>,
        dry_run: bool,
    ) -> Result<Vec<PairSyncResult>, ApiError> {
        if self.account.username().await.is_none() {
            return Err(not_logged_in());
        }
        let indices = match filter {
            Some(id) => vec![self.find_pair(id)?],
            None => (0..self.pairs.len()).collect(),
        };
        let mut results = Vec::with_capacity(indices.len());
        for index in indices {
            // A manual sync wants certainty: read the tree fully.
            results.push(self.sync_pair(index, Some(dry_run), TreeRead::Full).await);
        }
        Ok(results)
    }

    /// Automatic sync from the schedule with the cached remote tree.
    /// Without a login nothing happens.
    pub async fn scheduled_sync(&self, index: usize) -> Option<PairSyncResult> {
        self.account.username().await?;
        Some(self.sync_pair(index, None, TreeRead::Cached).await)
    }

    /// Like [`Self::scheduled_sync`], but reads the remote tree fully
    /// (safety net against missed events).
    pub async fn scheduled_full_sync(&self, index: usize) -> Option<PairSyncResult> {
        self.account.username().await?;
        Some(self.sync_pair(index, None, TreeRead::Full).await)
    }

    /// Asks Proton for events since the last sync; `true` if something has
    /// changed or there has never been a sync.
    pub async fn remote_changed(&self, index: usize) -> bool {
        let (Some(pair), Some(connection)) = (self.pairs.get(index), self.connection(index).await)
        else {
            return false;
        };
        let cursor = match self.state.event_cursor(&pair.id) {
            Ok(Some(cursor)) => cursor,
            Ok(None) => return true,
            Err(error) => {
                tracing::warn!(pair = %pair.config.id, %error, "cannot read the state");
                return false;
            }
        };
        match connection.remote.events_since(Some(&cursor)).await {
            Ok((events, _)) if !events.is_empty() => true,
            Ok((_, next)) => {
                if next != cursor
                    && let Err(error) = self.state.save_event_cursor(&pair.id, &next)
                {
                    tracing::warn!(pair = %pair.config.id, %error, "cursor not saved");
                }
                false
            }
            Err(RemoteError::Unauthorized) => {
                self.publish(Event::LoginRequired {
                    message: RemoteError::Unauthorized.to_string(),
                });
                false
            }
            Err(error) => {
                tracing::warn!(pair = %pair.config.id, %error, "cannot fetch events");
                false
            }
        }
    }

    /// Runs one sync of the pair. `dry_run = None` uses the setting that
    /// applies when the run starts (automatic runs).
    async fn sync_pair(
        &self,
        index: usize,
        dry_run: Option<bool>,
        read: TreeRead,
    ) -> PairSyncResult {
        let Some(pair) = self.pairs.get(index) else {
            return self.failed(String::new(), "unknown sync pair".into());
        };
        // Lock first: a settings change also holds this lock, so the run sees
        // either the old or the new settings, never a mix.
        let _guard = self.sync_lock.lock().await;
        let dry_run = dry_run.unwrap_or(self.settings().dry_run);
        let connection = match self.ensure_connection(pair).await {
            Ok(connection) => connection,
            Err(message) => return self.failed(pair.config.id.clone(), message),
        };
        pair.syncing
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.publish(Event::SyncStarted {
            pair: pair.config.id.clone(),
        });
        let started = self.clock.now();

        let outcome = self.run(pair, &connection, dry_run, read).await;
        let result = PairSyncResult {
            pair: pair.config.id.clone(),
            finished_at: self.clock.now().unix_seconds(),
            outcome: match &outcome {
                Ok(SyncOutcome::Planned(plan)) => convert::planned(plan),
                Ok(SyncOutcome::Applied(report)) => convert::applied(report),
                Err(error) => SyncOutcomeInfo::Failed {
                    message: convert::sync_error(error).message,
                },
            },
        };
        if matches!(outcome, Err(SyncError::Remote(RemoteError::Unauthorized))) {
            self.publish(Event::LoginRequired {
                message: RemoteError::Unauthorized.to_string(),
            });
        }
        self.publish_new_conflicts(pair, started.unix_seconds());

        if let Ok(mut last) = pair.last_sync.lock() {
            *last = Some(result.clone());
        }
        pair.syncing
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.publish(Event::SyncFinished(result.clone()));
        result
    }

    async fn run(
        &self,
        pair: &PairRuntime<A::Remote, L>,
        connection: &Connection<A::Remote>,
        dry_run: bool,
        read: TreeRead,
    ) -> Result<SyncOutcome, SyncError> {
        let context = SyncContext {
            remote: connection.remote.as_ref(),
            local: &pair.local,
            state: &self.state,
            clock: &self.clock,
            pair: &connection.pair,
        };
        let mut cache = connection.cache.lock().await;
        if read == TreeRead::Full {
            cache.invalidate();
        }
        let outcome = context.run(&mut cache, dry_run).await;
        // If the run discarded the cache as stale, sync fully once more right
        // away instead of waiting for the next trigger.
        if matches!(outcome, Ok(SyncOutcome::Applied(_))) && !cache.is_warm() {
            tracing::info!(pair = %pair.config.id, "cache stale, reading again");
            pair.trigger.notify_one();
        }
        outcome
    }

    fn publish_new_conflicts(&self, pair: &PairRuntime<A::Remote, L>, since: i64) {
        let Ok(conflicts) = self.state.conflicts(&pair.id) else {
            return;
        };
        for (id, conflict) in conflicts {
            if conflict.detected_at.unix_seconds() >= since {
                self.publish(Event::ConflictDetected(convert::conflict(
                    &pair.config.id,
                    id,
                    &conflict,
                )));
            }
        }
    }

    fn failed(&self, pair: String, message: String) -> PairSyncResult {
        PairSyncResult {
            pair,
            finished_at: self.clock.now().unix_seconds(),
            outcome: SyncOutcomeInfo::Failed { message },
        }
    }

    fn conflicts(&self) -> Result<Vec<protonsync_api::ConflictInfo>, ApiError> {
        let mut all = Vec::new();
        for pair in &self.pairs {
            let conflicts = self
                .state
                .conflicts(&pair.id)
                .map_err(|e| convert::state_api_error(&e))?;
            all.extend(
                conflicts
                    .iter()
                    .map(|(id, c)| convert::conflict(&pair.config.id, *id, c)),
            );
        }
        Ok(all)
    }

    fn resolve_conflict(
        &self,
        index: usize,
        id: ConflictId,
        resolution: protonsync_domain::conflict::Resolution,
    ) -> Result<(), ApiError> {
        let pair = self.pairs.get(index).ok_or_else(|| unknown_pair(""))?;
        let service = ConflictService {
            local: &pair.local,
            state: &self.state,
            clock: &self.clock,
            pair: &pair.id,
        };
        service
            .resolve(id, resolution)
            .map_err(|e| convert::use_case_error(&e))?;
        // The decision takes effect locally; the next sync transfers it.
        pair.trigger.notify_one();
        Ok(())
    }

    /// All owners of trash entries: sync pairs, upload jobs and drives.
    fn trash_owners(&self) -> impl Iterator<Item = (TrashOwner, String, &SyncPairId)> {
        let pairs = self
            .pairs
            .iter()
            .enumerate()
            .map(|(index, pair)| (TrashOwner::Pair(index), pair.config.id.clone(), &pair.id));
        let jobs = self.jobs.iter().enumerate().map(|(index, job)| {
            (
                TrashOwner::Job(index),
                job.owner.as_str().to_owned(),
                &job.owner,
            )
        });
        let mounts = self.mounts.iter().enumerate().map(|(index, mount)| {
            (
                TrashOwner::Mount(index),
                mount.owner.as_str().to_owned(),
                &mount.owner,
            )
        });
        let photos = self.photos.iter().map(|photos| {
            (
                TrashOwner::Photos,
                photos.owner.as_str().to_owned(),
                &photos.owner,
            )
        });
        pairs.chain(jobs).chain(mounts).chain(photos)
    }

    fn find_trash_owner(&self, name: &str) -> Result<TrashOwner, ApiError> {
        self.trash_owners()
            .find(|(_, owner_name, _)| owner_name == name)
            .map(|(owner, _, _)| owner)
            .ok_or_else(|| unknown_pair(name))
    }

    fn trash(&self) -> Result<Vec<protonsync_api::TrashInfo>, ApiError> {
        let mut all = Vec::new();
        for (_, name, id) in self.trash_owners() {
            let entries = self
                .state
                .trash_entries(id)
                .map_err(|e| convert::state_api_error(&e))?;
            all.extend(entries.iter().map(|(trash_id, entry)| {
                convert::trash(&name, *trash_id, entry, self.settings().retention)
            }));
        }
        Ok(all)
    }

    /// Proton access and local storage of a trash owner; `None` if not
    /// logged in.
    async fn trash_parts(&self, owner: TrashOwner) -> Option<(Arc<A::Remote>, &L, &SyncPairId)> {
        match owner {
            TrashOwner::Pair(index) => {
                let pair = self.pairs.get(index)?;
                let connection = self.connection(index).await?;
                Some((Arc::clone(&connection.remote), &pair.local, &pair.id))
            }
            TrashOwner::Job(index) => {
                let job = self.jobs.get(index)?;
                let remote = self.job_remote(index).await.ok()?;
                Some((remote, &job.source, &job.owner))
            }
            // Drives and photos have their own paths; see `mounts` and `photos`.
            TrashOwner::Mount(_) | TrashOwner::Photos => None,
        }
    }

    async fn restore_trash(&self, owner: TrashOwner, id: TrashId) -> Result<(), ApiError> {
        if let TrashOwner::Mount(index) = owner {
            let _guard = self.sync_lock.lock().await;
            return self.restore_mount_trash(index, id).await;
        }
        if let TrashOwner::Photos = owner {
            return self.restore_photo_trash(id).await;
        }
        let (remote, local, owner_id) = self.trash_parts(owner).await.ok_or_else(not_logged_in)?;
        let _guard = self.sync_lock.lock().await;
        TrashService {
            remote: remote.as_ref(),
            local,
            state: &self.state,
            clock: &self.clock,
            pair: owner_id,
        }
        .restore(id)
        .await
        .map_err(|e| convert::use_case_error(&e))?;
        if let TrashOwner::Pair(index) = owner
            && let Some(pair) = self.pairs.get(index)
        {
            pair.trigger.notify_one();
        }
        Ok(())
    }

    /// Permanently deletes expired trash entries of all connected pairs and
    /// jobs (with `dry_run` only lists them) and returns them.
    ///
    /// # Errors
    /// [`ApiError`] on the first error.
    pub async fn purge_trash(
        &self,
        dry_run: bool,
    ) -> Result<Vec<protonsync_api::TrashInfo>, ApiError> {
        let mut purged = Vec::new();
        let owners: Vec<(TrashOwner, String)> = self
            .trash_owners()
            .map(|(owner, name, _)| (owner, name))
            .collect();
        for (owner, name) in owners {
            if let TrashOwner::Mount(index) = owner {
                let _guard = self.sync_lock.lock().await;
                purged.extend(self.purge_mount_trash(index, dry_run).await?);
                continue;
            }
            if let TrashOwner::Photos = owner {
                purged.extend(self.purge_photo_trash(dry_run).await?);
                continue;
            }
            let Some((remote, local, owner_id)) = self.trash_parts(owner).await else {
                continue;
            };
            let _guard = self.sync_lock.lock().await;
            let entries = TrashService {
                remote: remote.as_ref(),
                local,
                state: &self.state,
                clock: &self.clock,
                pair: owner_id,
            }
            .purge_expired(self.settings().retention, dry_run)
            .await
            .map_err(|e| convert::use_case_error(&e))?;
            purged.extend(
                entries.iter().map(|(id, entry)| {
                    convert::trash(&name, *id, entry, self.settings().retention)
                }),
            );
        }
        Ok(purged)
    }

    /// Trash purge from the schedule; respects the dry-run setting.
    pub async fn scheduled_purge(&self) {
        match self.purge_trash(self.settings().dry_run).await {
            Ok(entries) if !entries.is_empty() => {
                tracing::info!(
                    count = entries.len(),
                    dry_run = self.settings().dry_run,
                    "trash purged"
                );
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(error = %error.message, "trash not purged"),
        }
    }
}
