//! [`RemoteStorage`] on a folder, served by the SDK helper process.

use std::io::{Read, Write};
use std::sync::Arc;

use protonsync_application::ports::{EventCursor, RemoteError, RemoteEvent, RemoteStorage};
use protonsync_domain::node::{Node, NodeId, NodeName};
use protonsync_domain::time::Timestamp;
use serde_json::json;

use crate::connection::Connection;
use crate::protocol::{
    CHUNK_BYTES, Chunk, DownloadHandle, UploadHandle, WireEvents, WireNode, decode, encode,
};

/// Remote storage below one folder (the folder of a sync pair).
///
/// Several `HelperRemote`s may share one helper [`Connection`].
#[derive(Clone)]
pub struct HelperRemote {
    connection: Arc<Connection>,
    root: NodeId,
}

impl HelperRemote {
    /// Storage whose [`RemoteStorage::root_folder`] is `root`.
    #[must_use]
    pub fn new(connection: Arc<Connection>, root: NodeId) -> Self {
        Self { connection, root }
    }

    async fn node_call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<Node, RemoteError> {
        let node: WireNode = self.connection.call(method, &params).await?;
        node.into_node()
    }

    async fn unit_call(&self, method: &str, params: serde_json::Value) -> Result<(), RemoteError> {
        let _: serde_json::Value = self.connection.call(method, &params).await?;
        Ok(())
    }

    /// Streams `content` into an opened upload and commits it. Reads at most
    /// one byte more than announced, so a growing source is detected without
    /// reading it to the end.
    async fn send_content<R: Read + Send>(
        &self,
        upload: UploadHandle,
        mut content: R,
        size: u64,
    ) -> Result<Node, RemoteError> {
        let mut buffer = vec![0_u8; CHUNK_BYTES];
        let mut sent: u64 = 0;
        loop {
            let read = match content.read(&mut buffer) {
                Ok(read) => read,
                Err(e) => {
                    self.abort(&upload).await;
                    return Err(RemoteError::Transfer(format!("reading the source: {e}")));
                }
            };
            if read == 0 {
                break;
            }
            sent += read as u64;
            if sent > size {
                break;
            }
            let params = json!({ "upload": upload.upload, "data": encode(&buffer[..read]) });
            if let Err(error) = self.unit_call("upload_write", params).await {
                self.abort(&upload).await;
                return Err(error);
            }
        }
        if sent != size {
            self.abort(&upload).await;
            return Err(RemoteError::Transfer(format!(
                "expected {size} bytes, read {}",
                if sent > size {
                    format!("more than {size}")
                } else {
                    sent.to_string()
                }
            )));
        }
        self.node_call("upload_commit", json!({ "upload": upload.upload }))
            .await
    }

    /// Best effort: the upload already failed, so an error here changes nothing.
    async fn abort(&self, upload: &UploadHandle) {
        let _ = self
            .unit_call("upload_abort", json!({ "upload": upload.upload }))
            .await;
    }

    async fn copy_download<W: Write + Send>(
        &self,
        download: &DownloadHandle,
        target: &mut W,
    ) -> Result<u64, RemoteError> {
        let mut written: u64 = 0;
        loop {
            let chunk: Chunk = self
                .connection
                .call(
                    "download_read",
                    &json!({ "download": download.download, "max": CHUNK_BYTES }),
                )
                .await?;
            let bytes = decode(&chunk.data)?;
            target
                .write_all(&bytes)
                .map_err(|e| RemoteError::Transfer(format!("writing the target: {e}")))?;
            written += bytes.len() as u64;
            if chunk.eof {
                return Ok(written);
            }
        }
    }
}

impl RemoteStorage for HelperRemote {
    async fn root_folder(&self) -> Result<Node, RemoteError> {
        self.node(&self.root).await
    }

    async fn node(&self, id: &NodeId) -> Result<Node, RemoteError> {
        self.node_call("node", json!({ "node": id.as_str() })).await
    }

