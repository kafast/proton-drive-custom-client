//! Zwischengespeicherter entfernter Baum, der über Proton-Ereignisse aktuell bleibt.
//!
//! Ohne diesen Zwischenspeicher müsste jeder Lauf den ganzen Baum bei Proton
//! lesen (ein Aufruf je Ordner). Mit ihm genügt meist ein einziger Aufruf
//! (`events_since`); nur geänderte Knoten werden einzeln nachgeladen.
//!
//! Regeln, damit der Zwischenspeicher nie zu falschen Entscheidungen führt:
//! - Bei Zweifeln (Lücke im Ereignisstrom, sehr viele Ereignisse, unbekannte
//!   Lage) wird der Baum vollständig neu gelesen.
//! - Jedes Ereignis lädt den **aktuellen** Stand des Knotens. Die Verarbeitung
//!   ist deshalb wiederholbar: Ein Ereignis zweimal anzuwenden schadet nicht.
//! - Der Cursor wird erst nach erfolgreicher Verarbeitung aller Ereignisse
//!   übernommen. Bei Fehlern wird der Zwischenspeicher verworfen.

use std::collections::{BTreeMap, HashMap, HashSet};

use protonsync_domain::node::{Node, NodeId};
use protonsync_domain::path::RelativePath;

use crate::ports::{EventCursor, RemoteError, RemoteEvent, RemoteStorage};

/// Ab so vielen Ereignissen ist vollständiges Neulesen günstiger als einzelnes Nachladen.
const MAX_EVENTS_PER_CATCH_UP: usize = 1000;
/// Tiefer suchen wir nicht nach einem bekannten Elternordner; dann lieber neu lesen.
const MAX_ANCESTOR_DEPTH: usize = 64;
/// Obergrenze der gemerkten Knoten außerhalb des Sync-Paares (begrenzt den Speicher).
const MAX_OUTSIDE_MEMO: usize = 10_000;

/// Wie der Zwischenspeicher aktualisiert wurde.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refresh {
    /// Der Baum wurde vollständig bei Proton gelesen.
    Rebuilt,
    /// Nur die Ereignisse seit dem letzten Stand wurden angewendet.
    CaughtUp,
}

/// Eine Änderung, die die Sync-Engine selbst bei Proton vorgenommen hat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RemoteMutation {
    /// Knoten wurde unter `path` angelegt oder geändert.
    Stored { path: RelativePath, node: Node },
    /// Knoten unter `path` (samt Inhalt) liegt jetzt im Papierkorb.
    Trashed { path: RelativePath },
}

/// Zwischenspeicher des entfernten Baums eines Sync-Paares.
///
/// Ein leerer (kalter) Zwischenspeicher wird beim nächsten
/// [`RemoteCache::refresh`] vollständig gefüllt.
#[derive(Debug, Default)]
pub struct RemoteCache {
    index: Option<RemoteIndex>,
}

impl RemoteCache {
    /// Ein leerer Zwischenspeicher.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Verwirft den Inhalt; der nächste Lauf liest den Baum vollständig.
    pub fn invalidate(&mut self) {
        self.index = None;
    }

    /// `true`, wenn ein Stand vorhanden ist.
    #[must_use]
    pub fn is_warm(&self) -> bool {
        self.index.is_some()
    }

    /// Der zwischengespeicherte Baum (Pfade relativ zur Wurzel des Paares).
    #[must_use]
    pub fn tree(&self) -> Option<&BTreeMap<RelativePath, Node>> {
        self.index.as_ref().map(|index| &index.nodes)
    }

    /// Ereignis-Cursor, bis zu dem der Baum aktuell ist.
    #[must_use]
    pub fn cursor(&self) -> Option<&EventCursor> {
        self.index.as_ref().map(|index| &index.cursor)
    }

