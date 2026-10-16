use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, MutexGuard};

use protonsync_application::ports::{
    BaseEntry, ConflictId, EventCursor, JOB_HISTORY_LIMIT, JobLog, StateError, StateRepo, TrashId,
};
use protonsync_domain::conflict::Conflict;
use protonsync_domain::job::{JobId, JobRun};
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::trash::TrashEntry;

/// Zustandsspeicher im Arbeitsspeicher, nur für Tests.
#[derive(Debug, Default)]
pub struct InMemoryStateRepo {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    base: HashMap<SyncPairId, BTreeMap<RelativePath, BaseEntry>>,
    cursors: HashMap<SyncPairId, EventCursor>,
    conflicts: BTreeMap<ConflictId, (SyncPairId, Conflict)>,
    trash: BTreeMap<TrashId, (SyncPairId, TrashEntry)>,
    /// Läufe je Job, älteste zuerst.
    job_runs: HashMap<JobId, Vec<JobRun>>,
    next_id: i64,
}

impl State {
    fn next_id(&mut self) -> i64 {
        self.next_id += 1;
        self.next_id
    }
}

impl InMemoryStateRepo {
    /// Leerer Zustand.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> Result<MutexGuard<'_, State>, StateError> {
        self.state
            .lock()
            .map_err(|_| StateError::Unavailable("Mutex vergiftet".into()))
    }
}

impl StateRepo for InMemoryStateRepo {
    fn base_entries(&self, pair: &SyncPairId) -> Result<Vec<BaseEntry>, StateError> {
        Ok(self
            .state()?
            .base
            .get(pair)
            .map(|entries| entries.values().cloned().collect())
            .unwrap_or_default())
    }

    fn save_base(&self, pair: &SyncPairId, entry: &BaseEntry) -> Result<(), StateError> {
        self.state()?
            .base
            .entry(pair.clone())
            .or_default()
            .insert(entry.path.clone(), entry.clone());
        Ok(())
    }

    fn remove_base_tree(&self, pair: &SyncPairId, path: &RelativePath) -> Result<(), StateError> {
        if let Some(entries) = self.state()?.base.get_mut(pair) {
            entries.retain(|candidate, _| !candidate.starts_with(path));
        }
        Ok(())
    }

    fn event_cursor(&self, pair: &SyncPairId) -> Result<Option<EventCursor>, StateError> {
        Ok(self.state()?.cursors.get(pair).cloned())
    }

    fn save_event_cursor(&self, pair: &SyncPairId, cursor: &EventCursor) -> Result<(), StateError> {
        self.state()?.cursors.insert(pair.clone(), cursor.clone());
        Ok(())
    }

    fn add_conflict(
        &self,
        pair: &SyncPairId,
        conflict: &Conflict,
    ) -> Result<ConflictId, StateError> {
        let mut state = self.state()?;
        let id = ConflictId(state.next_id());
        state.conflicts.insert(id, (pair.clone(), conflict.clone()));
        Ok(id)
    }

    fn conflicts(&self, pair: &SyncPairId) -> Result<Vec<(ConflictId, Conflict)>, StateError> {
        Ok(self
            .state()?
            .conflicts
            .iter()
            .filter(|(_, (owner, _))| owner == pair)
            .map(|(id, (_, conflict))| (*id, conflict.clone()))
            .collect())
    }

    fn remove_conflict(&self, id: ConflictId) -> Result<(), StateError> {
        self.state()?.conflicts.remove(&id);
        Ok(())
    }

    fn add_trash(&self, pair: &SyncPairId, entry: &TrashEntry) -> Result<TrashId, StateError> {
        let mut state = self.state()?;
        let id = TrashId(state.next_id());
        state.trash.insert(id, (pair.clone(), entry.clone()));
        Ok(id)
    }

    fn trash_entries(&self, pair: &SyncPairId) -> Result<Vec<(TrashId, TrashEntry)>, StateError> {
        Ok(self
            .state()?
            .trash
            .iter()
            .filter(|(_, (owner, _))| owner == pair)
            .map(|(id, (_, entry))| (*id, entry.clone()))
            .collect())
    }

    fn remove_trash(&self, id: TrashId) -> Result<(), StateError> {
        self.state()?.trash.remove(&id);
        Ok(())
    }
}

impl JobLog for InMemoryStateRepo {
    fn record_job_run(&self, job: &JobId, run: &JobRun) -> Result<(), StateError> {
        let mut state = self.state()?;
        let runs = state.job_runs.entry(job.clone()).or_default();
        runs.push(run.clone());
        let surplus = runs.len().saturating_sub(JOB_HISTORY_LIMIT);
        runs.drain(..surplus);
        Ok(())
    }

    fn job_runs(&self, job: &JobId, limit: usize) -> Result<Vec<JobRun>, StateError> {
        Ok(self
            .state()?
            .job_runs
            .get(job)
            .map(|runs| runs.iter().rev().take(limit).cloned().collect())
            .unwrap_or_default())
    }
}
