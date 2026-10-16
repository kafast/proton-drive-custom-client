//! `PhotoLibrary` auf Proton Photos (ADR 0012).
//!
//! Die Zeitleiste liefert nur Kennungen und Aufnahmezeiten; die Metadaten werden
//! in Stapeln nachgeladen. Zugehörige Fotos (Video-Teil eines Live-Fotos) stehen
//! nicht selbst in der Zeitleiste und werden über ihr Hauptfoto gefunden.

use std::collections::BTreeSet;
use std::io::{Read, Write};

use proton_drive_rs::proton_sdk::ids::{DriveEventId, NodeUid, VolumeId};
use proton_drive_rs::{
    DriveEventScopeId, Node as ProtonNode, NodeKind as ProtonKind, PhotoTag, PhotoUploadMetadata,
    ProtonPhotosClient,
};
use protonsync_application::ports::{
    EventCursor, NewPhoto, PhotoLibrary, RemoteError, RemoteEvent,
};
use protonsync_domain::node::{NodeId, NodeName, RevisionId};
use protonsync_domain::photo::Photo;
use protonsync_domain::time::Timestamp;
use tokio::sync::OnceCell;

use crate::convert;

/// Cursor-Wert, solange das Konto keinen Fotobereich hat.
const NO_LIBRARY_CURSOR: &str = "keine-fotos";

/// Proton Photos des angemeldeten Kontos.
pub struct ProtonPhotos {
    client: ProtonPhotosClient,
    /// Volume des Fotobereichs; `None`, wenn das Konto keinen hat.
    volume: OnceCell<Option<VolumeId>>,
}

impl ProtonPhotos {
    /// Fotobibliothek über einen angemeldeten Drive-Client.
    #[must_use]
    pub fn new(client: proton_drive_rs::ProtonDriveClient) -> Self {
        Self {
            client: ProtonPhotosClient::from_drive_client(client),
            volume: OnceCell::new(),
        }
    }

    async fn volume(&self) -> Result<Option<&VolumeId>, RemoteError> {
        let volume = self
            .volume
            .get_or_try_init(|| async {
                let root = Box::pin(self.client.get_photos_root())
                    .await
                    .map_err(|e| convert::error(&e, None, None))?;
                Ok::<_, RemoteError>(root.map(|root| root.uid.volume_id))
            })
            .await?;
        Ok(volume.as_ref())
    }

    async fn require_volume(&self) -> Result<&VolumeId, RemoteError> {
        self.volume().await?.ok_or_else(|| {
            RemoteError::Rejected(
                "Das Konto hat noch keinen Fotobereich; eine offizielle Proton-App legt ihn an"
                    .into(),
            )
        })
    }

    async fn nodes(&self, uids: &[NodeUid]) -> Result<Vec<ProtonNode>, RemoteError> {
        Box::pin(self.client.enumerate_nodes(uids))
            .await
            .map_err(|e| convert::error(&e, None, None))
    }
}

/// Proton-Knoten als Foto; `None` für Ordner, Alben, Papierkorb und Namen,
/// die hier nicht darstellbar sind.
fn photo(node: &ProtonNode) -> Option<Photo> {
    if node.trashed {
        return None;
    }
    let ProtonKind::File {
        media_type,
        total_size_on_storage,
        active_revision_id,
        claimed_size,
        content_sha1,
        ..
    } = &node.kind
    else {
        return None;
    };
    let properties = node.photo.as_ref();
    Some(Photo {
        id: convert::node_id(&node.uid),
        name: NodeName::new(node.name.clone()).ok()?,
        captured: Timestamp::from_unix_seconds(
            properties.map_or(node.creation_time, |p| p.capture_time),
        ),
        uploaded: Timestamp::from_unix_seconds(node.creation_time),
        revision: RevisionId::new(
            active_revision_id
                .clone()
                .unwrap_or_else(|| format!("mtime-{}", node.modification_time)),
        ),
        size: u64::try_from(claimed_size.unwrap_or(*total_size_on_storage)).unwrap_or_default(),
        content_hash: content_sha1.clone(),
        media_type: media_type.clone(),
        main_photo: properties
            .and_then(|p| p.main_photo_uid.as_ref())
            .map(convert::node_id),
        favorite: properties.is_some_and(|p| p.tags.contains(&PhotoTag::Favorite)),
        albums: properties.map_or(0, |p| p.album_uids.len()),
    })
}

fn to_i64(size: u64) -> Result<i64, RemoteError> {
    i64::try_from(size).map_err(|_| RemoteError::Rejected(format!("Foto mit {size} Bytes zu groß")))
}