    /// Bringt den Baum unter `root` auf den aktuellen Stand.
    ///
    /// # Errors
    /// [`RemoteError`] von Proton; der Zwischenspeicher ist danach leer.
    pub async fn refresh<R: RemoteStorage>(
        &mut self,
        remote: &R,
        root: &NodeId,
    ) -> Result<Refresh, RemoteError> {
        // `take` nimmt den Inhalt heraus: Bei einem Fehler (`?`) bleibt der
        // Zwischenspeicher dadurch automatisch leer.
        if let Some(mut index) = self.index.take().filter(|index| &index.root == root)
            && index.catch_up(remote).await? == CatchUp::Done
        {
            self.index = Some(index);
            return Ok(Refresh::CaughtUp);
        }
        self.index = Some(RemoteIndex::build(remote, root).await?);
        Ok(Refresh::Rebuilt)
    }

    /// Übernimmt eine eigene Änderung, ohne auf ihr Ereignis zu warten.
    pub(crate) fn record(&mut self, mutation: RemoteMutation) {
        let Some(index) = &mut self.index else {
            return;
        };
        match mutation {
            RemoteMutation::Stored { path, node } => index.insert(path, node),
            RemoteMutation::Trashed { path } => index.remove_subtree(&path),
        }
    }
}

/// Ergebnis des Nachholens von Ereignissen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CatchUp {
    Done,
    NeedsRebuild,
}

/// Wo ein Knoten mit unbekanntem Elternordner liegt.
enum Ancestry {
    /// Innerhalb des Paares; `chain` sind die unbekannten Vorfahren, oberster zuerst.
    Inside { chain: Vec<Node> },
    /// Außerhalb des Paares; `chain` sind die geprüften Vorfahren.
    Outside { chain: Vec<Node> },
    /// Nicht eindeutig feststellbar.
    Unclear,
}

#[derive(Debug)]
struct RemoteIndex {
    root: NodeId,
    cursor: EventCursor,
    nodes: BTreeMap<RelativePath, Node>,
    paths: HashMap<NodeId, RelativePath>,
    /// Knoten, von denen bekannt ist, dass sie außerhalb des Paares liegen.
    outside: HashSet<NodeId>,
}

impl RemoteIndex {
    /// Liest den ganzen Baum. Der Cursor wird **vorher** geholt: Was sich
    /// währenddessen ändert, kommt beim nächsten Nachholen noch einmal an.
    async fn build<R: RemoteStorage>(remote: &R, root: &NodeId) -> Result<Self, RemoteError> {
        let (_, cursor) = remote.events_since(None).await?;
        let mut index = Self {
            root: root.clone(),
            cursor,
            nodes: BTreeMap::new(),
            paths: HashMap::new(),
            outside: HashSet::new(),
        };
        for (path, node) in list_tree(remote, RelativePath::root(), root).await? {
            index.insert(path, node);
        }
        Ok(index)
    }

    async fn catch_up<R: RemoteStorage>(&mut self, remote: &R) -> Result<CatchUp, RemoteError> {
        let (events, cursor) = remote.events_since(Some(&self.cursor)).await?;
        if events.len() > MAX_EVENTS_PER_CATCH_UP || events.contains(&RemoteEvent::Gap) {
            return Ok(CatchUp::NeedsRebuild);
        }
        for (id, latest) in latest_per_node(&events) {
            let outcome = match latest {
                Latest::Trashed if id == self.root => CatchUp::NeedsRebuild,
                Latest::Trashed => {
                    self.forget(&id);
                    CatchUp::Done
                }
                Latest::Changed => self.refresh_node(remote, &id).await?,
            };
            if outcome == CatchUp::NeedsRebuild {
                return Ok(CatchUp::NeedsRebuild);
            }
        }
        self.cursor = cursor;
        if self.outside.len() > MAX_OUTSIDE_MEMO {
            self.outside.clear();
        }
        Ok(CatchUp::Done)
    }

