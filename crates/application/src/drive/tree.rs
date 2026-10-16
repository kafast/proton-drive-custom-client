//! Inode-Tabelle des virtuellen Laufwerks: welche Knoten bekannt sind, unter
//! welcher Nummer, und wie sie zusammenhängen.
//!
//! Rein im Speicher und ohne I/O. Ordner werden erst beim ersten Auflisten
//! gefüllt (`children` ist bis dahin `None`).

use std::collections::{BTreeMap, HashMap};

use protonsync_domain::node::{Node, NodeId, NodeKind, NodeName, RevisionId};
use protonsync_domain::path::RelativePath;
use protonsync_domain::time::Timestamp;

/// Nummer eines Eintrags im Laufwerk (wie ein Inode unter Linux).
pub type Ino = u64;

/// Die Wurzel hat bei FUSE immer die Nummer 1.
pub const ROOT_INO: Ino = 1;

/// Ordner oder Datei mit den Angaben, die das Laufwerk anzeigt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    /// Ordner; `children` erst nach dem ersten Auflisten bekannt.
    Folder {
        /// Kinder nach Name, `None` solange nicht gelesen.
        children: Option<BTreeMap<NodeName, Ino>>,
    },
    /// Datei.
    File {
        /// Größe in Bytes (bei lokalen Änderungen die lokale Größe).
        size: u64,
        /// Änderungszeit.
        modified: Timestamp,
        /// Revision in Proton; `None` bei einer lokal neu angelegten Datei.
        revision: Option<RevisionId>,
    },
}

/// Ein Eintrag der Tabelle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inode {
    /// Knoten in Proton; `None` bei einer noch nicht hochgeladenen Datei.
    pub node: Option<NodeId>,
    /// Elternordner (bei der Wurzel sie selbst).
    pub parent: Ino,
    /// Name im Elternordner (bei der Wurzel leer bedeutungslos).
    pub name: NodeName,
    /// Art und Angaben.
    pub kind: EntryKind,
}

impl Inode {
    /// `true` bei Ordnern.
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

/// Die Inode-Tabelle.
#[derive(Debug)]
pub struct Tree {
    inodes: HashMap<Ino, Inode>,
    by_node: HashMap<NodeId, Ino>,
    next: Ino,
}

impl Tree {
    /// Tabelle mit dem Wurzelordner des Laufwerks.
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

    /// Ein Eintrag.
    #[must_use]
    pub fn get(&self, ino: Ino) -> Option<&Inode> {
        self.inodes.get(&ino)
    }

    /// Ein Eintrag zum Ändern.
    pub fn get_mut(&mut self, ino: Ino) -> Option<&mut Inode> {
        self.inodes.get_mut(&ino)
    }

    /// Nummer zu einem Proton-Knoten, falls bekannt.
    #[must_use]
    pub fn ino_of(&self, node: &NodeId) -> Option<Ino> {
        self.by_node.get(node).copied()
    }

    /// Kind `name` in `parent`, falls der Ordner gelesen ist und es gibt.
    #[must_use]
    pub fn child(&self, parent: Ino, name: &NodeName) -> Option<Ino> {
        match &self.inodes.get(&parent)?.kind {
            EntryKind::Folder {
                children: Some(children),
            } => children.get(name).copied(),
            _ => None,
        }
    }

    /// Kinder eines gelesenen Ordners, sortiert nach Name; `None`, wenn der
    /// Ordner noch nicht gelesen ist (oder kein Ordner ist).
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

    /// Pfad relativ zur Wurzel des Laufwerks.
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

    /// Übernimmt die Kinder eines Ordners aus Proton. Bekannte Knoten behalten
    /// ihre Nummer; nicht mehr vorhandene fallen weg, außer lokal angelegten
    /// Dateien, die noch nicht hochgeladen sind.
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

    /// Fügt einen Knoten unter `parent` ein oder aktualisiert ihn (auch Umbenennen
    /// und Verschieben). Liefert die Nummer. Die Kinder eines Ordners bleiben erhalten.
    pub fn upsert(&mut self, parent: Ino, node: &Node) -> Ino {
        if let Some(ino) = self.by_node.get(&node.id).copied() {
            self.detach(ino);
            if let Some(inode) = self.inodes.get_mut(&ino) {
                inode.parent = parent;
                inode.name = node.name.clone();
                // Ein Ordner behält seine gelesenen Kinder; sonst gelten die neuen Angaben.
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

    /// Legt eine lokal neue, leere Datei an.
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

    /// Verbindet einen Eintrag nach dem Hochladen mit seinem neuen Knoten.
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

    /// Hängt einen Eintrag um (Umbenennen/Verschieben).
    pub fn rename(&mut self, ino: Ino, new_parent: Ino, new_name: NodeName) {
        self.detach(ino);
        if let Some(inode) = self.inodes.get_mut(&ino) {
            inode.parent = new_parent;
            inode.name = new_name;
        }
        self.attach(new_parent, ino);
    }

    /// Entfernt einen Eintrag samt allem darunter.
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

    /// Vergisst die Inhalte aller gelesenen Ordner (nach einer Lücke im
    /// Ereignisstrom); sie werden beim nächsten Zugriff neu gelesen.
    /// Die Einträge selbst bleiben, damit geöffnete Dateien gültig bleiben.
    pub fn forget_listings(&mut self) {
        for inode in self.inodes.values_mut() {
            if let EntryKind::Folder { children } = &mut inode.kind {
                *children = None;
            }
        }
    }

    /// `true`, wenn `folder` gelesen ist.
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
        let local = tree.add_local_file(ROOT_INO, name("neu.txt"), Timestamp::from_unix_seconds(1));
        tree.set_children(ROOT_INO, &[file("f", "a.txt", "r2", 4)]);
        assert_eq!(
            tree.child(ROOT_INO, &name("a.txt")),
            Some(a),
            "gleiche Nummer"
        );
        assert_eq!(tree.child(ROOT_INO, &name("Docs")), None);
        assert!(tree.get(docs).is_none());
        assert_eq!(
            tree.child(ROOT_INO, &name("neu.txt")),
            Some(local),
            "lokal neu bleibt"
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
        // Ein Ordner behält beim Aktualisieren seine gelesenen Kinder.
        tree.upsert(ROOT_INO, &folder("e", "Anders"));
        assert!(tree.is_listed(other));
        assert_eq!(tree.path(a).unwrap().as_str(), "Anders/b.txt");
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
        assert!(tree.get(local).is_none(), "mit dem Ordner entfernt");
        assert_eq!(tree.ino_of(&NodeId::new("n9")), None);
        tree.remove(ROOT_INO);
        assert!(tree.get(ROOT_INO).is_some(), "Wurzel bleibt");
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
