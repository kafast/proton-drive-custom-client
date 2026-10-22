//! Photo sync and duplicates (phase 5, ADR 0012).
//!
//! Steps of a run:
//! 1. Update the library via events ([`PhotoIndex`]).
//! 2. Read the photo folder and compare it with the base (`plan`).
//! 3. Execute, unless it is a dry run (`apply`).
//!
//! The base and the trash live in the normal state store under the
//! owner [`PHOTOS_OWNER`].

mod apply;
mod duplicates;
mod index;
mod plan;

pub use apply::{PhotoFailure, PhotoReport};
pub use duplicates::{DuplicateService, TrashedDuplicate};
pub use index::{IndexRefresh, PhotoIndex};
pub use plan::{PhotoPlan, PhotoStep, PhotoStepKind};

use std::collections::BTreeMap;

use protonsync_domain::photo::duplicates::DuplicateSelectionError;
use protonsync_domain::sync_pair::{SyncDirection, SyncPairId};
use thiserror::Error;

use crate::ports::{
    Calendar, Clock, LocalError, LocalKind, LocalStorage, PhotoLibrary, RemoteError, StateError,
    StateRepo,
};

/// Owner of the base and trash entries of the photo sync. The
/// colon does not occur in identifiers from the configuration.
pub const PHOTOS_OWNER: &str = "photos:library";

/// Identifier for the state store.
#[must_use]
pub fn photos_owner() -> SyncPairId {
    SyncPairId::new(PHOTOS_OWNER)
}

/// Error that prevents a whole run.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PhotoError {
    /// Library unreachable or not logged in.
    #[error(transparent)]
    Remote(#[from] RemoteError),
    /// Photo folder not readable.
    #[error(transparent)]
    Local(#[from] LocalError),
    /// State store unavailable.
    #[error(transparent)]
    State(#[from] StateError),
    /// The selection for deleting duplicates is not allowed.
    #[error(transparent)]
    Selection(#[from] DuplicateSelectionError),
}

/// What a run delivered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhotoOutcome {
    /// Dry run: only the plan.
    Planned(PhotoPlan),
    /// The plan was executed.
    Applied(PhotoReport),
}

/// All ports a photo sync needs.
pub struct PhotoSync<'a, P, L, S, C> {
    /// The photo library.
    pub library: &'a P,
    /// The local photo folder.
    pub local: &'a L,
    /// Stored state.
    pub state: &'a S,
    /// Clock (for the trash).
    pub clock: &'a C,
    /// Calendar for the monthly folders.
    pub calendar: &'a dyn Calendar,
    /// Direction of the sync.
    pub direction: SyncDirection,
    /// Local deletions are not passed on to Proton Photos.
    pub protect_remote: bool,
}

impl<P, L, S, C> PhotoSync<'_, P, L, S, C>
where
    P: PhotoLibrary,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    /// Updates `index` and syncs once; with `dry_run = true` it only plans.
    ///
    /// # Errors
    /// [`PhotoError`] if the library, folder or state cannot be read at all.
    /// Errors of individual photos are in [`PhotoReport::failures`].
    pub async fn run(
        &self,
        index: &mut PhotoIndex,
        dry_run: bool,
    ) -> Result<PhotoOutcome, PhotoError> {
        index.refresh(self.library).await?;
        let plan = self.plan(index)?;
        if dry_run {
            return Ok(PhotoOutcome::Planned(plan));
        }
        Ok(PhotoOutcome::Applied(apply::apply(self, index, plan).await))
    }

    /// Plans against the current state of `index`, without updating it.
    ///
    /// # Errors
    /// [`PhotoError::Local`] or [`PhotoError::State`].
    pub fn plan(&self, index: &PhotoIndex) -> Result<PhotoPlan, PhotoError> {
        let local: BTreeMap<_, _> = self
            .local
            .scan()?
            .into_iter()
            .filter_map(|entry| match entry.kind {
                LocalKind::File(fingerprint) => Some((entry.path, fingerprint)),
                LocalKind::Folder => None,
            })
            .collect();
        let base = self.state.base_entries(&photos_owner())?;
        Ok(plan::make_plan(plan::PlanInput {
            photos: index.photos().collect(),
            local,
            base,
            direction: self.direction,
            protect_remote: self.protect_remote,
            calendar: self.calendar,
            hash: |path: &_| self.local.content_hash(path),
        }))
    }
}
