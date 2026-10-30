//! `RemoteStorage` on Proton Drive.

use std::io::{Read, Write};

use proton_drive_rs::proton_sdk::ids::{DriveEventId, NodeUid};
use proton_drive_rs::{DriveEventScopeId, ProtonDriveClient};
use protonsync_application::ports::{EventCursor, RemoteError, RemoteEvent, RemoteStorage};
use protonsync_domain::node::{Node, NodeId, NodeName};
use protonsync_domain::time::Timestamp;

use crate::convert;

/// Media type for uploaded files, as long as it is not detected.
const DEFAULT_MEDIA_TYPE: &str = "application/octet-stream";

/// Proton Drive, restricted to a root folder (e.g. "My files", a subfolder
/// of it, or a folder in "Shared with me").
pub struct ProtonRemote {
    client: ProtonDriveClient,
    root: NodeUid,
}

impl ProtonRemote {
    /// Works below "My files".
    ///
    /// # Errors
    /// [`RemoteError`] if the folder cannot be determined.
    pub async fn my_files(client: ProtonDriveClient) -> Result<Self, RemoteError> {
        let root = Box::pin(client.get_my_files_folder())
            .await
            .map_err(|e| convert::error(&e, None, None))?;
        Ok(Self {
            root: root.uid,
            client,
        })
    }

    /// Works below the folder named `name` that another user shares with
    /// this account ("Shared with me").
    ///
    /// # Errors
    /// [`RemoteError::Rejected`] if no shared folder has that name or the
    /// name is ambiguous, otherwise [`RemoteError`] from Proton.
    pub async fn shared_with_me(
        client: ProtonDriveClient,
        name: &str,
    ) -> Result<Self, RemoteError> {
        let uids = Box::pin(client.enumerate_shared_with_me_node_uids())
            .await
            .map_err(|e| convert::error(&e, None, None))?;
        let nodes = Box::pin(client.enumerate_nodes(&uids))
            .await
            .map_err(|e| convert::error(&e, None, None))?;
        let mut matching = nodes
            .into_iter()
            .filter(|node| !node.trashed)
            .filter_map(|node| convert::node(&node).map(|domain| (node.uid, domain)))
            .filter(|(_, domain)| domain.name.as_str() == name);
        let (root, domain) = matching.next().ok_or_else(|| {
            RemoteError::Rejected(format!(
                "no folder {name:?} in \"Shared with me\" (invitation accepted?)"
            ))
        })?;
        if matching.next().is_some() {
            return Err(RemoteError::Rejected(format!(
                "several items named {name:?} in \"Shared with me\""
            )));
        }
        if !domain.is_folder() {
            return Err(RemoteError::NotAFolder(domain.id));
        }
        Ok(Self { client, root })
    }

    /// Works below the folder `root` (e.g. a test folder).
    ///
    /// # Errors
    /// [`RemoteError::NotFound`] if `root` is not a valid ID.
    pub fn with_root(client: ProtonDriveClient, root: &NodeId) -> Result<Self, RemoteError> {
        Ok(Self {
            root: convert::node_uid(root)?,
            client,
        })
    }

    /// The underlying client (e.g. for quota queries).
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
            .ok_or_else(|| RemoteError::Rejected(format!("name of {id} cannot be represented")))
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

    fn volume_scope(&self) -> DriveEventScopeId {
        DriveEventScopeId::new(self.root.volume_id.clone())
    }
}

fn to_i64(size: u64) -> Result<i64, RemoteError> {
    i64::try_from(size)
        .map_err(|_| RemoteError::Rejected(format!("file with {size} bytes too large")))
}

impl RemoteStorage for ProtonRemote {
    async fn root_folder(&self) -> Result<Node, RemoteError> {
        let mut root = self.domain_node(&convert::node_id(&self.root)).await?;
        // For the sync the root is the topmost node, even if in Proton itself
        // it lies inside a folder.
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
            // Names that cannot be represented locally are skipped.
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
        let mut counted = convert::CountingReader::new(content);
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
        if counted.read() != size {
            // The source changed during the upload. Remove the file again,
            // so that no wrong version is left behind.
            let _ =
                convert::single_outcome(&node_id, Box::pin(self.client.trash_nodes(&[uid])).await);
            return Err(RemoteError::Transfer(format!(
                "expected {size} bytes, read {}",
                counted.read()
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
        let mut counting = convert::CountingWriter::new(target);
        Box::pin(self.client.download_file_to(&uid, &mut counting))
            .await
            .map_err(|e| convert::error(&e, Some(file), None))?;
        Ok(counting.written())
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
                "a folder cannot be moved into itself".into(),
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
            return Err(RemoteError::InvalidMove("root folder".into()));
        }
        self.proton_node(id).await?;
        convert::single_outcome(id, Box::pin(self.client.trash_nodes(&[uid])).await)
    }

    async fn restore(&self, id: &NodeId) -> Result<Node, RemoteError> {
        let uid = convert::node_uid(id)?;
        convert::single_outcome(id, Box::pin(self.client.restore_nodes(&[uid])).await)?;
        self.domain_node(id).await
    }

    async fn delete_permanently(&self, id: &NodeId) -> Result<(), RemoteError> {
        let uid = convert::node_uid(id)?;
        match Box::pin(self.client.get_node(&uid)).await {
            Ok(Some(node)) if node.trashed => {}
            Ok(_) => return Err(RemoteError::NotFound(id.clone())),
            Err(error) => return Err(convert::error(&error, Some(id), None)),
        }
        convert::single_outcome(id, Box::pin(self.client.delete_nodes(&[uid])).await)
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
        Ok(convert::events(events, cursor))
    }
}
