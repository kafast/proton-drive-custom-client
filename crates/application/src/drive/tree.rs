//! Inode table of the virtual drive: which nodes are known, under
//! which number, and how they are connected.
//!
//! Purely in memory and without I/O. Folders are only filled on the first
//! listing (`children` is `None` until then).

use std::collections::{BTreeMap, HashMap};

use protonsync_domain::node::{Node, NodeId, NodeKind, NodeName, RevisionId};
use protonsync_domain::path::RelativePath;
use protonsync_domain::time::Timestamp;

/// Number of an entry in the drive (like an inode on Linux).
pub type Ino = u64;

/// With FUSE the root always has number 1.
pub const ROOT_INO: Ino = 1;

/// Folder or file with the details the drive shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    /// Folder; `children` only known after the first listing.
    Folder {
        /// Children by name, `None` as long as not read.
        children: Option<BTreeMap<NodeName, Ino>>,
    },
    /// File.
    File {
        /// Size in bytes (with local changes, the local size).
        size: u64,
        /// Modification time.
        modified: Timestamp,
        /// Revision in Proton; `None` for a file newly created locally.
        revision: Option<RevisionId>,
    },
}

/// An entry of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inode {
    /// Node in Proton; `None` for a file not uploaded yet.
    pub node: Option<NodeId>,
    /// Parent folder (for the root, the root itself).
    pub parent: Ino,
    /// Name in the parent folder (empty and meaningless for the root).
    pub name: NodeName,
    /// Kind and details.
    pub kind: EntryKind,
}

impl Inode {
    /// `true` for folders.
    #[must_use]
    pub fn is_folder(&self) -> bool {
        matches!(self.kind, EntryKind::Folder { .. })
    }
}

fn kind_of(node: &Node) -> EntryKind {
    match &node.kind {
        NodeKind::Folder => EntryKind::Folder { children: None },
        NodeKind::File {
            revision,
            fingerprint,
        } => EntryKind::File {
            size: fingerprint.size,
            modified: fingerprint.modified,
            revision: Some(revision.clone()),
        },
    }
}

/// The inode table.
#[derive(Debug)]
pub struct Tree {
    inodes: HashMap<Ino, Inode>,
    by_node: HashMap<NodeId, Ino>,
    next: Ino,
}

impl Tree {
    /// Table with the root folder of the drive.
    #[must_use]
    pub fn new(root: &Node) -> Self {
        let mut inodes = HashMap::new();
        inodes.insert(
            ROOT_INO,
            Inode {
                node: Some(root.id.clone()),
                parent: ROOT_INO,
                name: root.name.clone(),
                kind: EntryKind::Folder { children: None },
            },
        );
        let mut by_node = HashMap::new();
        by_node.insert(root.id.clone(), ROOT_INO);
        Self {
            inodes,
            by_node,
            next: ROOT_INO + 1,
        }
    }

    /// An entry.
    #[must_use]
    pub fn get(&self, ino: Ino) -> Option<&Inode> {
        self.inodes.get(&ino)
    }

    /// An entry for changing.
    pub fn get_mut(&mut self, ino: Ino) -> Option<&mut Inode> {
        self.inodes.get_mut(&ino)
    }

    /// Number for a Proton node, if known.
    #[must_use]
    pub fn ino_of(&self, node: &NodeId) -> Option<Ino> {
        self.by_node.get(node).copied()
    }

    /// Child `name` in `parent`, if the folder has been read and the child exists.
    #[must_use]
    pub fn child(&self, parent: Ino, name: &NodeName) -> Option<Ino> {
        match &self.inodes.get(&parent)?.kind {
            EntryKind::Folder {
                children: Some(children),
            } => children.get(name).copied(),
            _ => None,
        }
    }

    /// Children of a read folder, sorted by name; `None` if the
    /// folder has not been read yet (or is not a folder).
    #[must_use]
    pub fn children(&self, folder: Ino) -> Option<Vec<(NodeName, Ino)>> {
        match &self.inodes.get(&folder)?.kind {
            EntryKind::Folder {
                children: Some(children),
            } => Some(
                children
                    .iter()
                    .map(|(name, ino)| (name.clone(), *ino))
                    .collect(),
            ),
            _ => None,
        }
    }

    /// Path relative to the root of the drive.
    #[must_use]
    pub fn path(&self, ino: Ino) -> Option<RelativePath> {
        let mut names = Vec::new();
        let mut current = ino;
        while current != ROOT_INO {
            let inode = self.inodes.get(&current)?;
            names.push(inode.name.clone());
            current = inode.parent;
        }
        let mut path = RelativePath::root();
        for name in names.iter().rev() {
            path = path.join(name);
        }
        Some(path)
    }