impl PhotoLibrary for ProtonPhotos {
    async fn photos(&self) -> Result<Vec<Photo>, RemoteError> {
        if self.volume().await?.is_none() {
            return Ok(Vec::new());
        }
        let timeline = Box::pin(self.client.enumerate_timeline())
            .await
            .map_err(|e| convert::error(&e, None, None))?;
        let uids: Vec<NodeUid> = timeline.into_iter().map(|item| item.uid).collect();
        let mut nodes = self.nodes(&uids).await?;
        // Zugehörige Fotos stehen nicht in der Zeitleiste.
        let known: BTreeSet<String> = nodes.iter().map(|node| node.uid.to_string()).collect();
        let related: Vec<NodeUid> = nodes
            .iter()
            .filter_map(|node| node.photo.as_ref())
            .flat_map(|p| p.related_photo_uids.iter().cloned())
            .filter(|uid| !known.contains(&uid.to_string()))
            .collect();
        if !related.is_empty() {
            nodes.extend(self.nodes(&related).await?);
        }
        Ok(nodes.iter().filter_map(photo).collect())
    }

    async fn photo(&self, id: &NodeId) -> Result<Photo, RemoteError> {
        let uid = convert::node_uid(id)?;
        let node = Box::pin(self.client.get_node(&uid))
            .await
            .map_err(|e| convert::error(&e, Some(id), None))?;
        node.as_ref()
            .and_then(photo)
            .ok_or_else(|| RemoteError::NotFound(id.clone()))
    }

    async fn events_since(
        &self,
        cursor: Option<&EventCursor>,
    ) -> Result<(Vec<RemoteEvent>, EventCursor), RemoteError> {
        let Some(volume) = self.volume().await? else {
            return Ok((Vec::new(), EventCursor::new(NO_LIBRARY_CURSOR)));
        };
        let cursor = match cursor {
            Some(cursor) if cursor.as_str() != NO_LIBRARY_CURSOR => cursor,
            // Erster Aufruf, oder der Fotobereich ist inzwischen entstanden.
            _ => {
                let latest = Box::pin(self.client.drive_client().latest_event_id(volume))
                    .await
                    .map_err(|e| convert::error(&e, None, None))?;
                let gap = cursor.map(|_| vec![RemoteEvent::Gap]).unwrap_or_default();
                return Ok((gap, EventCursor::new(latest.as_str())));
            }
        };
        let events = Box::pin(self.client.enumerate_events(
            &DriveEventScopeId::new(volume.clone()),
            Some(&DriveEventId::new(cursor.as_str())),
        ))
        .await
        .map_err(|e| convert::error(&e, None, None))?;
        Ok(convert::events(events, cursor))
    }

    async fn download_to<W: Write + Send>(
        &self,
        id: &NodeId,
        target: &mut W,
    ) -> Result<u64, RemoteError> {
        self.photo(id).await?;
        let uid = convert::node_uid(id)?;
        let mut counting = convert::CountingWriter::new(target);
        Box::pin(self.client.download_photo_to(&uid, &mut counting))
            .await
            .map_err(|e| convert::error(&e, Some(id), None))?;
        Ok(counting.written())
    }

    async fn upload<R: Read + Send>(
        &self,
        new: &NewPhoto,
        content: R,
    ) -> Result<Photo, RemoteError> {
        self.require_volume().await?;
        let mut counted = convert::CountingReader::new(content);
        let metadata = PhotoUploadMetadata {
            capture_time: Some(new.captured.unix_seconds()),
            main_photo_uid: None,
            tags: Vec::new(),
        };
        let uid = Box::pin(self.client.upload_photo_from(
            new.name.as_str(),
            &new.media_type,
            &mut counted,
            to_i64(new.size)?,
            Vec::new(),
            metadata,
            false,
        ))
        .await
        .map_err(|e| convert::error(&e, None, Some(&new.name)))?;
        let id = convert::node_id(&uid);
        if counted.read() != new.size {
            // Die Datei hat sich während des Hochladens geändert: wieder entfernen.
            let _ = self.trash(&id).await;
            return Err(RemoteError::Transfer(format!(
                "erwartet {} Bytes, gelesen {}",
                new.size,
                counted.read()
            )));
        }
        self.photo(&id).await
    }

    async fn trash(&self, id: &NodeId) -> Result<(), RemoteError> {
        self.photo(id).await?;
        let uid = convert::node_uid(id)?;
        convert::single_outcome(
            id,
            Box::pin(self.client.drive_client().trash_nodes(&[uid])).await,
        )
    }

    async fn restore(&self, id: &NodeId) -> Result<(), RemoteError> {
        let uid = convert::node_uid(id)?;
        convert::single_outcome(
            id,
            Box::pin(self.client.drive_client().restore_nodes(&[uid])).await,
        )
    }

    async fn delete_permanently(&self, id: &NodeId) -> Result<(), RemoteError> {
        let uid = convert::node_uid(id)?;
        match Box::pin(self.client.get_node(&uid)).await {
            Ok(Some(node)) if node.trashed => {}
            Ok(_) => return Err(RemoteError::NotFound(id.clone())),
            Err(error) => return Err(convert::error(&error, Some(id), None)),
        }
        convert::single_outcome(
            id,
            Box::pin(self.client.drive_client().delete_nodes(&[uid])).await,
        )
    }
}
