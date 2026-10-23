use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, MutexGuard};

use async_lock::{Mutex as AsyncMutex, MutexGuardArc};

use protonsync_application::ports::{EventCursor, RemoteError, RemoteEvent, RemoteStorage};
use protonsync_domain::node::{Fingerprint, Node, NodeId, NodeKind, NodeName, RevisionId};
use protonsync_domain::time::Timestamp;

use crate::hash::sha1_hex;

/// ID of the root folder in the fake.
const ROOT_ID: &str = "root";

/// Deterministic in-memory stand-in for Proton Drive.
///
/// IDs are assigned sequentially (`n1`, `n2`, … and `r1`, `r2`, …) so that
/// tests are reproducible. [`FakeRemote::fail_next_call`] simulates network
/// errors.
#[derive(Debug)]
pub struct FakeRemote {
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    entries: BTreeMap<NodeId, Entry>,
    events: Vec<RemoteEvent>,
    next_number: u64,
    pending_failure: Option<RemoteError>,
    /// Changes without an event (simulates lost events).
    silent: bool,
    list_calls: u64,
    node_calls: u64,
    range_bytes: u64,
    /// Set by [`FakeRemote::hold_next_upload`]; the next upload waits for it.
    upload_hold: Option<Arc<AsyncMutex<()>>>,
    /// An upload is waiting at the hold.
    upload_held: bool,
}

/// Keeps one upload waiting until it is dropped (see
/// [`FakeRemote::hold_next_upload`]).
#[must_use = "the upload continues as soon as the hold is dropped"]
pub struct UploadHold {
    _guard: Option<MutexGuardArc<()>>,
}

#[derive(Debug, Clone)]
struct Entry {
    node: Node,
    content: Vec<u8>,
    trashed: bool,
}

impl Default for FakeRemote {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeRemote {
    /// Empty storage with only the root folder.
    #[must_use]
    pub fn new() -> Self {
        let root = Node {
            id: NodeId::new(ROOT_ID),
            parent: None,
            name: root_name(),
            kind: NodeKind::Folder,
        };
        let mut entries = BTreeMap::new();
        entries.insert(
            root.id.clone(),
            Entry {
                node: root,
                content: Vec::new(),
                trashed: false,
            },
        );
        Self {
            state: Mutex::new(State {
                entries,
                events: Vec::new(),
                next_number: 1,
                pending_failure: None,
                silent: false,
                list_calls: 0,
                node_calls: 0,
                range_bytes: 0,
                upload_hold: None,
                upload_held: false,
            }),
        }
    }

    /// The next call (whichever it is) fails with `error`.
    pub fn fail_next_call(&self, error: RemoteError) {
        if let Ok(mut state) = self.state.lock() {
            state.pending_failure = Some(error);
        }
    }

    /// Reports a gap in the event stream, as Proton does for a cursor that is too old.
    pub fn push_gap(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.events.push(RemoteEvent::Gap);
        }
    }

    /// While `true`, changes produce no events (lost events).
    pub fn silence_events(&self, silent: bool) {
        if let Ok(mut state) = self.state.lock() {
            state.silent = silent;
        }
    }

    /// The next upload (`upload_file` or `upload_revision`) waits until the
    /// returned hold is dropped, like a slow transfer. Lets tests run other
    /// operations while an upload is in flight.
    pub fn hold_next_upload(&self) -> UploadHold {
        let gate = Arc::new(AsyncMutex::new(()));
        let guard = gate.try_lock_arc();
        if let Ok(mut state) = self.state.lock() {
            state.upload_hold = Some(gate);
            state.upload_held = false;
        }
        UploadHold { _guard: guard }
    }

    /// `true` once an upload is waiting at the hold from [`Self::hold_next_upload`].
    #[must_use]
    pub fn upload_is_held(&self) -> bool {
        self.state.lock().is_ok_and(|state| state.upload_held)
    }

    /// Waits at the hold, if one is set (only the first upload after setting it).
    async fn wait_at_upload_hold(&self) {
        let gate = self.state.lock().ok().and_then(|mut state| {
            let gate = state.upload_hold.take();
            state.upload_held = gate.is_some();
            gate
        });
        if let Some(gate) = gate {
            drop(gate.lock().await);
        }
    }

