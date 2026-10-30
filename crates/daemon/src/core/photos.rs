//! Photo sync and duplicates in the daemon core (ADR 0012).
//!
//! The library is opened on first use and forgotten on logout. The
//! [`PhotoIndex`] stays in memory between runs and is kept up to date via
//! events.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use protonsync_api::{
    ApiError, DuplicateGroupInfo, ErrorCode, Event, PhotoOutcomeInfo, PhotoRunInfo, PhotoStatus,
    Request, Response, TrashInfo, TrashedPhotoInfo,
};
use protonsync_application::photos::{
    DuplicateService, IndexRefresh, PhotoError, PhotoIndex, PhotoOutcome, PhotoSync, photos_owner,
};
use protonsync_application::ports::{Account, Clock, LocalStorage, PhotoBin, RemoteError, TrashId};
use protonsync_application::use_cases::TrashService;
use protonsync_domain::node::NodeId;
use protonsync_domain::sync_pair::SyncPairId;
use tokio::sync::{Mutex, Notify};

use super::{Core, DaemonState, not_logged_in};
use crate::convert;
use crate::photo_config::PhotoConfig;

/// The photo sync during wiring: configuration and photo folder.
pub struct PhotoSetup<L> {
    /// Configuration.
    pub config: PhotoConfig,
    /// Local storage with the photo folder as its root.
    pub local: L,
}

pub(super) struct PhotoRuntime<P, L> {
    pub(super) config: PhotoConfig,
    /// The base and trash of the photos are stored under this identifier.
    pub(super) owner: SyncPairId,
    pub(super) local: L,
    /// The library; opened on first use.
    library: Mutex<Option<Arc<P>>>,
    index: Mutex<PhotoIndex>,
    /// Number of photos after the last refresh (readable without locking the index).
    photo_count: std::sync::Mutex<Option<usize>>,
    syncing: AtomicBool,
    last_run: std::sync::Mutex<Option<PhotoRunInfo>>,
    pub(super) trigger: Notify,
}

impl<P, L> PhotoRuntime<P, L> {
    pub(super) fn new(setup: PhotoSetup<L>) -> Self {
        Self {
            config: setup.config,
            owner: photos_owner(),
            local: setup.local,
            library: Mutex::new(None),
            index: Mutex::new(PhotoIndex::new()),
            photo_count: std::sync::Mutex::new(None),
            syncing: AtomicBool::new(false),
            last_run: std::sync::Mutex::new(None),
            trigger: Notify::new(),
        }
    }

    fn remember_count(&self, index: &PhotoIndex) {
        if let Ok(mut count) = self.photo_count.lock() {
            *count = index.is_loaded().then(|| index.len());
        }
    }
}

fn photos_not_configured() -> ApiError {
    ApiError::new(
        ErrorCode::NotFound,
        "photos are not set up (section [photos] in the configuration)",
    )
}

