//! Der Zwischenspeicher des entfernten Baums muss nach jedem Nachholen von
//! Ereignissen genau dem entsprechen, was ein vollständiges Lesen ergibt.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use proptest::prelude::*;
use protonsync_application::ports::RemoteStorage;
use protonsync_application::sync::{Refresh, RemoteCache};
use protonsync_domain::node::{Node, NodeId, NodeName};
use protonsync_domain::path::RelativePath;
use protonsync_domain::time::Timestamp;
use protonsync_testkit::FakeRemote;

const T0: Timestamp = Timestamp::from_unix_seconds(1_760_000_000);

fn name(value: &str) -> NodeName {
    NodeName::new(value).unwrap()
}

/// Proton-Baum mit dem Ordner des Sync-Paares und einem Ordner daneben.
struct Setup {
    remote: FakeRemote,
    pair_root: Node,
    outside: Node,
}

impl Setup {
    async fn new() -> Self {
        let remote = FakeRemote::new();
        let top = remote.root_folder().await.unwrap();
        let pair_root = remote.create_folder(&top.id, &name("paar")).await.unwrap();
        let outside = remote.create_folder(&top.id, &name("außen")).await.unwrap();
        Self {
            remote,
            pair_root,
            outside,
        }
    }

    async fn file(&self, parent: &NodeId, file: &str) -> Node {
        self.remote
            .upload_file(parent, &name(file), &b"x"[..], 1, T0)
            .await
            .unwrap()
    }

    async fn folder(&self, parent: &NodeId, folder: &str) -> Node {
        self.remote
            .create_folder(parent, &name(folder))
            .await
            .unwrap()
    }

    /// Was ein vollständiges Lesen jetzt ergäbe.
    async fn full_tree(&self) -> BTreeMap<RelativePath, Node> {
        let mut fresh = RemoteCache::new();
        fresh
            .refresh(&self.remote, &self.pair_root.id)
            .await
            .unwrap();
        fresh.tree().unwrap().clone()
    }

    async fn warm_cache(&self) -> RemoteCache {
        let mut cache = RemoteCache::new();
        let refresh = cache
            .refresh(&self.remote, &self.pair_root.id)
            .await
            .unwrap();
        assert_eq!(refresh, Refresh::Rebuilt);
        cache
    }

    async fn assert_caught_up(&self, cache: &mut RemoteCache) {
        let refresh = cache
            .refresh(&self.remote, &self.pair_root.id)
            .await
            .unwrap();
        assert_eq!(refresh, Refresh::CaughtUp);
        assert_eq!(cache.tree().unwrap(), &self.full_tree().await);
    }
}

fn paths(cache: &RemoteCache) -> Vec<&str> {
    cache
        .tree()
        .unwrap()
        .keys()
        .map(RelativePath::as_str)
        .collect()
}

#[tokio::test]
async fn catching_up_lists_no_folders() {
    let s = Setup::new().await;
    let docs = s.folder(&s.pair_root.id, "docs").await;
    s.file(&docs.id, "a.txt").await;
    let mut cache = s.warm_cache().await;

    s.file(&docs.id, "b.txt").await;
    let before = s.remote.list_calls();
    let refresh = cache.refresh(&s.remote, &s.pair_root.id).await.unwrap();
    assert_eq!(refresh, Refresh::CaughtUp);
    assert_eq!(s.remote.list_calls(), before, "kein Ordner wurde gelesen");
    assert_eq!(cache.tree().unwrap(), &s.full_tree().await);
    assert_eq!(paths(&cache), ["docs", "docs/a.txt", "docs/b.txt"]);
}

#[tokio::test]
async fn renamed_folder_moves_its_whole_subtree() {
    let s = Setup::new().await;
    let docs = s.folder(&s.pair_root.id, "docs").await;
    let deep = s.folder(&docs.id, "tief").await;
    s.file(&deep.id, "a.txt").await;
    // „docs x“ sortiert als Text zwischen „docs“ und „docs/…“.
    s.file(&s.pair_root.id, "docs x").await;
    let mut cache = s.warm_cache().await;

    s.remote.rename(&docs.id, &name("akten")).await.unwrap();
    s.assert_caught_up(&mut cache).await;
    assert_eq!(
        paths(&cache),
        ["akten", "akten/tief", "akten/tief/a.txt", "docs x"]
    );
}

