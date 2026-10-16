//! Planung: Für jeden Pfad bestimmen, was zu tun ist, ohne etwas zu ändern.
//!
//! Der Plan ist auch das Ergebnis eines Probelaufs (Dry-Run).

use std::collections::HashMap;

use protonsync_domain::conflict::{Change, ConflictKind, SyncAction, decide};
use protonsync_domain::node::{Fingerprint, Node, NodeId, NodeKind};
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncDirection;

use super::snapshot::Snapshot;
use crate::ports::{BaseEntry, LocalKind, LocalStorage};

/// Ein geplanter Schritt für einen Pfad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedStep {
    /// Betroffener Pfad.
    pub path: RelativePath,
    /// Was getan wird.
    pub action: SyncAction,
    /// Art des Konflikts, nur bei [`SyncAction::Conflict`].
    pub conflict: Option<ConflictKind>,
    pub(crate) local: Option<LocalKind>,
    pub(crate) remote: Option<Node>,
    /// Kennung in Proton laut Basis (zum Nachprüfen vor lokalem Löschen).
    pub(crate) base_remote: Option<NodeId>,
}

/// Ein Pfad, der nicht abgeglichen werden kann, mit Begründung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedPath {
    /// Betroffener Pfad.
    pub path: RelativePath,
    /// Warum er übersprungen wird.
    pub reason: String,
}

/// Der vollständige Plan eines Laufs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncPlan {
    /// Schritte in Ausführungsreihenfolge (Eltern vor Kindern).
    pub steps: Vec<PlannedStep>,
    /// Pfade, die übersprungen werden.
    pub skipped: Vec<SkippedPath>,
    /// Bekannte entfernte Kennungen je Pfad (für Elternordner beim Hochladen).
    pub(crate) remote_ids: HashMap<RelativePath, NodeId>,
}

impl SyncPlan {
    /// `true`, wenn nichts zu tun ist.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty() && self.skipped.is_empty()
    }
}

/// Erstellt den Plan aus einer Momentaufnahme.
pub(crate) fn make_plan<L: LocalStorage>(
    snapshot: &Snapshot,
    local_storage: &L,
    direction: SyncDirection,
) -> SyncPlan {
    let mut plan = SyncPlan {
        remote_ids: snapshot
            .remote
            .iter()
            .map(|(path, node)| (path.clone(), node.id.clone()))
            .collect(),
        ..SyncPlan::default()
    };
    // Unterbäume, die schon durch einen Schritt am Ordner erledigt sind.
    let mut covered: Vec<RelativePath> = Vec::new();

    for path in snapshot.all_paths() {
        if covered.iter().any(|done| path.starts_with(done)) {
            continue;
        }
        let local = snapshot.local.get(&path);
        let remote = snapshot.remote.get(&path);
        let base = snapshot.base.get(&path);

        if !kinds_agree(local, remote, base) {
            plan.skipped.push(SkippedPath {
                path: path.clone(),
                reason: "Datei und Ordner mit demselben Namen".into(),
            });
            covered.push(path);
            continue;
        }

        let local_change = local_change(base, local);
        let remote_change = remote_change(base, remote);
        let identical = local_change == Change::Modified
            && remote_change == Change::Modified
            && contents_identical(local_storage, &path, local, remote);
        let mut action = decide(local_change, remote_change, identical, direction);

        let is_folder = base.is_some_and(BaseEntry::is_folder);
        if is_folder {
            action = protect_folder_contents(snapshot, &path, action);
            if matches!(action, SyncAction::TrashLocal | SyncAction::TrashRemote) {
                covered.push(path.clone());
            }
        }

        if action != SyncAction::Nothing {
            plan.steps.push(PlannedStep {
                conflict: (action == SyncAction::Conflict)
                    .then(|| ConflictKind::from_changes(local_change, remote_change))
                    .flatten(),
                path,
                action,
                local: local.cloned(),
                remote: remote.cloned(),
                base_remote: base.map(|entry| entry.remote_id.clone()),
            });
        }
    }
    plan
}

