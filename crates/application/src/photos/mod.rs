//! Foto-Abgleich und Duplikate (Phase 5, ADR 0012).
//!
//! Ablauf eines Laufs:
//! 1. Bibliothek über Ereignisse aktualisieren ([`PhotoIndex`]).
//! 2. Fotoordner lesen und mit der Basis vergleichen ([`plan`]).
//! 3. Ausführen, sofern kein Probelauf ([`apply`]).
//!
//! Die Basis und der Papierkorb liegen im normalen Zustandsspeicher unter dem
//! Besitzer [`PHOTOS_OWNER`].

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

/// Besitzer von Basis und Papierkorbeinträgen des Foto-Abgleichs. Der
/// Doppelpunkt kommt in Kennungen aus der Konfiguration nicht vor.
pub const PHOTOS_OWNER: &str = "photos:library";

/// Kennung für den Zustandsspeicher.
#[must_use]
pub fn photos_owner() -> SyncPairId {
    SyncPairId::new(PHOTOS_OWNER)
}

/// Fehler, der einen ganzen Lauf verhindert.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PhotoError {
    /// Bibliothek nicht erreichbar oder nicht angemeldet.
    #[error(transparent)]
    Remote(#[from] RemoteError),
    /// Fotoordner nicht lesbar.
    #[error(transparent)]
    Local(#[from] LocalError),
    /// Zustandsspeicher nicht verfügbar.
    #[error(transparent)]
    State(#[from] StateError),
    /// Die Auswahl zum Löschen von Duplikaten ist nicht erlaubt.
    #[error(transparent)]
    Selection(#[from] DuplicateSelectionError),
}

/// Was ein Lauf geliefert hat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhotoOutcome {
    /// Probelauf: nur der Plan.
    Planned(PhotoPlan),
    /// Der Plan wurde ausgeführt.
    Applied(PhotoReport),
}

/// Alle Ports, die ein Foto-Abgleich braucht.
pub struct PhotoSync<'a, P, L, S, C> {
    /// Die Fotobibliothek.
    pub library: &'a P,
    /// Der lokale Fotoordner.
    pub local: &'a L,
    /// Gespeicherter Zustand.
    pub state: &'a S,
    /// Uhr (für den Papierkorb).
    pub clock: &'a C,
    /// Kalender für die Monatsordner.
    pub calendar: &'a dyn Calendar,
    /// Richtung des Abgleichs.
    pub direction: SyncDirection,
}

impl<P, L, S, C> PhotoSync<'_, P, L, S, C>
where
    P: PhotoLibrary,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    /// Aktualisiert `index` und gleicht einmal ab; mit `dry_run = true` wird nur geplant.
    ///
    /// # Errors
    /// [`PhotoError`], wenn Bibliothek, Ordner oder Zustand gar nicht lesbar sind.
    /// Fehler einzelner Fotos stehen in [`PhotoReport::failures`].
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

    /// Plant gegen den aktuellen Stand von `index`, ohne ihn zu aktualisieren.
    ///
    /// # Errors
    /// [`PhotoError::Local`] oder [`PhotoError::State`].
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
            calendar: self.calendar,
            hash: |path: &_| self.local.content_hash(path),
        }))
    }
}
