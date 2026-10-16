//! `RemoteStorage` auf Proton Drive.

use std::io::{Read, Write};

use proton_drive_rs::proton_sdk::ids::{DriveEventId, NodeUid};
use proton_drive_rs::{DriveEvent, DriveEventScopeId, ProtonDriveClient};
use protonsync_application::ports::{EventCursor, RemoteError, RemoteEvent, RemoteStorage};
use protonsync_domain::node::{Node, NodeId, NodeName};
use protonsync_domain::time::Timestamp;

use crate::convert;

/// Medientyp für hochgeladene Dateien, solange er nicht erkannt wird.
const DEFAULT_MEDIA_TYPE: &str = "application/octet-stream";

/// Proton Drive, beschränkt auf einen Wurzelordner (z. B. „Meine Dateien“ oder
/// einen Unterordner davon).
pub struct ProtonRemote {
    client: ProtonDriveClient,
    root: NodeUid,
}

impl ProtonRemote {
    /// Arbeitet unterhalb von „Meine Dateien“.
    ///
    /// # Errors
    /// [`RemoteError`], wenn der Ordner nicht ermittelt werden kann.
    pub async fn my_files(client: ProtonDriveClient) -> Result<Self, RemoteError> {
        let root = Box::pin(client.get_my_files_folder())
            .await
            .map_err(|e| convert::error(&e, None, None))?;
        Ok(Self {
            root: root.uid,
            client,
        })
    }

    /// Arbeitet unterhalb des Ordners `root` (z. B. eines Testordners).
    ///
    /// # Errors
    /// [`RemoteError::NotFound`], wenn `root` keine gültige Kennung ist.
    pub fn with_root(client: ProtonDriveClient, root: &NodeId) -> Result<Self, RemoteError> {
        Ok(Self {
            root: convert::node_uid(root)?,
            client,
        })
    }

    /// Der zugrunde liegende Client (z. B. für Kontingent-Abfragen).
    #[must_use]
    pub fn client(&self) -> &ProtonDriveClient {
        &self.client
    }

    async fn proton_node(&self, id: &NodeId) -> Result<proton_drive_rs::Node, RemoteError> {
        let uid = convert::node_uid(id)?;
        match Box::pin(self.client.get_node(&uid)).await {
            Ok(Some(node)) if !node.trashed => Ok(node),
            Ok(_) => Err(RemoteError::NotFound(id.clone())),
            Err(error) => Err(convert::error(&error, Some(id), None)),
        }
    }

    async fn domain_node(&self, id: &NodeId) -> Result<Node, RemoteError> {
        let proton = self.proton_node(id).await?;
        convert::node(&proton)
            .ok_or_else(|| RemoteError::Rejected(format!("Name von {id} nicht darstellbar")))
    }

    async fn require_folder(&self, id: &NodeId) -> Result<NodeUid, RemoteError> {
        if self.domain_node(id).await?.is_folder() {
            convert::node_uid(id)
        } else {
            Err(RemoteError::NotAFolder(id.clone()))
        }
    }

    async fn require_file(&self, id: &NodeId) -> Result<Node, RemoteError> {
        let node = self.domain_node(id).await?;
        if node.is_folder() {
            Err(RemoteError::NotAFile(id.clone()))
        } else {
            Ok(node)
        }
    }

    /// Prüft das Ergebnis einer Stapeloperation für genau einen Knoten.
    fn single_outcome(
        id: &NodeId,
        outcome: proton_drive_rs::proton_sdk::error::Result<
            Vec<(NodeUid, proton_drive_rs::proton_sdk::error::Result<()>)>,
        >,
    ) -> Result<(), RemoteError> {
        let results = outcome.map_err(|e| convert::error(&e, Some(id), None))?;
        match results.into_iter().next() {
            Some((_, Ok(()))) => Ok(()),
            Some((_, Err(error))) => Err(convert::error(&error, Some(id), None)),
            None => Err(RemoteError::NotFound(id.clone())),
        }
    }

    fn volume_scope(&self) -> DriveEventScopeId {
        DriveEventScopeId::new(self.root.volume_id.clone())
    }
}

/// Zählt die geschriebenen Bytes, weil `download_file_to` sie nicht liefert.
struct CountingWriter<'a, W> {
    inner: &'a mut W,
    written: u64,
}

impl<W: Write> Write for CountingWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn to_i64(size: u64) -> Result<i64, RemoteError> {
    i64::try_from(size)
        .map_err(|_| RemoteError::Rejected(format!("Datei mit {size} Bytes zu groß")))
}