    async fn list_children(&self, folder: &NodeId) -> Result<Vec<Node>, RemoteError> {
        let children: Vec<WireNode> = self
            .connection
            .call("list_children", &json!({ "folder": folder.as_str() }))
            .await?;
        let mut nodes = children
            .into_iter()
            .map(WireNode::into_node)
            .collect::<Result<Vec<_>, _>>()?;
        // The port promises name order; the SDK lists in its own order.
        nodes.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(nodes)
    }

    async fn create_folder(&self, parent: &NodeId, name: &NodeName) -> Result<Node, RemoteError> {
        self.node_call(
            "create_folder",
            json!({ "parent": parent.as_str(), "name": name.as_str() }),
        )
        .await
    }

    async fn upload_file<R: Read + Send>(
        &self,
        parent: &NodeId,
        name: &NodeName,
        content: R,
        size: u64,
        modified: Timestamp,
    ) -> Result<Node, RemoteError> {
        let upload: UploadHandle = self
            .connection
            .call(
                "upload_open",
                &json!({
                    "parent": parent.as_str(),
                    "name": name.as_str(),
                    "size": size,
                    "modified": modified.unix_seconds(),
                }),
            )
            .await?;
        self.send_content(upload, content, size).await
    }

    async fn upload_revision<R: Read + Send>(
        &self,
        file: &NodeId,
        content: R,
        size: u64,
        modified: Timestamp,
    ) -> Result<Node, RemoteError> {
        let upload: UploadHandle = self
            .connection
            .call(
                "revision_open",
                &json!({
                    "file": file.as_str(),
                    "size": size,
                    "modified": modified.unix_seconds(),
                }),
            )
            .await?;
        self.send_content(upload, content, size).await
    }

    async fn download_to<W: Write + Send>(
        &self,
        file: &NodeId,
        target: &mut W,
    ) -> Result<u64, RemoteError> {
        let download: DownloadHandle = self
            .connection
            .call("download_open", &json!({ "file": file.as_str() }))
            .await?;
        let result = self.copy_download(&download, target).await;
        // Frees the helper's stream also after an error; nothing to do if that fails.
        let _ = self
            .unit_call("download_close", json!({ "download": download.download }))
            .await;
        result
    }

    async fn download_range(
        &self,
        file: &NodeId,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, RemoteError> {
        let chunk: Chunk = self
            .connection
            .call(
                "download_range",
                &json!({ "file": file.as_str(), "offset": offset, "length": length }),
            )
            .await?;
        decode(&chunk.data)
    }

    async fn rename(&self, id: &NodeId, new_name: &NodeName) -> Result<Node, RemoteError> {
        self.node_call(
            "rename",
            json!({ "node": id.as_str(), "name": new_name.as_str() }),
        )
        .await
    }

    async fn move_node(&self, id: &NodeId, new_parent: &NodeId) -> Result<Node, RemoteError> {
        self.node_call(
            "move",
            json!({ "node": id.as_str(), "parent": new_parent.as_str() }),
        )
        .await
    }

    async fn trash(&self, id: &NodeId) -> Result<(), RemoteError> {
        self.unit_call("trash", json!({ "node": id.as_str() }))
            .await
    }

    async fn restore(&self, id: &NodeId) -> Result<Node, RemoteError> {
        self.node_call("restore", json!({ "node": id.as_str() }))
            .await
    }

    async fn delete_permanently(&self, id: &NodeId) -> Result<(), RemoteError> {
        self.unit_call("delete", json!({ "node": id.as_str() }))
            .await
    }

    async fn events_since(
        &self,
        cursor: Option<&EventCursor>,
    ) -> Result<(Vec<RemoteEvent>, EventCursor), RemoteError> {
        let events: WireEvents = self
            .connection
            .call(
                "events_since",
                &json!({ "cursor": cursor.map(EventCursor::as_str) }),
            )
            .await?;
        Ok(events.into_port())
    }
}