    /// Calls to `list_children` so far (a measure of the load on Proton).
    #[must_use]
    pub fn list_calls(&self) -> u64 {
        self.state.lock().map_or(0, |state| state.list_calls)
    }

    /// Calls to `node` so far.
    #[must_use]
    pub fn node_calls(&self) -> u64 {
        self.state.lock().map_or(0, |state| state.node_calls)
    }

    /// Bytes loaded through `download_range` so far.
    #[must_use]
    pub fn range_bytes(&self) -> u64 {
        self.state.lock().map_or(0, |state| state.range_bytes)
    }

    /// Runs `operation` with exclusive access to the state.
    fn with_state<T>(
        &self,
        operation: impl FnOnce(&mut State) -> Result<T, RemoteError>,
    ) -> Result<T, RemoteError> {
        let mut state: MutexGuard<'_, State> = self
            .state
            .lock()
            .map_err(|_| RemoteError::Unavailable("mutex poisoned".into()))?;
        if let Some(error) = state.pending_failure.take() {
            return Err(error);
        }
        operation(&mut state)
    }
}

fn root_name() -> NodeName {
    // "root" always satisfies the naming rules; the fallback is only formally needed.
    NodeName::new("root").unwrap_or_else(|_| unreachable!("valid name"))
}

/// Reads the whole content and checks the announced size, as real storage
/// would do too.
fn read_exactly(mut content: impl Read, size: u64) -> Result<Vec<u8>, RemoteError> {
    let mut buffer = Vec::new();
    content
        .read_to_end(&mut buffer)
        .map_err(|error| RemoteError::Transfer(error.to_string()))?;
    if buffer.len() as u64 == size {
        Ok(buffer)
    } else {
        Err(RemoteError::Transfer(format!(
            "expected {size} bytes, read {}",
            buffer.len()
        )))
    }
}

fn fingerprint(content: &[u8], modified: Timestamp) -> Fingerprint {
    Fingerprint {
        size: content.len() as u64,
        modified,
        content_hash: Some(sha1_hex(content)),
    }
}

impl State {
    fn next_number(&mut self) -> u64 {
        let number = self.next_number;
        self.next_number += 1;
        number
    }

    /// A node that is not trashed.
    fn live(&self, id: &NodeId) -> Result<&Entry, RemoteError> {
        self.entries
            .get(id)
            .filter(|entry| !entry.trashed && !self.has_trashed_ancestor(entry))
            .ok_or_else(|| RemoteError::NotFound(id.clone()))
    }

    fn has_trashed_ancestor(&self, entry: &Entry) -> bool {
        let mut parent = entry.node.parent.as_ref();
        while let Some(ancestor) = parent.and_then(|id| self.entries.get(id)) {
            if ancestor.trashed {
                return true;
            }
            parent = ancestor.node.parent.as_ref();
        }
        false
    }

    fn live_folder(&self, id: &NodeId) -> Result<&Entry, RemoteError> {
        let entry = self.live(id)?;
        if entry.node.is_folder() {
            Ok(entry)
        } else {
            Err(RemoteError::NotAFolder(id.clone()))
        }
    }

    fn live_file(&self, id: &NodeId) -> Result<&Entry, RemoteError> {
        let entry = self.live(id)?;
        if entry.node.is_folder() {
            Err(RemoteError::NotAFile(id.clone()))
        } else {
            Ok(entry)
        }
    }

    fn children(&self, folder: &NodeId) -> impl Iterator<Item = &Entry> {
        self.entries
            .values()
            .filter(move |entry| !entry.trashed && entry.node.parent.as_ref() == Some(folder))
    }

    fn ensure_name_free(
        &self,
        folder: &NodeId,
        name: &NodeName,
        except: Option<&NodeId>,
    ) -> Result<(), RemoteError> {
        let taken = self
            .children(folder)
            .any(|entry| &entry.node.name == name && Some(&entry.node.id) != except);
        if taken {
            Err(RemoteError::AlreadyExists(name.clone()))
        } else {
            Ok(())
        }
    }

    fn record(&mut self, event: RemoteEvent) {
        if !self.silent {
            self.events.push(event);
        }
    }

