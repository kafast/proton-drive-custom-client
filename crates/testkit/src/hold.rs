//! Holds that keep one transfer of a fake waiting, like a slow network.
//!
//! Lets tests change something else while an upload or download is in flight.

use std::sync::{Arc, Mutex};

use async_lock::{Mutex as AsyncMutex, MutexGuardArc};

/// Keeps one transfer waiting until it is dropped (see
/// [`crate::FakeRemote::hold_next_upload`] and similar methods).
#[must_use = "the transfer continues as soon as the hold is dropped"]
pub struct TransferHold {
    _guard: Option<MutexGuardArc<()>>,
}

/// A point where the next transfer of one kind waits until the test lets it go.
#[derive(Debug, Default)]
pub(crate) struct Gate {
    lock: Option<Arc<AsyncMutex<()>>>,
    /// A transfer is waiting at the gate.
    held: bool,
}

impl Gate {
    /// Closes the gate for the next transfer.
    pub(crate) fn close(&mut self) -> TransferHold {
        let lock = Arc::new(AsyncMutex::new(()));
        let guard = lock.try_lock_arc();
        *self = Self {
            lock: Some(lock),
            held: false,
        };
        TransferHold { _guard: guard }
    }

    /// `true` once a transfer is waiting at the gate.
    pub(crate) fn is_held(&self) -> bool {
        self.held
    }

    /// Takes the lock for the arriving transfer (only the first one after closing).
    fn arrive(&mut self) -> Option<Arc<AsyncMutex<()>>> {
        let lock = self.lock.take();
        self.held = lock.is_some();
        lock
    }
}

/// Waits at the gate that `gate` picks from the fake's state, if it is closed.
///
/// The state lock is released before waiting, so the test can keep using the fake.
pub(crate) async fn wait_at<S>(state: &Mutex<S>, gate: impl FnOnce(&mut S) -> &mut Gate) {
    let lock = state
        .lock()
        .ok()
        .and_then(|mut state| gate(&mut state).arrive());
    if let Some(lock) = lock {
        drop(lock.lock().await);
    }
}