/// Ein gelöschter Ordner wird nicht in den Papierkorb gelegt, wenn auf der
/// anderen Seite darin etwas geändert wurde. Stattdessen wird er neu angelegt,
/// und seine Kinder werden einzeln abgeglichen.
fn protect_folder_contents(
    snapshot: &Snapshot,
    folder: &RelativePath,
    action: SyncAction,
) -> SyncAction {
    match action {
        SyncAction::TrashRemote if remote_changed_below(snapshot, folder) => SyncAction::Download,
        SyncAction::TrashLocal if local_changed_below(snapshot, folder) => SyncAction::Upload,
        other => other,
    }
}

fn descendants<'a>(
    snapshot: &'a Snapshot,
    folder: &'a RelativePath,
) -> impl Iterator<Item = RelativePath> + 'a {
    snapshot
        .all_paths()
        .into_iter()
        .filter(move |path| path != folder && path.starts_with(folder))
}

fn remote_changed_below(snapshot: &Snapshot, folder: &RelativePath) -> bool {
    descendants(snapshot, folder).any(|path| {
        remote_change(snapshot.base.get(&path), snapshot.remote.get(&path)) == Change::Modified
    })
}

fn local_changed_below(snapshot: &Snapshot, folder: &RelativePath) -> bool {
    descendants(snapshot, folder).any(|path| {
        local_change(snapshot.base.get(&path), snapshot.local.get(&path)) == Change::Modified
    })
}

/// Alle vorhandenen Seiten müssen dieselbe Art (Datei/Ordner) haben.
fn kinds_agree(local: Option<&LocalKind>, remote: Option<&Node>, base: Option<&BaseEntry>) -> bool {
    let kinds = [
        local.map(|kind| matches!(kind, LocalKind::Folder)),
        remote.map(Node::is_folder),
        base.map(BaseEntry::is_folder),
    ];
    let mut present = kinds.into_iter().flatten();
    match present.next() {
        Some(first) => present.all(|other| other == first),
        None => true,
    }
}

/// Größe und Änderungszeit genügen lokal als Änderungsmerkmal.
fn same_local_state(known: &Fingerprint, current: &Fingerprint) -> bool {
    known.size == current.size && known.modified == current.modified
}

pub(crate) fn local_change(base: Option<&BaseEntry>, local: Option<&LocalKind>) -> Change {
    match (base, local) {
        (None, None) => Change::Unchanged,
        (Some(_), None) => Change::Deleted,
        (None, Some(_)) => Change::Modified,
        (Some(_), Some(LocalKind::Folder)) => Change::Unchanged,
        (Some(base), Some(LocalKind::File(current))) => match &base.local {
            Some(known) if same_local_state(known, current) => Change::Unchanged,
            _ => Change::Modified,
        },
    }
}

pub(crate) fn remote_change(base: Option<&BaseEntry>, remote: Option<&Node>) -> Change {
    match (base, remote) {
        (None, None) => Change::Unchanged,
        (Some(_), None) => Change::Deleted,
        (None, Some(_)) => Change::Modified,
        (Some(base), Some(node)) => {
            let same_revision = match &node.kind {
                NodeKind::Folder => true,
                NodeKind::File { revision, .. } => base.remote_revision.as_ref() == Some(revision),
            };
            if base.remote_id == node.id && same_revision {
                Change::Unchanged
            } else {
                Change::Modified
            }
        }
    }
}

/// Haben beide Seiten nach ihren Änderungen denselben Inhalt?
///
/// Ordner gelten als gleich. Bei Dateien muss die Größe stimmen und dann die
/// SHA-1-Prüfsumme (falls Proton eine kennt), sonst die Änderungszeit.
/// Im Zweifel `false`: Dann entsteht ein Konflikt, aber nie Datenverlust.
fn contents_identical<L: LocalStorage>(
    local_storage: &L,
    path: &RelativePath,
    local: Option<&LocalKind>,
    remote: Option<&Node>,
) -> bool {
    match (local, remote.map(|node| (node, node.fingerprint()))) {
        (Some(LocalKind::Folder), Some((node, None))) => node.is_folder(),
        (Some(LocalKind::File(local_fp)), Some((_, Some(remote_fp)))) => {
            if local_fp.size != remote_fp.size {
                return false;
            }
            match &remote_fp.content_hash {
                Some(remote_hash) => local_storage
                    .content_hash(path)
                    .is_ok_and(|local_hash| &local_hash == remote_hash),
                None => local_fp.modified == remote_fp.modified,
            }
        }
        _ => false,
    }
}