    /// Lädt den aktuellen Stand eines Knotens und ordnet ihn ein.
    async fn refresh_node<R: RemoteStorage>(
        &mut self,
        remote: &R,
        id: &NodeId,
    ) -> Result<CatchUp, RemoteError> {
        if *id == self.root {
            return Ok(CatchUp::Done);
        }
        let node = match remote.node(id).await {
            Ok(node) => node,
            Err(RemoteError::NotFound(_)) => {
                self.forget(id);
                return Ok(CatchUp::Done);
            }
            Err(error) => return Err(error),
        };
        if let Some(parent) = self.known_folder_path(node.parent.as_ref()) {
            self.place(remote, parent.join(&node.name), node).await?;
            return Ok(CatchUp::Done);
        }
        match self.ancestry(remote, &node).await? {
            Ancestry::Inside { chain } => {
                // Der oberste unbekannte Vorfahr hängt an einem bekannten Ordner.
                // Mit seinem Unterbaum kommt auch `node` selbst in den Index.
                let Some(top) = chain.into_iter().next() else {
                    return Ok(CatchUp::NeedsRebuild);
                };
                let Some(parent) = self.known_folder_path(top.parent.as_ref()) else {
                    return Ok(CatchUp::NeedsRebuild);
                };
                self.place(remote, parent.join(&top.name), top).await?;
                Ok(CatchUp::Done)
            }
            Ancestry::Outside { chain } => {
                // Hinausverschoben oder nie im Paar gewesen.
                self.forget(&node.id);
                self.outside.insert(node.id);
                self.outside
                    .extend(chain.into_iter().map(|ancestor| ancestor.id));
                Ok(CatchUp::Done)
            }
            Ancestry::Unclear => Ok(CatchUp::NeedsRebuild),
        }
    }

    /// Pfad eines bekannten Ordners (die Wurzel eingeschlossen).
    fn known_folder_path(&self, id: Option<&NodeId>) -> Option<RelativePath> {
        let id = id?;
        if *id == self.root {
            return Some(RelativePath::root());
        }
        let path = self.paths.get(id)?;
        self.nodes
            .get(path)
            .is_some_and(Node::is_folder)
            .then(|| path.clone())
    }

    /// Geht die Elternkette hoch, bis ein bekannter Ordner, die Wurzel des
    /// Paares oder eine Stelle außerhalb erreicht ist.
    async fn ancestry<R: RemoteStorage>(
        &self,
        remote: &R,
        node: &Node,
    ) -> Result<Ancestry, RemoteError> {
        let mut chain: Vec<Node> = Vec::new();
        let mut current = node.parent.clone();
        for _ in 0..MAX_ANCESTOR_DEPTH {
            let Some(id) = current.take() else {
                return Ok(Ancestry::Outside { chain });
            };
            if self.known_folder_path(Some(&id)).is_some() {
                chain.reverse();
                return Ok(Ancestry::Inside { chain });
            }
            if self.outside.contains(&id) {
                return Ok(Ancestry::Outside { chain });
            }
            let ancestor = match remote.node(&id).await {
                Ok(ancestor) => ancestor,
                // Ein Vorfahr im Papierkorb: Der Knoten gehört nicht mehr zum Paar.
                Err(RemoteError::NotFound(_)) => return Ok(Ancestry::Outside { chain }),
                Err(error) => return Err(error),
            };
            chain.push(ancestor);
            current = chain.last().and_then(|ancestor| ancestor.parent.clone());
        }
        Ok(Ancestry::Unclear)
    }

    /// Trägt `node` unter `path` ein. Ein neuer oder verschobener Ordner wird
    /// samt Inhalt gelesen, denn für seinen Inhalt kommen keine eigenen Ereignisse.
    async fn place<R: RemoteStorage>(
        &mut self,
        remote: &R,
        path: RelativePath,
        node: Node,
    ) -> Result<(), RemoteError> {
        let moved_or_new = self.paths.get(&node.id) != Some(&path);
        let folder = node.is_folder().then(|| node.id.clone());
        self.insert(path.clone(), node);
        if let Some(folder) = folder
            && moved_or_new
        {
            for (child_path, child) in list_tree(remote, path, &folder).await? {
                self.insert(child_path, child);
            }
        }
        Ok(())
    }

