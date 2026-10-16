//! Planung des Foto-Abgleichs: für jedes Foto bzw. jede lokale Mediendatei
//! bestimmen, was zu tun ist, ohne etwas zu ändern (ADR 0012).
//!
//! Für Fotos mit Basis gilt dieselbe Drei-Wege-Regel wie beim Datei-Abgleich
//! ([`decide`]). Fotos und Dateien ohne Basis werden anschließend zugeordnet:
//! gleicher Inhalt wird nur verknüpft, Neues wird geladen bzw. hochgeladen.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use protonsync_domain::conflict::{Change, SyncAction, decide};
use protonsync_domain::node::{Fingerprint, NodeId};
use protonsync_domain::path::RelativePath;
use protonsync_domain::photo::Photo;
use protonsync_domain::photo::layout::{distinct_name, media_type, month_folder};
use protonsync_domain::sync_pair::SyncDirection;

use crate::ports::{BaseEntry, Calendar, LocalError};
use crate::sync::SkippedPath;

/// Was mit einem Foto bzw. einer Datei geschieht.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhotoStepKind {
    /// Foto nach `path` laden (neu oder in Proton geändert).
    Download(Photo),
    /// Lokale Datei und Foto haben denselben Inhalt: nur verknüpfen.
    Link(Photo),
    /// Neue lokale Datei als Foto hochladen.
    Upload,
    /// In Proton gelöscht: lokale Kopie in den lokalen Papierkorb.
    TrashLocal(NodeId),
    /// Lokal gelöscht: Foto in den Proton-Papierkorb.
    TrashRemote(Photo),
    /// Basis vergessen (beide Seiten gelöscht).
    Forget,
}

/// Ein geplanter Schritt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoStep {
    /// Pfad im Fotoordner.
    pub path: RelativePath,
    /// Was getan wird.
    pub kind: PhotoStepKind,
    /// Lokale Merkmale beim Planen (vor dem Ausführen erneut geprüft).
    pub(crate) local: Option<Fingerprint>,
}

/// Der vollständige Plan eines Laufs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhotoPlan {
    /// Schritte in Ausführungsreihenfolge.
    pub steps: Vec<PhotoStep>,
    /// Übersprungene Pfade mit Begründung (z. B. lokal bearbeitete Fotos).
    pub skipped: Vec<SkippedPath>,
    /// Wegen der Richtung nicht übertragene Änderungen.
    pub ignored: usize,
}

impl PhotoPlan {
    /// `true`, wenn nichts zu tun ist.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty() && self.skipped.is_empty()
    }

    fn step(&mut self, path: &RelativePath, kind: PhotoStepKind, local: Option<&Fingerprint>) {
        self.steps.push(PhotoStep {
            path: path.clone(),
            kind,
            local: local.cloned(),
        });
    }

    fn skip(&mut self, path: &RelativePath, reason: impl Into<String>) {
        self.skipped.push(SkippedPath {
            path: path.clone(),
            reason: reason.into(),
        });
    }
}

/// Alles, was die Planung braucht.
pub(crate) struct PlanInput<'a, H> {
    /// Fotos der Bibliothek.
    pub photos: Vec<&'a Photo>,
    /// Lokale Dateien im Fotoordner (alle, nicht nur Medien).
    pub local: BTreeMap<RelativePath, Fingerprint>,
    /// Gespeicherte Basis.
    pub base: Vec<BaseEntry>,
    /// Richtung.
    pub direction: SyncDirection,
    /// Für die Monatsordner.
    pub calendar: &'a dyn Calendar,
    /// SHA-1 einer lokalen Datei (wird nur bei Bedarf aufgerufen).
    pub hash: H,
}

/// Erstellt den Plan.
pub(crate) fn make_plan<H>(input: PlanInput<'_, H>) -> PhotoPlan
where
    H: FnMut(&RelativePath) -> Result<String, LocalError>,
{
    Planner::new(input).run()
}

struct Planner<'a, H> {
    photos: HashMap<&'a NodeId, &'a Photo>,
    local: BTreeMap<RelativePath, Fingerprint>,
    base: Vec<BaseEntry>,
    direction: SyncDirection,
    calendar: &'a dyn Calendar,
    hash: H,
    hashes: HashMap<RelativePath, Option<String>>,
    /// Pfade, die schon vergeben sind (Basis oder geplant).
    taken: BTreeSet<RelativePath>,
    /// Fotos, die schon zugeordnet sind.
    claimed_photos: BTreeSet<NodeId>,
    /// Lokale Dateien, die schon zugeordnet sind.
    claimed_local: BTreeSet<RelativePath>,
    plan: PhotoPlan,
}