    /// Takes over the children of a folder from Proton. Known nodes keep
    /// their number; ones that no longer exist are dropped, except locally created
    /// files that have not been uploaded yet.
    pub fn set_children(&mut self, folder: Ino, nodes: &[Node]) {
        let old: Vec<Ino> = self
            .children(folder)
            .map(|children| children.into_iter().map(|(_, ino)| ino).collect())
            .unwrap_or_default();
        if let Some(Inode {
            kind: EntryKind::Folder { children },
            ..
        }) = self.inodes.get_mut(&folder)
            && children.is_none()
        {
            *children = Some(BTreeMap::new());
        }
        let mut current = Vec::with_capacity(nodes.len());
        for node in nodes {
            current.push(self.upsert(folder, node));
        }
        for ino in old {
            let keep =
                current.contains(&ino) || self.inodes.get(&ino).is_some_and(|i| i.node.is_none());
            if !keep {
                self.remove(ino);
            }
        }
    }

    /// Inserts a node under `parent` or updates it (including renaming
    /// and moving). Returns the number. The children of a folder are kept.
    pub fn upsert(&mut self, parent: Ino, node: &Node) -> Ino {
        if let Some(ino) = self.by_node.get(&node.id).copied() {
            self.detach(ino);
            if let Some(inode) = self.inodes.get_mut(&ino) {
                inode.parent = parent;
                inode.name = node.name.clone();
                // A folder keeps its read children; otherwise the new details apply.
                let keep_listing = inode.is_folder() && node.is_folder();
                if !keep_listing {
                    inode.kind = kind_of(node);
                }
            }
            self.attach(parent, ino);
            return ino;
        }
        self.insert(
            parent,
            Inode {
                node: Some(node.id.clone()),
                parent,
                name: node.name.clone(),
                kind: kind_of(node),
            },
        )
    }

    /// Creates a new, empty local file.
    pub fn add_local_file(&mut self, parent: Ino, name: NodeName, now: Timestamp) -> Ino {
        self.insert(
            parent,
            Inode {
                node: None,
                parent,
                name,
                kind: EntryKind::File {
                    size: 0,
                    modified: now,
                    revision: None,
                },
            },
        )
    }

    /// Connects an entry with its new node after uploading.
    pub fn bind(&mut self, ino: Ino, node: &Node) {
        if let Some(inode) = self.inodes.get_mut(&ino) {
            if let Some(old) = inode.node.take() {
                self.by_node.remove(&old);
            }
            inode.node = Some(node.id.clone());
            inode.kind = kind_of(node);
            self.by_node.insert(node.id.clone(), ino);
        }
    }

    /// Re-parents an entry (rename/move).
    pub fn rename(&mut self, ino: Ino, new_parent: Ino, new_name: NodeName) {
        self.detach(ino);
        if let Some(inode) = self.inodes.get_mut(&ino) {
            inode.parent = new_parent;
            inode.name = new_name;
        }
        self.attach(new_parent, ino);
    }

    /// Removes an entry including everything below it.
    pub fn remove(&mut self, ino: Ino) {
        if ino == ROOT_INO {
            return;
        }
        self.detach(ino);
        let mut pending = vec![ino];
        while let Some(current) = pending.pop() {
            if let Some(inode) = self.inodes.remove(&current) {
                if let Some(node) = &inode.node {
                    self.by_node.remove(node);
                }
                if let EntryKind::Folder {
                    children: Some(children),
                } = inode.kind
                {
                    pending.extend(children.into_values());
                }
            }
        }
    }

    /// Forgets the contents of all read folders (after a gap in the
    /// event stream); they are read again on the next access.
    /// The entries themselves stay so that open files remain valid.
    pub fn forget_listings(&mut self) {
        for inode in self.inodes.values_mut() {
            if let EntryKind::Folder { children } = &mut inode.kind {
                *children = None;
            }
        }
    }

    /// `true` if `folder` has been read.
    #[must_use]
    pub fn is_listed(&self, folder: Ino) -> bool {
        matches!(
            self.inodes.get(&folder).map(|inode| &inode.kind),
            Some(EntryKind::Folder { children: Some(_) })
        )
    }

    fn insert(&mut self, parent: Ino, inode: Inode) -> Ino {
        let ino = self.next;
        self.next += 1;
        if let Some(node) = &inode.node {
            self.by_node.insert(node.clone(), ino);
        }
        self.inodes.insert(ino, inode);
        self.attach(parent, ino);
        ino
    }

    fn attach(&mut self, parent: Ino, ino: Ino) {
        let Some(name) = self.inodes.get(&ino).map(|inode| inode.name.clone()) else {
            return;
        };
        if let Some(Inode {
            kind: EntryKind::Folder {
                children: Some(children),
            },
            ..
        }) = self.inodes.get_mut(&parent)
        {
            children.insert(name, ino);
        }
    }

