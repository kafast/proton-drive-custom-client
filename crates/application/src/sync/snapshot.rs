//! Momentaufnahmen beider Seiten und der Basis zu Beginn eines Laufs.

use std::collections::{BTreeMap, BTreeSet};

use protonsync_domain::node::Node;
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncPairId;

use crate::ports::{BaseEntry, LocalError, LocalKind, LocalStorage, StateError, StateRepo};

/// Stand von lokal, entfernt und Basis, jeweils nach Pfad.
#[derive(Debug, Default)]
pub(crate) struct Snapshot {
    pub local: BTreeMap<RelativePath, LocalKind>,
    pub remote: BTreeMap<RelativePath, Node>,
    pub base: BTreeMap<RelativePath, BaseEntry>,
}

impl Snapshot {
    /// Alle Pfade, die auf mindestens einer Seite oder in der Basis vorkommen,
    /// sortiert. Eltern stehen dabei immer vor ihren Kindern.
    pub fn all_paths(&self) -> BTreeSet<RelativePath> {
        self.local
            .keys()
            .chain(self.remote.keys())
            .chain(self.base.keys())
            .cloned()
            .collect()
    }
}

/// Lokaler Baum als Map.
pub(crate) fn local_tree<L: LocalStorage>(
    local: &L,
) -> Result<BTreeMap<RelativePath, LocalKind>, LocalError> {
    Ok(local
        .scan()?
        .into_iter()
        .map(|entry| (entry.path, entry.kind))
        .collect())
}

/// Basis als Map.
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