impl<'a, H> Planner<'a, H>
where
    H: FnMut(&RelativePath) -> Result<String, LocalError>,
{
    fn new(input: PlanInput<'a, H>) -> Self {
        let local = input
            .local
            .into_iter()
            .filter(|(path, _)| path.file_name().as_ref().and_then(media_type).is_some())
            .collect();
        Self {
            photos: input.photos.into_iter().map(|p| (&p.id, p)).collect(),
            local,
            base: input.base,
            direction: input.direction,
            calendar: input.calendar,
            hash: input.hash,
            hashes: HashMap::new(),
            taken: BTreeSet::new(),
            claimed_photos: BTreeSet::new(),
            claimed_local: BTreeSet::new(),
            plan: PhotoPlan::default(),
        }
    }

    fn run(mut self) -> PhotoPlan {
        for base in std::mem::take(&mut self.base) {
            self.plan_known(&base);
        }
        if self.direction.allows_download() {
            self.plan_new_photos();
        }
        if self.direction.allows_upload() {
            self.plan_new_files();
        }
        self.plan
    }

    /// Fotos mit Basis: Drei-Wege-Vergleich.
    fn plan_known(&mut self, base: &BaseEntry) {
        let photo = self.photos.get(&base.remote_id).copied();
        let local = self.local.get(&base.path).cloned();
        let local_change = match (&base.local, &local) {
            (_, None) => Change::Deleted,
            (Some(known), Some(current))
                if known.size == current.size && known.modified == current.modified =>
            {
                Change::Unchanged
            }
            _ => Change::Modified,
        };
        let remote_change = match photo {
            None => Change::Deleted,
            Some(photo) if Some(&photo.revision) == base.remote_revision.as_ref() => {
                Change::Unchanged
            }
            Some(_) => Change::Modified,
        };
        let identical = local_change == Change::Modified
            && remote_change == Change::Modified
            && photo.is_some_and(|photo| self.same_content(&base.path, photo));
        let action = decide(local_change, remote_change, identical, self.direction);

        // Was hier zugeordnet bleibt, steht für die zweite Runde nicht mehr zur Wahl.
        let mut keep_claim = true;
        match action {
            SyncAction::Nothing => {}
            SyncAction::IgnoreLocalChange | SyncAction::IgnoreRemoteChange => {
                self.plan.ignored += 1;
            }
            SyncAction::RecordConverged => {
                if let Some(photo) = photo {
                    self.plan.step(
                        &base.path,
                        PhotoStepKind::Link(photo.clone()),
                        local.as_ref(),
                    );
                } else {
                    self.plan.step(&base.path, PhotoStepKind::Forget, None);
                    keep_claim = false;
                }
            }
            SyncAction::Download => {
                if let Some(photo) = photo {
                    self.plan.step(
                        &base.path,
                        PhotoStepKind::Download(photo.clone()),
                        local.as_ref(),
                    );
                }
            }
            SyncAction::Upload => {
                self.plan.skip(
                    &base.path,
                    "lokal bearbeitet; Proton Photos nimmt keine neue Version an",
                );
            }
            SyncAction::TrashLocal => {
                self.plan.step(
                    &base.path,
                    PhotoStepKind::TrashLocal(base.remote_id.clone()),
                    local.as_ref(),
                );
            }
            SyncAction::TrashRemote => {
                if let Some(photo) = photo {
                    self.plan
                        .step(&base.path, PhotoStepKind::TrashRemote(photo.clone()), None);
                }
            }
            SyncAction::Conflict => {
                // Beide Versionen behalten: Die Basis wird vergessen, dann gelten
                // Foto und Datei als neu und werden unten getrennt behandelt.
                self.plan.step(&base.path, PhotoStepKind::Forget, None);
                keep_claim = false;
            }
        }
        if keep_claim {
            self.taken.insert(base.path.clone());
            self.claimed_photos.insert(base.remote_id.clone());
            self.claimed_local.insert(base.path.clone());
        } else if local.is_some() {
            // Die Datei bleibt liegen; ihr Pfad ist weiterhin belegt.
            self.taken.insert(base.path.clone());
        }
    }

    /// Fotos ohne Basis: verknüpfen, wenn der Inhalt schon lokal liegt, sonst laden.
    fn plan_new_photos(&mut self) {
        let mut new_photos: Vec<&Photo> = self
            .photos
            .values()
            .copied()
            .filter(|photo| !self.claimed_photos.contains(&photo.id))
            .collect();
        new_photos.sort_by(|a, b| a.captured.cmp(&b.captured).then_with(|| a.id.cmp(&b.id)));
        for photo in new_photos {
            let Some(target) = self.target_path(photo) else {
                continue;
            };
            if let Some(existing) = self.local_copy_of(photo, &target) {
                let fingerprint = self.local.get(&existing).cloned();
                self.claim(&existing, photo);
                self.plan.step(
                    &existing,
                    PhotoStepKind::Link(photo.clone()),
                    fingerprint.as_ref(),
                );
                continue;
            }
            let path = if self.is_free(&target) {
                target
            } else {
                let Some(folder) = target.parent() else {
                    continue;
                };
                let alternative = folder.join(&distinct_name(&photo.name, &photo.id));
                if !self.is_free(&alternative) {
                    self.plan
                        .skip(&alternative, "Name im Fotoordner schon vergeben");
                    continue;
                }
                alternative
            };
            self.claim(&path, photo);
            self.plan
                .step(&path, PhotoStepKind::Download(photo.clone()), None);
        }
    }

    /// Lokale Mediendateien ohne Basis: hochladen, außer der Inhalt ist schon da.
    fn plan_new_files(&mut self) {
        let new_files: Vec<(RelativePath, Fingerprint)> = self
            .local
            .iter()
            .filter(|(path, _)| !self.claimed_local.contains(*path))
            .map(|(path, fingerprint)| (path.clone(), fingerprint.clone()))
            .collect();
        let by_content: HashMap<(u64, &str), &'a Photo> = self
            .photos
            .values()
            .filter_map(|photo| Some(((photo.size, photo.content_hash.as_deref()?), *photo)))
            .collect();
        for (path, fingerprint) in new_files {
            let Some(hash) = self.local_hash(&path) else {
                self.plan.skip(&path, "Datei nicht lesbar");
                continue;
            };
            let existing = by_content.get(&(fingerprint.size, hash.as_str())).copied();
            match existing {
                Some(photo) if !self.claimed_photos.contains(&photo.id) => {
                    self.claim(&path, photo);
                    self.plan.step(
                        &path,
                        PhotoStepKind::Link(photo.clone()),
                        Some(&fingerprint),
                    );
                }
                Some(photo) => self.plan.skip(
                    &path,
                    format!("dasselbe Foto ist schon verknüpft ({})", photo.name),
                ),
                None => {
                    self.claimed_local.insert(path.clone());
                    self.plan
                        .step(&path, PhotoStepKind::Upload, Some(&fingerprint));
                }
            }
        }
    }

    fn target_path(&mut self, photo: &Photo) -> Option<RelativePath> {
        let Some(captured) = self.calendar.local(photo.captured) else {
            self.plan.skip(
                &RelativePath::root().join(&photo.name),
                "Aufnahmezeit nicht darstellbar",
            );
            return None;
        };
        Some(month_folder(captured).join(&photo.name))
    }

    /// Eine noch nicht zugeordnete lokale Datei mit demselben Inhalt; die am
    /// Zielpfad wird bevorzugt. Gehasht werden nur Dateien gleicher Größe.
    fn local_copy_of(&mut self, photo: &Photo, target: &RelativePath) -> Option<RelativePath> {
        photo.content_hash.as_ref()?;
        let mut candidates: Vec<RelativePath> = self
            .local
            .iter()
            .filter(|(path, fingerprint)| {
                fingerprint.size == photo.size && !self.claimed_local.contains(*path)
            })
            .map(|(path, _)| path.clone())
            .collect();
        candidates.sort_by_key(|path| path != target);
        candidates
            .into_iter()
            .find(|path| self.same_content(path, photo))
    }

    fn same_content(&mut self, path: &RelativePath, photo: &Photo) -> bool {
        photo.content_hash.is_some() && self.local_hash(path) == photo.content_hash
    }

    fn local_hash(&mut self, path: &RelativePath) -> Option<String> {
        if let Some(known) = self.hashes.get(path) {
            return known.clone();
        }
        let hash = (self.hash)(path).ok();
        self.hashes.insert(path.clone(), hash.clone());
        hash
    }

    fn is_free(&self, path: &RelativePath) -> bool {
        !self.taken.contains(path) && !self.local.contains_key(path)
    }

    fn claim(&mut self, path: &RelativePath, photo: &Photo) {
        self.taken.insert(path.clone());
        self.claimed_local.insert(path.clone());
        self.claimed_photos.insert(photo.id.clone());
    }
}