    /// Trägt einen Knoten ein und räumt dabei auf: Sein alter Platz (nach
    /// Umbenennen/Verschieben) und ein anderer Knoten am neuen Platz verschwinden.
    fn insert(&mut self, path: RelativePath, node: Node) {
        if let Some(old) = self.paths.get(&node.id).cloned()
            && old != path
        {
            self.remove_subtree(&old);
        }
        if self
            .nodes
            .get(&path)
            .is_some_and(|other| other.id != node.id)
        {
            self.remove_subtree(&path);
        }
        self.outside.remove(&node.id);
        self.paths.insert(node.id.clone(), path.clone());
        self.nodes.insert(path, node);
    }

    /// Entfernt einen Knoten samt Unterbaum, falls bekannt.
    fn forget(&mut self, id: &NodeId) {
        if let Some(path) = self.paths.get(id).cloned() {
            self.remove_subtree(&path);
        }
    }

    fn remove_subtree(&mut self, path: &RelativePath) {
        // Die Map ist nach dem Text des Pfades sortiert. Alle Texte, die mit
        // `path` beginnen, stehen deshalb zusammen; darunter sind die Nachfahren,
        // aber auch Geschwister wie „a b“ neben „a“, die der Filter aussortiert.
        let doomed: Vec<RelativePath> = self
            .nodes
            .range(path.clone()..)
            .take_while(|(candidate, _)| candidate.as_str().starts_with(path.as_str()))
            .filter(|(candidate, _)| candidate.starts_with(path))
            .map(|(candidate, _)| candidate.clone())
            .collect();
        for candidate in doomed {
            if let Some(node) = self.nodes.remove(&candidate) {
                self.paths.remove(&node.id);
            }
        }
    }
}

/// Letzter bekannter Zustand eines Knotens laut Ereignissen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Latest {
    /// Angelegt oder geändert: den aktuellen Stand nachladen.
    Changed,
    /// Zuletzt in den Papierkorb gelegt: kein Nachladen nötig.
    Trashed,
}

/// Fasst die Ereignisse je Knoten zusammen. Die Reihenfolge folgt dem
/// **ersten** Ereignis (so kommen Elternordner meist vor ihren Kindern), der
/// Zustand dem **letzten** (ein später wiederhergestellter Knoten ist nicht weg).
fn latest_per_node(events: &[RemoteEvent]) -> Vec<(NodeId, Latest)> {
    let mut order: Vec<NodeId> = Vec::new();
    let mut latest: HashMap<NodeId, Latest> = HashMap::new();
    for event in events {
        let (id, state) = match event {
            RemoteEvent::Created(id) | RemoteEvent::Updated(id) => (id, Latest::Changed),
            RemoteEvent::Trashed(id) => (id, Latest::Trashed),
            // Lücken werden vorher behandelt.
            RemoteEvent::Gap => continue,
        };
        if latest.insert(id.clone(), state).is_none() {
            order.push(id.clone());
        }
    }
    order
        .into_iter()
        .filter_map(|id| latest.get(&id).copied().map(|state| (id, state)))
        .collect()
}

/// Liest den ganzen Baum unterhalb von `folder`; die Pfade beginnen bei `base`.
/// Eltern stehen in der Liste immer vor ihren Kindern.
async fn list_tree<R: RemoteStorage>(
    remote: &R,
    base: RelativePath,
    folder: &NodeId,
) -> Result<Vec<(RelativePath, Node)>, RemoteError> {
    let mut tree = Vec::new();
    // Eine eigene Liste statt Rekursion: rekursive `async fn` sind in Rust umständlich.
    let mut pending = vec![(base, folder.clone())];
    while let Some((folder_path, folder_id)) = pending.pop() {
        for child in remote.list_children(&folder_id).await? {
            let child_path = folder_path.join(&child.name);
            if child.is_folder() {
                pending.push((child_path.clone(), child.id.clone()));
            }
            tree.push((child_path, child));
        }
    }
    Ok(tree)
}
