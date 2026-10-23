//! One upload at a time per file.
//!
//! Closing a file (FUSE `release`) reaches the drive asynchronously: the
//! program has long continued and may already write the file again or
//! rename a helper file over it while the first upload is still running.
//! Two overlapping uploads of the same file both start from the revision
//! known before the first one finished. The second one then either sees
//! "Proton has a different version" and creates a needless conflict copy,
//! or overtakes the first one so that the older content wins. (Issue #35.)
//!
//! Every upload and every rename therefore enters the gate of the affected
//! files first. The gate is an async mutex (`async-lock`, independent of the
//! runtime), so a waiting task does not block a thread. Uploads of
//! different files still run in parallel.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use async_lock::{Mutex as AsyncMutex, MutexGuardArc};

use super::{DriveError, Ino};
use crate::ports::CacheKey;

/// What a gate protects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum GateKey {
    /// An entry in the tree (stays the same across renames and when a
    /// helper file replaces it).
    Entry(Ino),
    /// A cached content that is not (yet) in the tree, e.g. after a restart.
    Content(CacheKey),
}

/// The gates of one drive. A gate exists only while someone holds or
/// waits for it, so the map stays small.
#[derive(Default)]
pub(super) struct UploadGates {
    gates: Mutex<GateMap>,
}

type GateMap = HashMap<GateKey, Arc<AsyncMutex<()>>>;

/// Held gate; dropping it lets the next waiting task in.
pub(super) struct GateGuard<'a> {
    gates: &'a UploadGates,
    key: GateKey,
    // `Option` so that `drop` can release the lock before tidying up the map.
    guard: Option<MutexGuardArc<()>>,
}

impl UploadGates {
    /// Waits until no one else holds the gate for `key`, then holds it.
    ///
    /// # Errors
    /// [`DriveError::Internal`] if the map lock is poisoned.
    pub(super) async fn enter(&self, key: GateKey) -> Result<GateGuard<'_>, DriveError> {
        let gate = Arc::clone(self.map()?.entry(key).or_default());
        // The map lock is released here; only the async lock is awaited.
        let guard = gate.lock_arc().await;
        Ok(GateGuard {
            gates: self,
            key,
            guard: Some(guard),
        })
    }

    /// Holds the gates of two entries. They are always entered in the
    /// same order (smaller number first), so two renames in opposite
    /// directions cannot wait for each other forever.
    ///
    /// # Errors
    /// [`DriveError::Internal`] if the map lock is poisoned.
    pub(super) async fn enter_entries(
        &self,
        first: Ino,
        second: Option<Ino>,
    ) -> Result<Vec<GateGuard<'_>>, DriveError> {
        let mut inos = vec![first];
        inos.extend(second.filter(|other| *other != first));
        inos.sort_unstable();
        let mut guards = Vec::with_capacity(inos.len());
        for ino in inos {
            guards.push(self.enter(GateKey::Entry(ino)).await?);
        }
        Ok(guards)
    }

    fn map(&self) -> Result<MutexGuard<'_, GateMap>, DriveError> {
        self.gates
            .lock()
            .map_err(|_| DriveError::Internal("lock poisoned"))
    }
}

impl Drop for GateGuard<'_> {
    fn drop(&mut self) {
        drop(self.guard.take());
        // Remove the gate once nobody holds or waits for it any more (the
        // map's own reference is then the only one).
        if let Ok(mut map) = self.gates.gates.lock()
            && map
                .get(&self.key)
                .is_some_and(|gate| Arc::strong_count(gate) == 1)
        {
            map.remove(&self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[tokio::test]
    async fn a_released_gate_is_removed() {
        let gates = UploadGates::default();
        let guard = gates.enter(GateKey::Entry(7)).await.unwrap();
        assert_eq!(gates.map().unwrap().len(), 1);
        drop(guard);
        assert!(gates.map().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_held_gate_blocks_the_same_key_only() {
        let gates = UploadGates::default();
        let _held = gates.enter(GateKey::Entry(7)).await.unwrap();
        let gate = Arc::clone(gates.map().unwrap().get(&GateKey::Entry(7)).unwrap());
        assert!(gate.try_lock().is_none(), "same entry has to wait");
        let other = gates.enter(GateKey::Content(CacheKey(7))).await;
        assert!(other.is_ok(), "other keys are independent");
    }
}
