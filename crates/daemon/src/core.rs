//! Der Geschäftsablauf des Daemons: beantwortet API-Anfragen und führt
//! Abgleiche aus. Generisch über die Ports, damit er mit Fakes testbar ist.

use std::sync::Arc;

use protonsync_api::{
    ApiError, ErrorCode, Event, PairStatus, PairSyncResult, Request, Response, StatusInfo,
    SyncOutcomeInfo,
};
use protonsync_application::ports::{
    Account, AccountError, Clock, ConflictId, LocalStorage, RemoteError, RemoteStorage, StateRepo,
    TrashId,
};
use protonsync_application::sync::{RemoteCache, SyncContext, SyncError, SyncOutcome};
use protonsync_application::use_cases::{ConflictService, TrashService};
use protonsync_domain::sync_pair::{StorageMode, SyncPair, SyncPairId};
use protonsync_domain::trash::RetentionPeriod;
use tokio::sync::{Mutex, Notify, RwLock, broadcast};

use crate::config::PairConfig;
use crate::convert;

/// Wie viele Ereignisse ein langsamer Abonnent verpassen darf, bevor er Lücken sieht.
const EVENT_BUFFER: usize = 64;

/// Einstellungen, die der Ablauf braucht.
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    /// Automatische Abgleiche und Papierkorb-Leerung nur planen.
    pub dry_run: bool,
    /// Aufbewahrung im Papierkorb.
    pub retention: RetentionPeriod,
}

/// Ein Sync-Paar beim Zusammenbau: Konfiguration und lokaler Speicher.
pub struct PairSetup<L> {
    /// Konfiguration.
    pub config: PairConfig,
    /// Lokaler Speicher des Paares.
    pub local: L,
}

/// Verbindung eines Sync-Paares zu Proton (nur wenn angemeldet).
struct Connection<R> {
    remote: R,
    pair: SyncPair,
    /// Zwischengespeicherter entfernter Baum; gehört zur Verbindung, damit
    /// eine neue Anmeldung immer mit frisch gelesenem Baum beginnt.
    cache: Mutex<RemoteCache>,
}

/// Wie ein Lauf den entfernten Baum liest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreeRead {
    /// Zwischenspeicher nutzen und nur Ereignisse nachholen.
    Cached,
    /// Vollständig bei Proton lesen (Sicherheitsnetz, manueller Abgleich).
    Full,
}

struct PairRuntime<R, L> {
    config: PairConfig,
    id: SyncPairId,
    local: L,
    connection: RwLock<Option<Arc<Connection<R>>>>,
    syncing: std::sync::atomic::AtomicBool,
    last_sync: std::sync::Mutex<Option<PairSyncResult>>,
    trigger: Notify,
}

/// Der Daemon-Kern.
pub struct Core<A: Account, L, S, C> {
    account: A,
    state: S,
    clock: C,
    settings: Settings,
    pairs: Vec<PairRuntime<A::Remote, L>>,
    events: broadcast::Sender<Event>,
    // Immer nur ein Abgleich zur Zeit: schont Proton und vermeidet
    // gleichzeitige Schreibzugriffe auf den Zustand.
    sync_lock: Mutex<()>,
}

fn not_logged_in() -> ApiError {
    ApiError::new(ErrorCode::NotLoggedIn, "Nicht angemeldet")
}

fn unknown_pair(id: &str) -> ApiError {
    ApiError::new(ErrorCode::NotFound, format!("Unbekanntes Sync-Paar {id}"))
}

