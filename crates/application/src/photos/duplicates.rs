//! Duplikate finden und auf ausdrücklichen Wunsch in den Papierkorb legen.
//!
//! Das Finden ist eine reine Funktion der Domäne; hier kommt nur das Löschen
//! dazu: geprüft gegen die aktuellen Gruppen, immer über den Papierkorb.

use protonsync_domain::node::NodeId;
use protonsync_domain::path::RelativePath;
use protonsync_domain::photo::duplicates::{DuplicateGroup, check_selection, find_duplicates};
use protonsync_domain::photo::layout::month_folder;
use protonsync_domain::trash::{TrashEntry, TrashOrigin};

use super::{PhotoError, PhotoIndex, photos_owner};
use crate::ports::{Calendar, Clock, PhotoLibrary, StateRepo};

/// Ein (zum Löschen vorgemerktes oder gelöschtes) Duplikat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashedDuplicate {
    /// Kennung des Fotos.
    pub id: NodeId,
    /// Pfad im Fotoordner (bekannt oder nach Aufnahmemonat).
    pub path: RelativePath,
}

/// Duplikate der Bibliothek.
pub struct DuplicateService<'a, P, S, C> {
    /// Die Fotobibliothek.
    pub library: &'a P,
    /// Gespeicherter Zustand (Papierkorb).
    pub state: &'a S,
    /// Uhr.
    pub clock: &'a C,
    /// Kalender für Pfade noch nicht geladener Fotos.
    pub calendar: &'a dyn Calendar,
}

impl<P, S, C> DuplicateService<'_, P, S, C>
where
    P: PhotoLibrary,
    S: StateRepo,
    C: Clock,
{
    /// Aktualisiert `index` und liefert die Duplikat-Gruppen, neueste zuerst.
    ///
    /// # Errors
    /// [`PhotoError::Remote`], wenn die Bibliothek nicht lesbar ist.
    pub async fn find(&self, index: &mut PhotoIndex) -> Result<Vec<DuplicateGroup>, PhotoError> {
        index.refresh(self.library).await?;
        let photos: Vec<_> = index.photos().cloned().collect();
        Ok(find_duplicates(&photos))
    }

    /// Legt die Fotos `ids` in den Papierkorb (mit `dry_run` nur prüfen).
    ///
    /// Erlaubt sind nur Kennungen aus aktuellen Gruppen, und von jeder Gruppe
    /// bleibt mindestens ein Foto. Die lokalen Kopien folgen beim nächsten
    /// Abgleich ebenfalls in den Papierkorb.
    ///
    /// # Errors
    /// [`PhotoError::Selection`] bei unerlaubter Auswahl, sonst Fehler von
    /// Bibliothek oder Zustand; bereits Gelöschtes bleibt im Papierkorb.
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