impl RemoteStorage for ProtonRemote {
    async fn root_folder(&self) -> Result<Node, RemoteError> {
        let mut root = self.domain_node(&convert::node_id(&self.root)).await?;
        // Für den Sync ist die Wurzel der oberste Knoten, auch wenn sie in Proton
        // selbst in einem Ordner liegt.
        root.parent = None;
        Ok(root)
    }

    async fn node(&self, id: &NodeId) -> Result<Node, RemoteError> {
        self.domain_node(id).await
    }

    async fn list_children(&self, folder: &NodeId) -> Result<Vec<Node>, RemoteError> {
        let uid = self.require_folder(folder).await?;
        let child_uids = Box::pin(self.client.enumerate_folder_children_node_uids(&uid))
            .await
            .map_err(|e| convert::error(&e, Some(folder), None))?;
        let children = Box::pin(self.client.enumerate_nodes(&child_uids))
            .await
            .map_err(|e| convert::error(&e, Some(folder), None))?;
        let mut nodes: Vec<Node> = children
            .iter()
            .filter(|child| !child.trashed)
            // Namen, die lokal nicht darstellbar sind, werden übersprungen.
            .filter_map(convert::node)
            .collect();
        nodes.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(nodes)
    }

    async fn create_folder(&self, parent: &NodeId, name: &NodeName) -> Result<Node, RemoteError> {
        let parent_uid = self.require_folder(parent).await?;
        let uid = Box::pin(self.client.create_folder(&parent_uid, name.as_str(), None))
            .await
            .map_err(|e| convert::error(&e, Some(parent), Some(name)))?;
        self.domain_node(&convert::node_id(&uid)).await
    }

    async fn upload_file<R: Read + Send>(
        &self,
        parent: &NodeId,
        name: &NodeName,
        content: R,
        size: u64,
        modified: Timestamp,
    ) -> Result<Node, RemoteError> {
        let parent_uid = self.require_folder(parent).await?;
        let mut counted = CountingReader {
            inner: content,
            read: 0,
        };
        let uid = Box::pin(self.client.upload_file_from(
            &parent_uid,
            name.as_str(),
            DEFAULT_MEDIA_TYPE,
            &mut counted,
            to_i64(size)?,
            Vec::new(),
            Some(modified.unix_seconds()),
            false,
        ))
        .await
        .map_err(|e| convert::error(&e, Some(parent), Some(name)))?;
        let node_id = convert::node_id(&uid);
        if counted.read != size {
            // Die Quelle hat sich während des Hochladens geändert. Die Datei
            // wieder entfernen, damit keine falsche Fassung stehen bleibt.
            let _ = Self::single_outcome(&node_id, Box::pin(self.client.trash_nodes(&[uid])).await);
            return Err(RemoteError::Transfer(format!(
                "erwartet {size} Bytes, gelesen {}",
                counted.read
            )));
        }
        self.domain_node(&node_id).await
    }

    async fn upload_revision<R: Read + Send>(
        &self,
        file: &NodeId,
        content: R,
        size: u64,
        modified: Timestamp,
    ) -> Result<Node, RemoteError> {
        self.require_file(file).await?;
        let uid = convert::node_uid(file)?;
        Box::pin(self.client.upload_new_revision_from(
            &uid,
            content,
            to_i64(size)?,
            Vec::new(),
            Some(modified.unix_seconds()),
        ))
        .await
        .map_err(|e| convert::error(&e, Some(file), None))?;
        self.domain_node(file).await
    }

    async fn download_to<W: Write + Send>(
        &self,
        file: &NodeId,
        target: &mut W,
    ) -> Result<u64, RemoteError> {
        self.require_file(file).await?;
        let uid = convert::node_uid(file)?;
        let mut counting = CountingWriter {
            inner: target,
            written: 0,
        };
        Box::pin(self.client.download_file_to(&uid, &mut counting))
            .await
            .map_err(|e| convert::error(&e, Some(file), None))?;
        Ok(counting.written)
    }

