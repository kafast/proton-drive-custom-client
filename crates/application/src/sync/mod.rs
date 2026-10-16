//! Sync-Engine: gleicht ein Sync-Paar einmal vollständig ab.
//!
//! Ablauf eines Laufs:
//! 1. Momentaufnahme von lokal, entfernt und Basis ([`snapshot`]).
//! 2. Plan je Pfad über [`protonsync_domain::conflict::decide`] ([`plan`]).
//! 3. Ausführung, sofern kein Probelauf ([`apply`]).
//! 4. Ereignis-Cursor speichern, damit der Daemon nur bei neuen Ereignissen erneut läuft.

mod apply;
mod plan;
mod snapshot;

use protonsync_domain::conflict::SyncAction;
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncPair;
use thiserror::Error;

pub use plan::{PlannedStep, SkippedPath, SyncPlan};

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
    /// Gleicht das Sync-Paar einmal ab. Mit `dry_run = true` wird nur geplant.
    ///
    /// # Errors
    /// [`SyncError`], wenn eine Seite gar nicht lesbar ist oder die Anmeldung fehlt.
    /// Fehler einzelner Dateien stehen stattdessen in [`SyncReport::failures`].
    pub async fn run_once(&self, dry_run: bool) -> Result<SyncOutcome, SyncError> {
        // Cursor vor dem Lesen merken: Änderungen während des Laufs lösen so
        // einen weiteren Lauf aus, statt verloren zu gehen.
        let (_, cursor) = self.remote.events_since(None).await?;
        let snapshot = snapshot::Snapshot {
            remote: snapshot::remote_tree(self.remote, &self.pair.remote_root).await?,
            local: snapshot::local_tree(self.local)?,
            base: snapshot::base_tree(self.state, &self.pair.id)?,
        };
        let plan = plan::make_plan(&snapshot, self.local, self.pair.direction);
        if dry_run {
            return Ok(SyncOutcome::Planned(plan));
        }
        let report = apply::apply_plan(self, plan).await?;
        self.state.save_event_cursor(&self.pair.id, &cursor)?;
        Ok(SyncOutcome::Applied(report))
    }
}