#[tokio::test]
async fn moving_out_removes_and_moving_in_adds_with_content() {
    let s = Setup::new().await;
    let leaving = s.file(&s.pair_root.id, "geht.txt").await;
    let arriving = s.folder(&s.outside.id, "kommt").await;
    s.file(&arriving.id, "inhalt.txt").await;
    let mut cache = s.warm_cache().await;

    s.remote
        .move_node(&leaving.id, &s.outside.id)
        .await
        .unwrap();
    s.remote
        .move_node(&arriving.id, &s.pair_root.id)
        .await
        .unwrap();
    s.assert_caught_up(&mut cache).await;
    assert_eq!(paths(&cache), ["kommt", "kommt/inhalt.txt"]);
}

#[tokio::test]
async fn trash_and_restore_are_followed() {
    let s = Setup::new().await;
    let folder = s.folder(&s.pair_root.id, "ordner").await;
    s.file(&folder.id, "a.txt").await;
    let mut cache = s.warm_cache().await;

    s.remote.trash(&folder.id).await.unwrap();
    s.assert_caught_up(&mut cache).await;
    assert_eq!(paths(&cache), Vec::<&str>::new());

    s.remote.restore(&folder.id).await.unwrap();
    s.assert_caught_up(&mut cache).await;
    assert_eq!(paths(&cache), ["ordner", "ordner/a.txt"]);
}

#[tokio::test]
async fn new_folder_chain_is_found_through_its_ancestors() {
    let s = Setup::new().await;
    let mut cache = s.warm_cache().await;
    // Ordner außen anlegen und befüllen, dann hineinschieben: Für die Datei
    // tief unten kommt das Ereignis, bevor der Index ihre Vorfahren kennt.
    let a = s.folder(&s.outside.id, "a").await;
    let b = s.folder(&a.id, "b").await;
    let file = s.file(&b.id, "c.txt").await;
    s.remote.move_node(&a.id, &s.pair_root.id).await.unwrap();
    s.remote.rename(&file.id, &name("d.txt")).await.unwrap();
    s.assert_caught_up(&mut cache).await;
    assert_eq!(paths(&cache), ["a", "a/b", "a/b/d.txt"]);
}

#[tokio::test]
async fn changes_outside_the_pair_are_ignored_and_remembered() {
    let s = Setup::new().await;
    let mut cache = s.warm_cache().await;
    let elsewhere = s.folder(&s.outside.id, "anderswo").await;
    s.file(&elsewhere.id, "1.txt").await;
    s.assert_caught_up(&mut cache).await;

    let before = s.remote.node_calls();
    s.file(&elsewhere.id, "2.txt").await;
    cache.refresh(&s.remote, &s.pair_root.id).await.unwrap();
    // Nur der Knoten selbst; sein Ordner ist schon als „außerhalb“ bekannt.
    assert_eq!(s.remote.node_calls(), before + 1);
    assert_eq!(paths(&cache), Vec::<&str>::new());
}

#[tokio::test]
async fn a_gap_in_the_event_stream_forces_a_full_read() {
    let s = Setup::new().await;
    let mut cache = s.warm_cache().await;
    s.remote.silence_events(true);
    s.file(&s.pair_root.id, "verpasst.txt").await;
    s.remote.silence_events(false);
    s.remote.push_gap();

    let refresh = cache.refresh(&s.remote, &s.pair_root.id).await.unwrap();
    assert_eq!(refresh, Refresh::Rebuilt);
    assert_eq!(paths(&cache), ["verpasst.txt"]);
}

#[tokio::test]
async fn errors_leave_the_cache_cold_and_invalidate_empties_it() {
    let s = Setup::new().await;
    let mut cache = s.warm_cache().await;
    s.remote
        .fail_next_call(protonsync_application::ports::RemoteError::Unavailable(
            "weg".into(),
        ));
    assert!(cache.refresh(&s.remote, &s.pair_root.id).await.is_err());
    assert!(!cache.is_warm());

    let mut cache = s.warm_cache().await;
    cache.invalidate();
    assert!(cache.tree().is_none());
    assert_eq!(
        cache.refresh(&s.remote, &s.pair_root.id).await.unwrap(),
        Refresh::Rebuilt
    );
}

