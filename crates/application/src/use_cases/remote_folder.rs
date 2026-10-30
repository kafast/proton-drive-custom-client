//! Find or create the remote folder of a sync pair.

use protonsync_domain::node::{Node, NodeName};
use protonsync_domain::path::RelativePath;

use crate::ports::{RemoteError, RemoteStorage};

/// Returns the folder `path` below `start` and creates missing folders.
/// Creating is harmless (nothing is overwritten); a file with the name
/// of a folder leads to [`RemoteError::NotAFolder`].
///
/// # Errors
/// [`RemoteError`] from the remote storage.
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

/// Returns the folder `path` below `start` without creating anything
/// (for folders others share with us read-only); `None` if a segment is missing.
///
/// # Errors
/// [`RemoteError::NotAFolder`] if a file stands in the way, otherwise
/// [`RemoteError`] from the remote storage.
pub async fn find_folder<R: RemoteStorage>(
    remote: &R,
    start: &Node,
    path: &RelativePath,
) -> Result<Option<Node>, RemoteError> {
    let mut current = start.clone();
    for segment in path.segments() {
        let existing = remote
            .list_children(&current.id)
            .await?
            .into_iter()
            .find(|child| child.name.as_str() == segment);
        current = match existing {
            Some(child) if child.is_folder() => child,
            Some(_) => return Err(RemoteError::NotAFolder(current.id)),
            None => return Ok(None),
        };
    }
    Ok(Some(current))
}
