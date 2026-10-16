//! Cached remote tree that stays up to date through Proton events.
//!
//! Without this cache every run would have to read the whole tree from Proton
//! (one call per folder). With it a single call is usually enough
//! (`events_since`); only changed nodes are fetched individually.
//!
//! Rules so that the cache never leads to wrong decisions:
//! - When in doubt (gap in the event stream, very many events, unknown
//!   location) the tree is read again completely.
//! - Every event loads the **current** state of the node. Processing
//!   is therefore repeatable: applying an event twice does no harm.
//! - The cursor is only taken over after all events have been processed
//!   successfully. On errors the cache is discarded.

use std::collections::{BTreeMap, HashMap, HashSet};

use protonsync_domain::node::{Node, NodeId};
use protonsync_domain::path::RelativePath;

use crate::ports::{EventCursor, RemoteError, RemoteEvent, RemoteStorage};

/// From this many events on, a full re-read is cheaper than fetching individually.
const MAX_EVENTS_PER_CATCH_UP: usize = 1000;
/// We do not search deeper than this for a known parent folder; rather re-read then.
const MAX_ANCESTOR_DEPTH: usize = 64;
/// Upper limit of remembered nodes outside the sync pair (limits the memory).
const MAX_OUTSIDE_MEMO: usize = 10_000;

/// How the cache was updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refresh {
    /// The tree was read completely from Proton.
    Rebuilt,
    /// Only the events since the last state were applied.
    CaughtUp,
}

/// A change that the sync engine itself made in Proton.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RemoteMutation {
    /// Node was created or changed under `path`.
    Stored { path: RelativePath, node: Node },
    /// Node under `path` (including its content) is now in the trash.
    Trashed { path: RelativePath },
}

/// Cache of the remote tree of a sync pair.
///
/// An empty (cold) cache is filled completely on the next
/// [`RemoteCache::refresh`].
#[derive(Debug, Default)]
pub struct RemoteCache {
    index: Option<RemoteIndex>,
}

impl RemoteCache {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Discards the content; the next run reads the tree completely.
    pub fn invalidate(&mut self) {
        self.index = None;
    }

    /// `true` if a state is present.
    #[must_use]
    pub fn is_warm(&self) -> bool {
        self.index.is_some()
    }

    /// The cached tree (paths relative to the root of the pair).
    #[must_use]
    pub fn tree(&self) -> Option<&BTreeMap<RelativePath, Node>> {
        self.index.as_ref().map(|index| &index.nodes)
    }

    /// Event cursor up to which the tree is current.
    #[must_use]
    pub fn cursor(&self) -> Option<&EventCursor> {
        self.index.as_ref().map(|index| &index.cursor)
    }

    /// Brings the tree under `root` up to date.
    ///
    /// # Errors
    /// [`RemoteError`] from Proton; the cache is empty afterwards.
    pub async fn refresh<R: RemoteStorage>(
        &mut self,
        remote: &R,
        root: &NodeId,
    ) -> Result<Refresh, RemoteError> {
        // `take` takes the content out: on an error (`?`) the cache
        // therefore stays empty automatically.
        if let Some(mut index) = self.index.take().filter(|index| &index.root == root)
            && index.catch_up(remote).await? == CatchUp::Done
        {
            self.index = Some(index);
            return Ok(Refresh::CaughtUp);
        }
        self.index = Some(RemoteIndex::build(remote, root).await?);
        Ok(Refresh::Rebuilt)
    }

    /// Takes over an own change without waiting for its event.
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

/// Result of catching up on events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CatchUp {
    Done,
    NeedsRebuild,
}

/// Where a node with an unknown parent folder is located.
enum Ancestry {
    /// Inside the pair; `chain` are the unknown ancestors, topmost first.
    Inside { chain: Vec<Node> },
    /// Outside the pair; `chain` are the checked ancestors.
    Outside { chain: Vec<Node> },
    /// Cannot be determined unambiguously.
    Unclear,
}

#[derive(Debug)]
struct RemoteIndex {
    root: NodeId,
    cursor: EventCursor,
    nodes: BTreeMap<RelativePath, Node>,
    paths: HashMap<NodeId, RelativePath>,
    /// Nodes known to lie outside the pair.
    outside: HashSet<NodeId>,
}

impl RemoteIndex {
    /// Reads the whole tree. The cursor is fetched **before**: whatever
    /// changes meanwhile arrives once more on the next catch-up.
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

    /// Loads the current state of a node and classifies it.
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
                // The topmost unknown ancestor hangs on a known folder.
                // Together with its subtree, `node` itself also enters the index.
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
                // Moved out or never in the pair.
                self.forget(&node.id);
                self.outside.insert(node.id);
                self.outside
                    .extend(chain.into_iter().map(|ancestor| ancestor.id));
                Ok(CatchUp::Done)
            }
            Ancestry::Unclear => Ok(CatchUp::NeedsRebuild),
        }
    }

    /// Path of a known folder (including the root).
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

    /// Walks up the parent chain until a known folder, the root of the
    /// pair or a place outside is reached.
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
                // An ancestor in the trash: the node no longer belongs to the pair.
                Err(RemoteError::NotFound(_)) => return Ok(Ancestry::Outside { chain }),
                Err(error) => return Err(error),
            };
            chain.push(ancestor);
            current = chain.last().and_then(|ancestor| ancestor.parent.clone());
        }
        Ok(Ancestry::Unclear)
    }

    /// Records `node` under `path`. A new or moved folder is read
    /// including its content, because no separate events come for its content.
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

    /// Records a node and cleans up: its old place (after
    /// rename/move) and another node at the new place disappear.
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

    /// Removes a node including its subtree, if known.
    fn forget(&mut self, id: &NodeId) {
        if let Some(path) = self.paths.get(id).cloned() {
            self.remove_subtree(&path);
        }
    }

    fn remove_subtree(&mut self, path: &RelativePath) {
        // The map is sorted by the text of the path. All texts that start with
        // `path` are therefore together; among them are the descendants,
        // but also siblings like "a b" next to "a", which the filter sorts out.
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

/// Last known state of a node according to the events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Latest {
    /// Created or changed: fetch the current state.
    Changed,
    /// Last moved to the trash: no fetch needed.
    Trashed,
}

/// Combines the events per node. The order follows the
/// **first** event (so parent folders usually come before their children), the
/// state follows the **last** one (a node restored later is not gone).
fn latest_per_node(events: &[RemoteEvent]) -> Vec<(NodeId, Latest)> {
    let mut order: Vec<NodeId> = Vec::new();
    let mut latest: HashMap<NodeId, Latest> = HashMap::new();
    for event in events {
        let (id, state) = match event {
            RemoteEvent::Created(id) | RemoteEvent::Updated(id) => (id, Latest::Changed),
            RemoteEvent::Trashed(id) => (id, Latest::Trashed),
            // Gaps are handled beforehand.
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

/// Reads the whole tree below `folder`; the paths start at `base`.
/// Parents always come before their children in the list.
async fn list_tree<R: RemoteStorage>(
    remote: &R,
    base: RelativePath,
    folder: &NodeId,
) -> Result<Vec<(RelativePath, Node)>, RemoteError> {
    let mut tree = Vec::new();
    // An explicit list instead of recursion: recursive `async fn` are awkward in Rust.
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
