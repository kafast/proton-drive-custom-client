//! Find duplicates and move them to the trash on explicit request.
//!
//! Finding is a pure function of the domain; this only adds the
//! deletion: checked against the current groups, always via the trash.

use protonsync_domain::node::NodeId;
use protonsync_domain::path::RelativePath;
use protonsync_domain::photo::duplicates::{DuplicateGroup, check_selection, find_duplicates};
use protonsync_domain::photo::layout::month_folder;
use protonsync_domain::trash::{TrashEntry, TrashOrigin};

use super::{PhotoError, PhotoIndex, photos_owner};
use crate::ports::{Calendar, Clock, PhotoLibrary, StateRepo};

/// A duplicate (marked for deletion or deleted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashedDuplicate {
    /// Identifier of the photo.
    pub id: NodeId,
    /// Path in the photo folder (known or by capture month).
    pub path: RelativePath,
}

/// Duplicates of the library.
pub struct DuplicateService<'a, P, S, C> {
    /// The photo library.
    pub library: &'a P,
    /// Stored state (trash).
    pub state: &'a S,
    /// Clock.
    pub clock: &'a C,
    /// Calendar for paths of photos not downloaded yet.
    pub calendar: &'a dyn Calendar,
}

impl<P, S, C> DuplicateService<'_, P, S, C>
where
    P: PhotoLibrary,
    S: StateRepo,
    C: Clock,
{
    /// Updates `index` and returns the duplicate groups, newest first.
    ///
    /// # Errors
    /// [`PhotoError::Remote`] if the library cannot be read.
    pub async fn find(&self, index: &mut PhotoIndex) -> Result<Vec<DuplicateGroup>, PhotoError> {
        index.refresh(self.library).await?;
        let photos: Vec<_> = index.photos().cloned().collect();
        Ok(find_duplicates(&photos))
    }

    /// Moves the photos `ids` to the trash (with `dry_run` only checks).
    ///
    /// Only identifiers from current groups are allowed, and at least one photo
    /// of each group stays. The local copies follow into the trash
    /// on the next sync as well.
    ///
    /// # Errors
    /// [`PhotoError::Selection`] for a disallowed selection, otherwise errors from
    /// the library or state; what is already deleted stays in the trash.
    pub async fn trash(
        &self,
        index: &mut PhotoIndex,
        ids: &[NodeId],
        dry_run: bool,
    ) -> Result<Vec<TrashedDuplicate>, PhotoError> {
        let groups = self.find(index).await?;
        check_selection(&groups, ids)?;
        let owner = photos_owner();
        let base = self.state.base_entries(&owner)?;
        let mut trashed = Vec::new();
        for id in ids {
            let path = match base.iter().find(|entry| &entry.remote_id == id) {
                Some(entry) => entry.path.clone(),
                None => self.expected_path(index, id),
            };
            if !dry_run {
                self.library.trash(id).await?;
                index.remove(id);
                let entry = TrashEntry {
                    original_path: path.clone(),
                    origin: TrashOrigin::Remote { node: id.clone() },
                    trashed_at: self.clock.now(),
                };
                self.state.add_trash(&owner, &entry)?;
            }
            trashed.push(TrashedDuplicate {
                id: id.clone(),
                path,
            });
        }
        Ok(trashed)
    }

    fn expected_path(&self, index: &PhotoIndex, id: &NodeId) -> RelativePath {
        let Some(photo) = index.get(id) else {
            return RelativePath::root();
        };
        let folder = self
            .calendar
            .local(photo.captured)
            .map_or_else(RelativePath::root, month_folder);
        folder.join(&photo.name)
    }
}