impl<A, L, S, C> Core<A, L, S, C>
where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: DaemonState,
    C: Clock + 'static,
{
    /// Sets up the photo sync.
    #[must_use]
    pub fn with_photos(mut self, setup: PhotoSetup<L>) -> Self {
        self.photos = Some(PhotoRuntime::new(setup));
        self
    }

    /// Signal for changes in the photo folder (from the watcher); `None` without photos.
    #[must_use]
    pub fn photo_trigger(&self) -> Option<&Notify> {
        self.photos.as_ref().map(|photos| &photos.trigger)
    }

    fn photo_runtime(&self) -> Result<&PhotoRuntime<A::Photos, L>, ApiError> {
        self.photos.as_ref().ok_or_else(photos_not_configured)
    }

    /// Opens (once) the photo library.
    async fn photo_library(&self) -> Result<Arc<A::Photos>, ApiError> {
        let photos = self.photo_runtime()?;
        if self.account.username().await.is_none() {
            return Err(not_logged_in());
        }
        let mut library = photos.library.lock().await;
        if let Some(library) = library.as_ref() {
            return Ok(Arc::clone(library));
        }
        let opened = Arc::new(
            self.account
                .open_photos()
                .await
                .map_err(|e| convert::account_error(&e))?,
        );
        *library = Some(Arc::clone(&opened));
        Ok(opened)
    }

    /// Forgets the library and index (on logout).
    pub(super) async fn forget_photo_library(&self) {
        if let Some(photos) = &self.photos {
            *photos.library.lock().await = None;
            *photos.index.lock().await = PhotoIndex::new();
            photos.remember_count(&PhotoIndex::new());
        }
    }

    /// Answers the photo requests of the API.
    pub(super) async fn dispatch_photos(&self, request: Request) -> Result<Response, ApiError> {
        match request {
            Request::Photos => Ok(Response::Photos(self.photo_status()?)),
            // A manual sync wants certainty: read the library fully.
            Request::SyncPhotos { apply } => {
                Ok(Response::PhotoRun(self.sync_photos(!apply, true).await?))
            }
            Request::PhotoDuplicates => Ok(Response::PhotoDuplicates {
                groups: self.photo_duplicates().await?,
            }),
            Request::TrashPhotoDuplicates { ids, apply } => Ok(Response::PhotosTrashed {
                dry_run: !apply,
                photos: self.trash_photo_duplicates(&ids, !apply).await?,
            }),
            _ => Err(ApiError::new(
                ErrorCode::InvalidRequest,
                "not a photo request",
            )),
        }
    }

    fn photo_status(&self) -> Result<PhotoStatus, ApiError> {
        let photos = self.photo_runtime()?;
        Ok(PhotoStatus {
            folder: photos.config.folder.display().to_string(),
            direction: convert::direction(photos.config.direction),
            syncing: photos.syncing.load(Ordering::Relaxed),
            photos: photos.photo_count.lock().ok().and_then(|count| *count),
            last_run: photos.last_run.lock().ok().and_then(|run| run.clone()),
            trash_entries: self
                .state
                .trash_entries(&photos.owner)
                .map_err(|e| convert::state_api_error(&e))?
                .len(),
        })
    }

    /// Syncs the photos (with `dry_run` only plans); `full` first reads the
    /// library again in full (safety net).
    ///
    /// # Errors
    /// [`ApiError`] if photos are not set up or not logged in. Errors of the
    /// run itself are in the result.
    pub async fn sync_photos(&self, dry_run: bool, full: bool) -> Result<PhotoRunInfo, ApiError> {
        let photos = self.photo_runtime()?;
        let library = self.photo_library().await?;
        let _guard = self.sync_lock.lock().await;
        photos.syncing.store(true, Ordering::Relaxed);
        self.publish(Event::PhotoSyncStarted);

        let mut index = photos.index.lock().await;
        if full {
            index.invalidate();
        }
        let sync = PhotoSync {
            library: library.as_ref(),
            local: &photos.local,
            state: self.state.as_ref(),
            clock: self.clock.as_ref(),
            calendar: &self.zone,
            direction: photos.config.direction,
            protect_remote: photos.config.protect_remote,
        };
        let outcome = sync.run(&mut index, dry_run).await;
        photos.remember_count(&index);
        // If a step discarded the index as stale, read it again right away.
        if matches!(outcome, Ok(PhotoOutcome::Applied(_))) && !index.is_loaded() {
            photos.trigger.notify_one();
        }
        drop(index);
        self.report_unauthorized(&outcome);

        let info = PhotoRunInfo {
            finished_at: self.clock.now().unix_seconds(),
            outcome: convert::photo_outcome(&outcome),
        };
        if let Ok(mut last) = photos.last_run.lock() {
            *last = Some(info.clone());
        }
        photos.syncing.store(false, Ordering::Relaxed);
        self.publish(Event::PhotoSyncFinished(info.clone()));
        Ok(info)
    }

    fn report_unauthorized<T>(&self, outcome: &Result<T, PhotoError>) {
        if matches!(outcome, Err(PhotoError::Remote(RemoteError::Unauthorized))) {
            self.publish(Event::LoginRequired {
                message: RemoteError::Unauthorized.to_string(),
            });
        }
    }

    /// Sync from the schedule; respects the dry-run setting. `None` if not
    /// logged in or no photos are set up.
    pub async fn scheduled_photo_sync(&self, full: bool) -> Option<PhotoRunInfo> {
        let run = self.sync_photos(self.settings().dry_run, full).await.ok()?;
        if let PhotoOutcomeInfo::Failed { message } = &run.outcome {
            tracing::warn!(%message, "photo sync failed");
        }
        Some(run)
    }

    /// Catches up on the library's events; `true` if something has changed or
    /// it was read for the first time.
    pub async fn photos_changed(&self) -> bool {
        let (Ok(photos), Ok(library)) = (self.photo_runtime(), self.photo_library().await) else {
            return false;
        };
        let mut index = photos.index.lock().await;
        let refreshed = index.refresh(library.as_ref()).await;
        photos.remember_count(&index);
        match refreshed {
            Ok(IndexRefresh::Full | IndexRefresh::Events(_)) => true,
            Ok(IndexRefresh::Unchanged) => false,
            Err(error) => {
                self.report_unauthorized::<()>(&Err(PhotoError::Remote(error.clone())));
                tracing::warn!(%error, "cannot fetch photo events");
                false
            }
        }
    }

    pub(super) async fn photo_duplicates(&self) -> Result<Vec<DuplicateGroupInfo>, ApiError> {
        let photos = self.photo_runtime()?;
        let library = self.photo_library().await?;
        let mut index = photos.index.lock().await;
        let groups = self
            .duplicate_service(library.as_ref())
            .find(&mut index)
            .await;
        photos.remember_count(&index);
        self.report_unauthorized(&groups);
        let groups = groups.map_err(|e| convert::photo_error(&e))?;
        Ok(groups
            .iter()
            .filter_map(|group| convert::duplicate_group(group, &index))
            .collect())
    }

    pub(super) async fn trash_photo_duplicates(
        &self,
        ids: &[String],
        dry_run: bool,
    ) -> Result<Vec<TrashedPhotoInfo>, ApiError> {
        let photos = self.photo_runtime()?;
        let library = self.photo_library().await?;
        let ids: Vec<NodeId> = ids.iter().map(|id| NodeId::new(id.clone())).collect();
        let _guard = self.sync_lock.lock().await;
        let mut index = photos.index.lock().await;
        let trashed = self
            .duplicate_service(library.as_ref())
            .trash(&mut index, &ids, dry_run)
            .await;
        photos.remember_count(&index);
        self.report_unauthorized(&trashed);
        let trashed = trashed.map_err(|e| convert::photo_error(&e))?;
        if !dry_run {
            // The local copies follow into the trash on the next sync.
            photos.trigger.notify_one();
        }
        Ok(trashed.iter().map(convert::trashed_photo).collect())
    }

    fn duplicate_service<'a>(
        &'a self,
        library: &'a A::Photos,
    ) -> DuplicateService<'a, A::Photos, S, C> {
        DuplicateService {
            library,
            state: self.state.as_ref(),
            clock: self.clock.as_ref(),
            calendar: &self.zone,
        }
    }

    /// Restores a trash entry of the photos (under `sync_lock`).
    pub(super) async fn restore_photo_trash(&self, id: TrashId) -> Result<(), ApiError> {
        let photos = self.photo_runtime()?;
        let library = self.photo_library().await?;
        let _guard = self.sync_lock.lock().await;
        TrashService {
            remote: &PhotoBin(library.as_ref()),
            local: &photos.local,
            state: self.state.as_ref(),
            clock: self.clock.as_ref(),
            pair: &photos.owner,
        }
        .restore(id)
        .await
        .map_err(|e| convert::use_case_error(&e))?;
        // A restored photo surely arrives, even if an event is missing.
        photos.index.lock().await.invalidate();
        photos.trigger.notify_one();
        Ok(())
    }

    /// Deletes expired trash entries of the photos; nothing without a login.
    pub(super) async fn purge_photo_trash(
        &self,
        dry_run: bool,
    ) -> Result<Vec<TrashInfo>, ApiError> {
        let Some(photos) = &self.photos else {
            return Ok(Vec::new());
        };
        let Ok(library) = self.photo_library().await else {
            return Ok(Vec::new());
        };
        let _guard = self.sync_lock.lock().await;
        let entries = TrashService {
            remote: &PhotoBin(library.as_ref()),
            local: &photos.local,
            state: self.state.as_ref(),
            clock: self.clock.as_ref(),
            pair: &photos.owner,
        }
        .purge_expired(self.settings().retention, dry_run)
        .await
        .map_err(|e| convert::use_case_error(&e))?;
        Ok(entries
            .iter()
            .map(|(id, entry)| {
                convert::trash(photos.owner.as_str(), *id, entry, self.settings().retention)
            })
            .collect())
    }
}