    async fn download_range(
        &self,
        file: &NodeId,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, RemoteError> {
        let node = self.require_file(file).await?;
        let size = node.fingerprint().map_or(0, |fingerprint| fingerprint.size);
        if offset > size {
            return Err(RemoteError::InvalidRange { offset, size });
        }
        if offset == size || length == 0 {
            return Ok(Vec::new());
        }
        let uid = convert::node_uid(file)?;
        Box::pin(self.client.download_range(&uid, offset, length))
            .await
            .map_err(|e| convert::error(&e, Some(file), None))
    }

    async fn rename(&self, id: &NodeId, new_name: &NodeName) -> Result<Node, RemoteError> {
        self.proton_node(id).await?;
        let uid = convert::node_uid(id)?;
        Box::pin(self.client.rename_node(&uid, new_name.as_str(), None))
            .await
            .map_err(|e| convert::error(&e, Some(id), Some(new_name)))?;
        self.domain_node(id).await
    }

    async fn move_node(&self, id: &NodeId, new_parent: &NodeId) -> Result<Node, RemoteError> {
        let node = self.domain_node(id).await?;
        let parent_uid = self.require_folder(new_parent).await?;
        let hierarchy = Box::pin(self.client.get_node_hierarchy(&parent_uid))
            .await
            .map_err(|e| convert::error(&e, Some(new_parent), None))?
            .unwrap_or_default();
        let uid = convert::node_uid(id)?;
        if parent_uid == uid || hierarchy.iter().any(|ancestor| ancestor.uid == uid) {
            return Err(RemoteError::InvalidMove(
                "Ordner kann nicht in sich selbst verschoben werden".into(),
            ));
        }
        Box::pin(self.client.move_node(&uid, &parent_uid))
            .await
            .map_err(|e| convert::error(&e, Some(id), Some(&node.name)))?;
        self.domain_node(id).await
    }

    async fn trash(&self, id: &NodeId) -> Result<(), RemoteError> {
        let uid = convert::node_uid(id)?;
        if uid == self.root {
            return Err(RemoteError::InvalidMove("Wurzelordner".into()));
        }
        self.proton_node(id).await?;
        Self::single_outcome(id, Box::pin(self.client.trash_nodes(&[uid])).await)
    }

    async fn restore(&self, id: &NodeId) -> Result<Node, RemoteError> {
        let uid = convert::node_uid(id)?;
        Self::single_outcome(id, Box::pin(self.client.restore_nodes(&[uid])).await)?;
        self.domain_node(id).await
    }

    async fn delete_permanently(&self, id: &NodeId) -> Result<(), RemoteError> {
        let uid = convert::node_uid(id)?;
        match Box::pin(self.client.get_node(&uid)).await {
            Ok(Some(node)) if node.trashed => {}
            Ok(_) => return Err(RemoteError::NotFound(id.clone())),
            Err(error) => return Err(convert::error(&error, Some(id), None)),
        }
        Self::single_outcome(id, Box::pin(self.client.delete_nodes(&[uid])).await)
    }

    async fn events_since(
        &self,
        cursor: Option<&EventCursor>,
    ) -> Result<(Vec<RemoteEvent>, EventCursor), RemoteError> {
        let Some(cursor) = cursor else {
            let latest = Box::pin(self.client.latest_event_id(&self.root.volume_id))
                .await
                .map_err(|e| convert::error(&e, None, None))?;
            return Ok((Vec::new(), EventCursor::new(latest.as_str())));
        };
        let events = Box::pin(self.client.enumerate_events(
            &self.volume_scope(),
            Some(&DriveEventId::new(cursor.as_str())),
        ))
        .await
        .map_err(|e| convert::error(&e, None, None))?;
        let mut next = cursor.clone();
        let mut changes = Vec::new();
        for event in events {
            match event {
                DriveEvent::NodeUpdated {
                    id,
                    node_uid,
                    is_trashed,
                    ..
                } => {
                    let node = convert::node_id(&node_uid);
                    changes.push(if is_trashed {
                        RemoteEvent::Trashed(node)
                    } else {
                        RemoteEvent::Updated(node)
                    });
                    next = EventCursor::new(id.as_str());
                }
                DriveEvent::NodeDeleted { id, node_uid, .. } => {
                    changes.push(RemoteEvent::Trashed(convert::node_id(&node_uid)));
                    next = EventCursor::new(id.as_str());
                }
                // Lücke im Ereignisstrom: Ein vollständiger Abgleich ist nötig.
                DriveEvent::ContinuityLost { id } | DriveEvent::ScopeAccessLost { id } => {
                    changes.push(RemoteEvent::Gap);
                    next = EventCursor::new(id.as_str());
                }
                DriveEvent::CursorAdvanced { id } | DriveEvent::SharedWithMeUpdated { id } => {
                    next = EventCursor::new(id.as_str());
                }
            }
        }
        Ok((changes, next))
    }
}

/// Zählt die gelesenen Bytes, um Änderungen der Quelle während des Uploads zu erkennen.
struct CountingReader<R> {
    inner: R,
    read: u64,
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.read += read as u64;
        Ok(read)
    }
}
