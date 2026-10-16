//! Den entfernten Ordner eines Sync-Paares finden oder anlegen.

use protonsync_domain::node::{Node, NodeName};
use protonsync_domain::path::RelativePath;

use crate::ports::{RemoteError, RemoteStorage};

/// Liefert den Ordner `path` unterhalb von `start` und legt fehlende Ordner an.
/// Anlegen ist harmlos (nichts wird überschrieben); eine Datei mit dem Namen
/// eines Ordners führt zu [`RemoteError::NotAFolder`].
///
/// # Errors
/// [`RemoteError`] aus dem entfernten Speicher.
pub async fn ensure_folder<R: RemoteStorage>(
    remote: &R,
    start: &Node,
    path: &RelativePath,
) -> Result<Node, RemoteError> {
    let mut current = start.clone();
    if path.is_root() {
        return Ok(current);
    }
    for segment in path.as_str().split('/') {
        let name = NodeName::new(segment).map_err(|e| RemoteError::Rejected(e.to_string()))?;
        let existing = remote
            .list_children(&current.id)
            .await?
            .into_iter()
            .find(|child| child.name == name);
        current = match existing {
            Some(child) if child.is_folder() => child,
            Some(_) => return Err(RemoteError::NotAFolder(current.id)),
            None => remote.create_folder(&current.id, &name).await?,
        };
    }
    Ok(current)
}