impl<A, L, S, C> Core<A, L, S, C>
where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    /// Baut den Kern zusammen.
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
            state,
            clock,
            settings,
            pairs: pairs
                .into_iter()
                .map(|setup| PairRuntime {
                    id: SyncPairId::new(setup.config.id.clone()),
                    config: setup.config,
                    local: setup.local,
                    connection: RwLock::new(None),
                    syncing: std::sync::atomic::AtomicBool::new(false),
                    last_sync: std::sync::Mutex::new(None),
                    trigger: Notify::new(),
                })
                .collect(),
            events,
            sync_lock: Mutex::new(()),
        }
    }

    /// Anzahl der Sync-Paare (Indizes `0..pair_count()`).
    #[must_use]
    pub fn pair_count(&self) -> usize {
        self.pairs.len()
    }

    /// Signal, das einen baldigen Abgleich des Paares anstößt (z. B. bei lokalen Änderungen).
    #[must_use]
    pub fn trigger(&self, index: usize) -> Option<&Notify> {
        self.pairs.get(index).map(|pair| &pair.trigger)
    }

    /// Abonniert Ereignisse.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    fn publish(&self, event: Event) {
        // Fehler heißt nur: Gerade hört niemand zu.
        let _ = self.events.send(event);
    }

    /// Setzt eine gespeicherte Sitzung fort und verbindet die Sync-Paare.
    pub async fn startup(&self) {
        match self.account.resume().await {
            Ok(Some(user)) => {
                tracing::info!(%user, "Sitzung fortgesetzt");
                self.connect_all().await;
            }
            Ok(None) => tracing::info!("Nicht angemeldet; `protonsync login` ausführen"),
            Err(error) => {
                tracing::warn!(%error, "Sitzung nicht fortsetzbar");
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
                    tracing::warn!(pair = %pair.config.id, %error, "Sync-Paar nicht verbunden");
                }
            }
        }
    }

    async fn connect(
        &self,
        pair: &PairRuntime<A::Remote, L>,
    ) -> Result<Connection<A::Remote>, AccountError> {
        let remote = self.account.open_remote(&pair.config.remote).await?;
        let root = remote.root_folder().await?;
        Ok(Connection {
            remote,
            pair: SyncPair {
                id: pair.id.clone(),
                local_root: pair.config.local.clone(),
                remote_root: root.id,
                direction: pair.config.direction,
                storage_mode: StorageMode::FullLocal,
            },
            cache: Mutex::new(RemoteCache::new()),
        })
    }

    async fn disconnect_all(&self) {
        for pair in &self.pairs {
            *pair.connection.write().await = None;
        }
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

    /// Beantwortet eine Anfrage.
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
                let index = self.find_pair(&pair)?;
                self.restore_trash(index, TrashId(id)).await?;
                Ok(Response::Done)
            }
            Request::PurgeTrash { apply } => Ok(Response::Trash {
                entries: self.purge_trash(!apply).await?,
            }),
            // Das Abo selbst verwaltet der Server; hier nur bestätigen.
            Request::Subscribe => Ok(Response::Done),
        }
    }

    async fn status(&self) -> Result<StatusInfo, ApiError> {
        let mut pairs = Vec::with_capacity(self.pairs.len());
        for pair in &self.pairs {
            pairs.push(PairStatus {
                id: pair.config.id.clone(),
                local_root: pair.config.local.display().to_string(),
                remote_path: pair.config.remote.as_str().to_owned(),
                direction: convert::direction(pair.config.direction),
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
            dry_run: self.settings.dry_run,
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
            // Wer von Hand abgleicht, will Gewissheit: Baum vollständig lesen.
            results.push(self.sync_pair(index, dry_run, TreeRead::Full).await);
        }
        Ok(results)
    }

    /// Automatischer Abgleich aus dem Zeitplan mit zwischengespeichertem
    /// entferntem Baum. Ohne Anmeldung passiert nichts.
    pub async fn scheduled_sync(&self, index: usize) -> Option<PairSyncResult> {
        self.connection(index).await?;
        Some(
            self.sync_pair(index, self.settings.dry_run, TreeRead::Cached)
                .await,
        )
    }

    /// Wie [`Self::scheduled_sync`], liest den entfernten Baum aber vollständig
    /// (Sicherheitsnetz gegen verpasste Ereignisse).
    pub async fn scheduled_full_sync(&self, index: usize) -> Option<PairSyncResult> {
        self.connection(index).await?;
        Some(
            self.sync_pair(index, self.settings.dry_run, TreeRead::Full)
                .await,
        )
    }

    /// Fragt Proton nach Ereignissen seit dem letzten Abgleich; `true`, wenn
    /// sich etwas geändert hat oder noch nie abgeglichen wurde.
    pub async fn remote_changed(&self, index: usize) -> bool {
        let (Some(pair), Some(connection)) = (self.pairs.get(index), self.connection(index).await)
        else {
            return false;
        };
        let cursor = match self.state.event_cursor(&pair.id) {
            Ok(Some(cursor)) => cursor,
            Ok(None) => return true,
            Err(error) => {
                tracing::warn!(pair = %pair.config.id, %error, "Zustand nicht lesbar");
                return false;
            }
        };
        match connection.remote.events_since(Some(&cursor)).await {
            Ok((events, _)) if !events.is_empty() => true,
            Ok((_, next)) => {
                if next != cursor
                    && let Err(error) = self.state.save_event_cursor(&pair.id, &next)
                {
                    tracing::warn!(pair = %pair.config.id, %error, "Cursor nicht gespeichert");
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
                tracing::warn!(pair = %pair.config.id, %error, "Ereignisse nicht abrufbar");
                false
            }
        }
    }

    async fn sync_pair(&self, index: usize, dry_run: bool, read: TreeRead) -> PairSyncResult {
        let Some(pair) = self.pairs.get(index) else {
            return self.failed(String::new(), "Unbekanntes Sync-Paar".into());
        };
        let Some(connection) = self.connection(index).await else {
            return self.failed(pair.config.id.clone(), not_logged_in().message);
        };
        let _guard = self.sync_lock.lock().await;
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
            remote: &connection.remote,
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
        // Hat der Lauf den Zwischenspeicher als veraltet verworfen, gleich noch
        // einmal vollständig abgleichen, statt auf den nächsten Anlass zu warten.
        if matches!(outcome, Ok(SyncOutcome::Applied(_))) && !cache.is_warm() {
            tracing::info!(pair = %pair.config.id, "Zwischenspeicher veraltet, lese neu");
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
        // Die Entscheidung wirkt lokal; der nächste Abgleich überträgt sie.
        pair.trigger.notify_one();
        Ok(())
    }

    fn trash(&self) -> Result<Vec<protonsync_api::TrashInfo>, ApiError> {
        let mut all = Vec::new();
        for pair in &self.pairs {
            let entries = self
                .state
                .trash_entries(&pair.id)
                .map_err(|e| convert::state_api_error(&e))?;
            all.extend(entries.iter().map(|(id, entry)| {
                convert::trash(&pair.config.id, *id, entry, self.settings.retention)
            }));
        }
        Ok(all)
    }

    async fn restore_trash(&self, index: usize, id: TrashId) -> Result<(), ApiError> {
        let pair = self.pairs.get(index).ok_or_else(|| unknown_pair(""))?;
        let connection = self.connection(index).await.ok_or_else(not_logged_in)?;
        let _guard = self.sync_lock.lock().await;
        self.trash_service(pair, &connection)
            .restore(id)
            .await
            .map_err(|e| convert::use_case_error(&e))?;
        pair.trigger.notify_one();
        Ok(())
    }

    /// Löscht abgelaufene Papierkorbeinträge aller verbundenen Paare endgültig
    /// (mit `dry_run` nur auflisten) und liefert sie.
    ///
    /// # Errors
    /// [`ApiError`] beim ersten Fehler.
    pub async fn purge_trash(
        &self,
        dry_run: bool,
    ) -> Result<Vec<protonsync_api::TrashInfo>, ApiError> {
        let mut purged = Vec::new();
        for (index, pair) in self.pairs.iter().enumerate() {
            let Some(connection) = self.connection(index).await else {
                continue;
            };
            let _guard = self.sync_lock.lock().await;
            let entries = self
                .trash_service(pair, &connection)
                .purge_expired(self.settings.retention, dry_run)
                .await
                .map_err(|e| convert::use_case_error(&e))?;
            purged.extend(entries.iter().map(|(id, entry)| {
                convert::trash(&pair.config.id, *id, entry, self.settings.retention)
            }));
        }
        Ok(purged)
    }

    /// Papierkorb-Leerung aus dem Zeitplan; beachtet die Probelauf-Einstellung.
    pub async fn scheduled_purge(&self) {
        match self.purge_trash(self.settings.dry_run).await {
            Ok(entries) if !entries.is_empty() => {
                tracing::info!(
                    count = entries.len(),
                    dry_run = self.settings.dry_run,
                    "Papierkorb geleert"
                );
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(error = %error.message, "Papierkorb nicht geleert"),
        }
    }

    fn trash_service<'a>(
        &'a self,
        pair: &'a PairRuntime<A::Remote, L>,
        connection: &'a Connection<A::Remote>,
    ) -> TrashService<'a, A::Remote, L, S, C> {
        TrashService {
            remote: &connection.remote,
            local: &pair.local,
            state: &self.state,
            clock: &self.clock,
            pair: &pair.id,
        }
    }
}
