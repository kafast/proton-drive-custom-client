//! Sync-Engine: gleicht ein Sync-Paar einmal vollständig ab.
//!
//! Ablauf eines Laufs:
//! 1. Entfernten Baum aktualisieren: über Ereignisse oder vollständig ([`remote_index`]).
//! 2. Momentaufnahme von lokal, entfernt und Basis ([`snapshot`]).
//! 3. Plan je Pfad über [`protonsync_domain::conflict::decide`] ([`plan`]).
//! 4. Ausführung, sofern kein Probelauf ([`apply`]).
//! 5. Ereignis-Cursor speichern, damit der Daemon nur bei neuen Ereignissen erneut läuft.

mod apply;
mod plan;
mod remote_index;
mod snapshot;

use protonsync_domain::conflict::SyncAction;
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncPair;
use thiserror::Error;

pub use plan::{PlannedStep, SkippedPath, SyncPlan};
pub use remote_index::{Refresh, RemoteCache};

use crate::ports::{
    Clock, LocalError, LocalStorage, RemoteError, RemoteStorage, StateError, StateRepo,
};

/// Fehler, der einen ganzen Lauf verhindert oder abbricht.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SyncError {
    /// Proton nicht erreichbar oder nicht angemeldet.
    #[error(transparent)]
    Remote(#[from] RemoteError),
    /// Lokaler Ordner nicht lesbar.
    #[error(transparent)]
    Local(#[from] LocalError),
    /// Zustandsspeicher nicht verfügbar.
    #[error(transparent)]
    State(#[from] StateError),
}

/// Ein fehlgeschlagener Schritt; er wird beim nächsten Lauf erneut versucht.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepFailure {
    /// Betroffener Pfad.
    pub path: RelativePath,
    /// Geplante Aktion.
    pub action: SyncAction,
    /// Fehlermeldung.
    pub message: String,
}

/// Ergebnis eines Laufs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Hochgeladene Dateien und angelegte entfernte Ordner.
    pub uploaded: usize,
    /// Heruntergeladene Dateien und angelegte lokale Ordner.
    pub downloaded: usize,
    /// In den Proton-Papierkorb gelegt.
    pub trashed_remote: usize,
    /// In den lokalen Papierkorb gelegt.
    pub trashed_local: usize,
    /// Neue Konflikte (beide Versionen wurden behalten).
    pub conflicts: usize,
    /// Wegen der Sync-Richtung nicht übertragene Änderungen.
    pub ignored: usize,
    /// Übersprungene Pfade.
    pub skipped: Vec<SkippedPath>,
    /// Fehlgeschlagene Schritte.
    pub failures: Vec<StepFailure>,
}

impl SyncReport {
    fn count(&mut self, action: SyncAction) {
        match action {
            SyncAction::Upload => self.uploaded += 1,
            SyncAction::Download => self.downloaded += 1,
            SyncAction::TrashRemote => self.trashed_remote += 1,
            SyncAction::TrashLocal => self.trashed_local += 1,
            SyncAction::Conflict => self.conflicts += 1,
            SyncAction::IgnoreLocalChange | SyncAction::IgnoreRemoteChange => self.ignored += 1,
            SyncAction::Nothing | SyncAction::RecordConverged => {}
        }
    }
}

/// Was ein Lauf geliefert hat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// Probelauf: nur der Plan, nichts wurde geändert.
    Planned(SyncPlan),
    /// Der Plan wurde ausgeführt.
    Applied(SyncReport),
}

/// Alle Ports, die ein Lauf braucht, für genau ein Sync-Paar.
///
/// Die Ports kommen als Referenzen (`&'a R`): Die Engine besitzt sie nicht,
/// sie leiht sie sich nur für die Dauer des Laufs (ähnlich wie Dependency
/// Injection per Konstruktor in Java).
pub struct SyncContext<'a, R, L, S, C> {
    /// Entfernter Speicher.
    pub remote: &'a R,
    /// Lokaler Ordner des Sync-Paares.
    pub local: &'a L,
    /// Gespeicherter Zustand.
    pub state: &'a S,
    /// Uhr.
    pub clock: &'a C,
    /// Das Sync-Paar.
    pub pair: &'a SyncPair,
}

impl<R, L, S, C> SyncContext<'_, R, L, S, C>
where
    R: RemoteStorage,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    /// Gleicht das Sync-Paar einmal ab und liest dafür den entfernten Baum
    /// vollständig. Mit `dry_run = true` wird nur geplant.
    ///
    /// # Errors
    /// [`SyncError`], wenn eine Seite gar nicht lesbar ist oder die Anmeldung fehlt.
    /// Fehler einzelner Dateien stehen stattdessen in [`SyncReport::failures`].
    pub async fn run_once(&self, dry_run: bool) -> Result<SyncOutcome, SyncError> {
        self.run(&mut RemoteCache::new(), dry_run).await
    }

    /// Wie [`Self::run_once`], aber mit zwischengespeichertem entferntem Baum:
    /// Ist `cache` gefüllt, werden nur die Proton-Ereignisse seit dem letzten
    /// Lauf nachgeholt statt alle Ordner zu lesen.
    ///
    /// # Errors
    /// Wie [`Self::run_once`]; bei einem Fehler ist `cache` danach leer.
    pub async fn run(
        &self,
        cache: &mut RemoteCache,
        dry_run: bool,
    ) -> Result<SyncOutcome, SyncError> {
        let refresh = cache.refresh(self.remote, &self.pair.remote_root).await?;
        let (Some(remote), Some(cursor)) = (cache.tree().cloned(), cache.cursor().cloned()) else {
            // `refresh` füllt den Zwischenspeicher immer. Ohne Baum lieber abbrechen,
            // als ihn für leer zu halten und alles lokal zu löschen.
            return Err(SyncError::Remote(RemoteError::Unavailable(
                "entfernter Baum fehlt".into(),
            )));
        };
        let snapshot = snapshot::Snapshot {
            remote,
            local: snapshot::local_tree(self.local)?,
            base: snapshot::base_tree(self.state, &self.pair.id)?,
        };
        let plan = plan::make_plan(&snapshot, self.local, self.pair.direction);
        if dry_run {
            self.state.save_event_cursor(&self.pair.id, &cursor)?;
            return Ok(SyncOutcome::Planned(plan));
        }
        let mut view = apply::RemoteView {
            cache,
            verify_deletions: refresh == Refresh::CaughtUp,
        };
        let report = apply::apply_plan(self, plan, &mut view).await?;
        self.state.save_event_cursor(&self.pair.id, &cursor)?;
        Ok(SyncOutcome::Applied(report))
    }
}
