//! The change log: stores what each sync run changed and answers queries.
//!
//! Writing the log never fails a sync: an error is only logged, because the
//! changes themselves have already happened.

use protonsync_api::{ApiError, Response};
use protonsync_application::ports::{Account, ChangeLog, ChangeQuery, Clock, LocalStorage};
use protonsync_domain::change_log::{ChangeEntry, keep_since};
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::time::Timestamp;

use super::{Core, DaemonState};
use crate::convert;

/// Upper bound for one query, so a client cannot make the daemon read everything at once.
pub const MAX_CHANGES_PER_QUERY: u32 = 10_000;

impl<A, L, S, C> Core<A, L, S, C>
where
    A: Account,
    A::Remote: 'static,
    L: LocalStorage,
    S: DaemonState,
    C: Clock + 'static,
{
    /// Stores the changes of a run of `pair` and drops entries past the retention.
    pub(super) fn log_changes(&self, pair: &SyncPairId, changes: &[ChangeEntry]) {
        let days = self.change_log_days;
        if days == 0 || changes.is_empty() {
            return;
        }
        for change in changes {
            // Also in the daemon's log (journal), for `journalctl -u protonsync`.
            tracing::info!(
                pair = pair.as_str(),
                path = change.path.as_str(),
                side = ?change.side,
                kind = ?change.kind,
                "change"
            );
        }
        if let Err(error) = self.state.record_changes(pair, changes) {
            tracing::warn!(pair = pair.as_str(), %error, "change log not written");
        }
        match self.state.prune_changes(keep_since(self.clock.now(), days)) {
            Ok(0) => {}
            Ok(removed) => tracing::debug!(removed, "old changes removed from the log"),
            Err(error) => tracing::warn!(%error, "old changes not removed"),
        }
    }

    /// Answers [`protonsync_api::Request::Changes`]: the logged changes, newest first.
    pub(super) fn changes(
        &self,
        since: Option<i64>,
        pair: Option<&str>,
        limit: u32,
    ) -> Result<Response, ApiError> {
        if let Some(id) = pair {
            self.find_pair(id)?;
        }
        let query = ChangeQuery {
            since: Timestamp::from_unix_seconds(since.unwrap_or(i64::MIN)),
            pair: pair.map(SyncPairId::new),
            limit: usize::try_from(limit.min(MAX_CHANGES_PER_QUERY)).unwrap_or(usize::MAX),
        };
        let logged = self
            .state
            .changes(&query)
            .map_err(|error| convert::state_api_error(&error))?;
        Ok(Response::Changes {
            changes: logged.iter().map(convert::change).collect(),
            keep_days: self.change_log_days,
        })
    }

    /// Days the change log keeps its entries (`0`: no change log).
    #[must_use]
    pub const fn change_log_days(&self) -> u32 {
        self.change_log_days
    }
}
