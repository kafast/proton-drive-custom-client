//! Snapshots of both sides and of the base at the start of a run.

use std::collections::{BTreeMap, BTreeSet};

use protonsync_domain::archive::is_archived;
use protonsync_domain::node::Node;
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncPairId;

use crate::ports::{BaseEntry, LocalError, LocalKind, LocalStorage, StateError, StateRepo};

/// State of local, remote and base, each by path.
#[derive(Debug, Default)]
pub(crate) struct Snapshot {
    pub local: BTreeMap<RelativePath, LocalKind>,
    pub remote: BTreeMap<RelativePath, Node>,
    pub base: BTreeMap<RelativePath, BaseEntry>,
}

impl Snapshot {
    /// All paths that occur on at least one side or in the base,
    /// sorted. Parents always come before their children.
    pub fn all_paths(&self) -> BTreeSet<RelativePath> {
        self.local
            .keys()
            .chain(self.remote.keys())
            .chain(self.base.keys())
            .cloned()
            .collect()
    }

    /// Removes the version archives (`_archiv` folders) from all three views,
    /// so that they are never synced: each side keeps its own archive.
    pub fn drop_archives(&mut self) {
        self.local.retain(|path, _| !is_archived(path));
        self.remote.retain(|path, _| !is_archived(path));
        self.base.retain(|path, _| !is_archived(path));
    }
}

/// Local tree as a map.
pub(crate) fn local_tree<L: LocalStorage>(
    local: &L,
) -> Result<BTreeMap<RelativePath, LocalKind>, LocalError> {
    Ok(local
        .scan()?
        .into_iter()
        .map(|entry| (entry.path, entry.kind))
        .collect())
}

/// Base as a map.
pub(crate) fn base_tree<S: StateRepo>(
    state: &S,
    pair: &SyncPairId,
) -> Result<BTreeMap<RelativePath, BaseEntry>, StateError> {
    Ok(state
        .base_entries(pair)?
        .into_iter()
        .map(|entry| (entry.path.clone(), entry))
        .collect())
}