    fn detach(&mut self, ino: Ino) {
        let Some((parent, name)) = self
            .inodes
            .get(&ino)
            .map(|inode| (inode.parent, inode.name.clone()))
        else {
            return;
        };
        if let Some(Inode {
            kind: EntryKind::Folder {
                children: Some(children),
            },
            ..
        }) = self.inodes.get_mut(&parent)
            && children.get(&name) == Some(&ino)
        {
            children.remove(&name);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use protonsync_domain::node::Fingerprint;

    fn name(text: &str) -> NodeName {
        NodeName::new(text).unwrap()
    }

    fn folder(id: &str, label: &str) -> Node {
        Node {
            id: NodeId::new(id),
            parent: None,
            name: name(label),
            kind: NodeKind::Folder,
        }
    }

    fn file(id: &str, label: &str, revision: &str, size: u64) -> Node {
        Node {
            id: NodeId::new(id),
            parent: None,
            name: name(label),
            kind: NodeKind::File {
                revision: RevisionId::new(revision),
                fingerprint: Fingerprint {
                    size,
                    modified: Timestamp::from_unix_seconds(5),
                    content_hash: None,
                },
            },
        }
    }

    fn tree() -> Tree {
        Tree::new(&folder("root", "root"))
    }

    #[test]
    fn listing_keeps_numbers_and_drops_vanished_nodes() {
        let mut tree = tree();
        assert!(!tree.is_listed(ROOT_INO));
        tree.set_children(
            ROOT_INO,
            &[folder("d", "Docs"), file("f", "a.txt", "r1", 3)],
        );
        let docs = tree.child(ROOT_INO, &name("Docs")).unwrap();
        let a = tree.child(ROOT_INO, &name("a.txt")).unwrap();
        assert_eq!(tree.path(a).unwrap().as_str(), "a.txt");
        let local = tree.add_local_file(ROOT_INO, name("new.txt"), Timestamp::from_unix_seconds(1));
        tree.set_children(ROOT_INO, &[file("f", "a.txt", "r2", 4)]);
        assert_eq!(tree.child(ROOT_INO, &name("a.txt")), Some(a), "same number");
        assert_eq!(tree.child(ROOT_INO, &name("Docs")), None);
        assert!(tree.get(docs).is_none());
        assert_eq!(
            tree.child(ROOT_INO, &name("new.txt")),
            Some(local),
            "new local file stays"
        );
        assert!(matches!(
            tree.get(a).unwrap().kind,
            EntryKind::File { size: 4, .. }
        ));
    }

    #[test]
    fn upsert_moves_and_renames_and_keeps_folder_listing() {
        let mut tree = tree();
        tree.set_children(ROOT_INO, &[folder("d", "Docs"), folder("e", "Else")]);
        let docs = tree.child(ROOT_INO, &name("Docs")).unwrap();
        let other = tree.child(ROOT_INO, &name("Else")).unwrap();
        tree.set_children(docs, &[file("f", "a.txt", "r1", 1)]);
        tree.set_children(other, &[]);
        let a = tree.child(docs, &name("a.txt")).unwrap();
        assert_eq!(tree.upsert(other, &file("f", "b.txt", "r1", 1)), a);
        assert_eq!(tree.path(a).unwrap().as_str(), "Else/b.txt");
        assert_eq!(tree.children(docs).unwrap(), vec![]);
        // A folder keeps its read children when updated.
        tree.upsert(ROOT_INO, &folder("e", "Other"));
        assert!(tree.is_listed(other));
        assert_eq!(tree.path(a).unwrap().as_str(), "Other/b.txt");
    }

    #[test]
    fn bind_rename_and_remove() {
        let mut tree = tree();
        tree.set_children(ROOT_INO, &[folder("d", "Docs")]);
        let docs = tree.child(ROOT_INO, &name("Docs")).unwrap();
        tree.set_children(docs, &[]);
        let local = tree.add_local_file(docs, name("x"), Timestamp::from_unix_seconds(1));
        assert_eq!(tree.get(local).unwrap().node, None);
        tree.bind(local, &file("n9", "x", "r1", 7));
        assert_eq!(tree.ino_of(&NodeId::new("n9")), Some(local));
        tree.rename(local, ROOT_INO, name("y"));
        assert_eq!(tree.path(local).unwrap().as_str(), "y");
        tree.rename(local, docs, name("z"));
        tree.remove(docs);
        assert!(tree.get(local).is_none(), "removed with the folder");
        assert_eq!(tree.ino_of(&NodeId::new("n9")), None);
        tree.remove(ROOT_INO);
        assert!(tree.get(ROOT_INO).is_some(), "root stays");
    }

    #[test]
    fn forgetting_listings_keeps_entries() {
        let mut tree = tree();
        tree.set_children(ROOT_INO, &[file("f", "a.txt", "r1", 3)]);
        let a = tree.child(ROOT_INO, &name("a.txt")).unwrap();
        tree.forget_listings();
        assert!(!tree.is_listed(ROOT_INO));
        assert!(tree.get(a).is_some());
        assert_eq!(tree.children(ROOT_INO), None);
        tree.set_children(ROOT_INO, &[file("f", "a.txt", "r1", 3)]);
        assert_eq!(tree.child(ROOT_INO, &name("a.txt")), Some(a));
    }
}