/// Eine zufällige Änderung bei Proton; Indizes wählen Knoten und Namen modulo Anzahl.
#[derive(Debug, Clone)]
enum Op {
    File { parent: usize, name: usize },
    Folder { parent: usize, name: usize },
    Revise { node: usize },
    Rename { node: usize, name: usize },
    Move { node: usize, target: usize },
    Trash { node: usize },
    Restore { node: usize },
    CatchUp,
}

/// Wenige, teils ähnliche Namen, damit Kollisionen und die Sortierung
/// („a“, „a b“, „a/…“) vorkommen.
const NAMES: [&str; 4] = ["a", "a b", "b", "c.txt"];

fn op() -> impl Strategy<Value = Op> {
    let index = 0..64usize;
    let name = 0..NAMES.len();
    prop_oneof![
        (index.clone(), name.clone()).prop_map(|(parent, name)| Op::File { parent, name }),
        (index.clone(), name.clone()).prop_map(|(parent, name)| Op::Folder { parent, name }),
        index.clone().prop_map(|node| Op::Revise { node }),
        (index.clone(), name).prop_map(|(node, name)| Op::Rename { node, name }),
        (index.clone(), index.clone()).prop_map(|(node, target)| Op::Move { node, target }),
        index.clone().prop_map(|node| Op::Trash { node }),
        index.prop_map(|node| Op::Restore { node }),
        Just(Op::CatchUp),
    ]
}

/// Führt eine Änderung aus; ungültige Änderungen (Namenskollision, Ordner in
/// sich selbst, …) schlagen bei Proton fehl und werden übergangen.
async fn perform(s: &Setup, known: &mut Vec<NodeId>, op: &Op) {
    let pick = |i: usize| known[i % known.len()].clone();
    let result = match op {
        Op::File { parent, name: n } => s
            .remote
            .upload_file(&pick(*parent), &name(NAMES[*n]), &b"x"[..], 1, T0)
            .await
            .map(|node| known.push(node.id)),
        Op::Folder { parent, name: n } => s
            .remote
            .create_folder(&pick(*parent), &name(NAMES[*n]))
            .await
            .map(|node| known.push(node.id)),
        Op::Revise { node } => s
            .remote
            .upload_revision(&pick(*node), &b"neu"[..], 3, T0)
            .await
            .map(drop),
        Op::Rename { node, name: n } => s
            .remote
            .rename(&pick(*node), &name(NAMES[*n]))
            .await
            .map(drop),
        Op::Move { node, target } => s
            .remote
            .move_node(&pick(*node), &pick(*target))
            .await
            .map(drop),
        Op::Trash { node } => s.remote.trash(&pick(*node)).await,
        Op::Restore { node } => s.remote.restore(&pick(*node)).await.map(drop),
        Op::CatchUp => Ok(()),
    };
    drop(result);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn cache_always_matches_a_full_read(ops in prop::collection::vec(op(), 1..40)) {
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        runtime.block_on(async {
            let s = Setup::new().await;
            // Der Wurzelordner des Paares wird nie verschoben oder gelöscht;
            // alles andere darf sich frei bewegen, auch hinaus und hinein.
            let mut known = vec![s.pair_root.id.clone(), s.outside.id.clone()];
            let mut cache = s.warm_cache().await;
            for op in &ops {
                let protected = matches!(
                    op,
                    Op::Rename { node, .. } | Op::Move { node, .. } | Op::Trash { node }
                        if known[node % known.len()] == s.pair_root.id
                );
                if protected {
                    continue;
                }
                perform(&s, &mut known, op).await;
                if matches!(op, Op::CatchUp) {
                    cache.refresh(&s.remote, &s.pair_root.id).await.unwrap();
                    assert_eq!(cache.tree().unwrap(), &s.full_tree().await, "nach {ops:?}");
                }
            }
            cache.refresh(&s.remote, &s.pair_root.id).await.unwrap();
            assert_eq!(cache.tree().unwrap(), &s.full_tree().await, "nach {ops:?}");
        });
    }
}