    fn insert(&mut self, node: Node, content: Vec<u8>) -> Node {
        self.record(RemoteEvent::Created(node.id.clone()));
        self.entries.insert(
            node.id.clone(),
            Entry {
                node: node.clone(),
                content,
                trashed: false,
            },
        );
        node
    }

    /// Changes an existing entry and records an `Updated` event.
    fn update(
        &mut self,
        id: &NodeId,
        change: impl FnOnce(&mut Entry),
    ) -> Result<Node, RemoteError> {
        let entry = self
            .entries
            .get_mut(id)
            .ok_or_else(|| RemoteError::NotFound(id.clone()))?;
        change(entry);
        let node = entry.node.clone();
        self.record(RemoteEvent::Updated(id.clone()));
        Ok(node)
    }

    fn is_same_or_ancestor(&self, candidate: &NodeId, of: &NodeId) -> bool {
        let mut current = Some(of.clone());
        while let Some(id) = current {
            if &id == candidate {
                return true;
            }
            current = self
                .entries
                .get(&id)
                .and_then(|entry| entry.node.parent.clone());
        }
        false
    }
}

// The fake answers immediately and waits for nothing. `async fn` still keeps
// the code as readable as a real implementation.
#[allow(clippy::unused_async_trait_impl)]
impl RemoteStorage for FakeRemote {
    async fn root_folder(&self) -> Result<Node, RemoteError> {
        self.with_state(|state| Ok(state.live(&NodeId::new(ROOT_ID))?.node.clone()))
    }

    async fn node(&self, id: &NodeId) -> Result<Node, RemoteError> {
        self.with_state(|state| {
            state.node_calls += 1;
            Ok(state.live(id)?.node.clone())
        })
    }

    async fn list_children(&self, folder: &NodeId) -> Result<Vec<Node>, RemoteError> {
        self.with_state(|state| {
            state.list_calls += 1;
            state.live_folder(folder)?;
            let mut children: Vec<Node> = state
                .children(folder)
                .map(|entry| entry.node.clone())
                .collect();
            children.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(children)
        })
    }

    async fn create_folder(&self, parent: &NodeId, name: &NodeName) -> Result<Node, RemoteError> {
        self.with_state(|state| {
            state.live_folder(parent)?;
            state.ensure_name_free(parent, name, None)?;
            let id = NodeId::new(format!("n{}", state.next_number()));
            let node = Node {
                id,
                parent: Some(parent.clone()),
                name: name.clone(),
                kind: NodeKind::Folder,
            };
            Ok(state.insert(node, Vec::new()))
        })
    }

    async fn upload_file<R: Read + Send>(
        &self,
        parent: &NodeId,
        name: &NodeName,
        content: R,
        size: u64,
        modified: Timestamp,
    ) -> Result<Node, RemoteError> {
        self.wait_at_upload_hold().await;
        let content = read_exactly(content, size)?;
        self.with_state(|state| {
            state.live_folder(parent)?;
            state.ensure_name_free(parent, name, None)?;
            let id = NodeId::new(format!("n{}", state.next_number()));
            let revision = RevisionId::new(format!("r{}", state.next_number()));
            let node = Node {
                id,
                parent: Some(parent.clone()),
                name: name.clone(),
                kind: NodeKind::File {
                    revision,
                    fingerprint: fingerprint(&content, modified),
                },
            };
            Ok(state.insert(node, content))
        })
    }

    async fn upload_revision<R: Read + Send>(
        &self,
        file: &NodeId,
        content: R,
        size: u64,
        modified: Timestamp,
    ) -> Result<Node, RemoteError> {
        self.wait_at_upload_hold().await;
        let content = read_exactly(content, size)?;
        self.with_state(|state| {
            state.live_file(file)?;
            let revision = RevisionId::new(format!("r{}", state.next_number()));
            state.update(file, |entry| {
                entry.node.kind = NodeKind::File {
                    revision,
                    fingerprint: fingerprint(&content, modified),
                };
                entry.content = content;
            })
        })
    }

    async fn download_to<W: Write + Send>(
        &self,
        file: &NodeId,
        target: &mut W,
    ) -> Result<u64, RemoteError> {
        let content = self.with_state(|state| Ok(state.live_file(file)?.content.clone()))?;
        target
            .write_all(&content)
            .map_err(|error| RemoteError::Transfer(error.to_string()))?;
        Ok(content.len() as u64)
    }

    async fn download_range(
        &self,
        file: &NodeId,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, RemoteError> {
        self.with_state(|state| {
            let content = &state.live_file(file)?.content;
            let size = content.len() as u64;
            if offset > size {
                return Err(RemoteError::InvalidRange { offset, size });
            }
            let end = offset.saturating_add(length).min(size);
            // Both values are <= content.len() and therefore fit into usize.
            let (start, end) = (
                usize::try_from(offset).unwrap_or(usize::MAX),
                usize::try_from(end).unwrap_or(usize::MAX),
            );
            let data = content[start..end].to_vec();
            state.range_bytes += data.len() as u64;
            Ok(data)
        })
    }

    async fn rename(&self, id: &NodeId, new_name: &NodeName) -> Result<Node, RemoteError> {
        self.with_state(|state| {
            let parent = state
                .live(id)?
                .node
                .parent
                .clone()
                .ok_or_else(|| RemoteError::InvalidMove("root folder".into()))?;
            state.ensure_name_free(&parent, new_name, Some(id))?;
            state.update(id, |entry| entry.node.name = new_name.clone())
        })
    }

    async fn move_node(&self, id: &NodeId, new_parent: &NodeId) -> Result<Node, RemoteError> {
        self.with_state(|state| {
            let name = state.live(id)?.node.name.clone();
            state.live_folder(new_parent)?;
            if state.is_same_or_ancestor(id, new_parent) {
                return Err(RemoteError::InvalidMove(
                    "cannot move a folder into itself".into(),
                ));
            }
            state.ensure_name_free(new_parent, &name, Some(id))?;
            state.update(id, |entry| entry.node.parent = Some(new_parent.clone()))
        })
    }

    async fn trash(&self, id: &NodeId) -> Result<(), RemoteError> {
        self.with_state(|state| {
            if state.live(id)?.node.parent.is_none() {
                return Err(RemoteError::InvalidMove("root folder".into()));
            }
            if let Some(entry) = state.entries.get_mut(id) {
                entry.trashed = true;
            }
            state.record(RemoteEvent::Trashed(id.clone()));
            Ok(())
        })
    }

    async fn restore(&self, id: &NodeId) -> Result<Node, RemoteError> {
        self.with_state(|state| {
            let entry = state
                .entries
                .get(id)
                .filter(|entry| entry.trashed)
                .ok_or_else(|| RemoteError::NotFound(id.clone()))?;
            let (name, parent) = (entry.node.name.clone(), entry.node.parent.clone());
            if let Some(parent) = parent {
                state.live_folder(&parent)?;
                state.ensure_name_free(&parent, &name, Some(id))?;
            }
            state.update(id, |entry| entry.trashed = false)
        })
    }

    async fn delete_permanently(&self, id: &NodeId) -> Result<(), RemoteError> {
        self.with_state(|state| {
            let ids: Vec<NodeId> = match state.entries.get(id) {
                Some(entry) if entry.trashed => state
                    .entries
                    .keys()
                    .filter(|candidate| state.is_same_or_ancestor(id, candidate))
                    .cloned()
                    .collect(),
                _ => return Err(RemoteError::NotFound(id.clone())),
            };
            for removed in ids {
                state.entries.remove(&removed);
            }
            Ok(())
        })
    }

    async fn events_since(
        &self,
        cursor: Option<&EventCursor>,
    ) -> Result<(Vec<RemoteEvent>, EventCursor), RemoteError> {
        self.with_state(|state| {
            let latest = EventCursor::new(state.events.len().to_string());
            let Some(cursor) = cursor else {
                return Ok((Vec::new(), latest));
            };
            let position: usize = cursor
                .as_str()
                .parse()
                .map_err(|_| RemoteError::Unavailable("invalid cursor".into()))?;
            let events = state.events.get(position..).unwrap_or_default().to_vec();
            Ok((events, latest))
        })
    }
}
