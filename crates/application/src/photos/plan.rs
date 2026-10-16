//! Planning of the photo sync: for each photo or local media file,
//! determine what to do, without changing anything (ADR 0012).
//!
//! Photos with a base follow the same three-way rule as the file sync
//! ([`decide`]). Photos and files without a base are matched afterwards:
//! the same content is only linked, new items are downloaded or uploaded.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use protonsync_domain::conflict::{Change, SyncAction, decide};
use protonsync_domain::node::{Fingerprint, NodeId};
use protonsync_domain::path::RelativePath;
use protonsync_domain::photo::Photo;
use protonsync_domain::photo::layout::{distinct_name, media_type, month_folder};
use protonsync_domain::sync_pair::SyncDirection;

use crate::ports::{BaseEntry, Calendar, LocalError};
use crate::sync::SkippedPath;

/// What happens to a photo or a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhotoStepKind {
    /// Download the photo to `path` (new or changed in Proton).
    Download(Photo),
    /// Local file and photo have the same content: only link them.
    Link(Photo),
    /// Upload a new local file as a photo.
    Upload,
    /// Deleted in Proton: local copy to the local trash.
    TrashLocal(NodeId),
    /// Deleted locally: photo to the Proton trash.
    TrashRemote(Photo),
    /// Forget the base (both sides deleted).
    Forget,
}

/// A planned step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoStep {
    /// Path in the photo folder.
    pub path: RelativePath,
    /// What is done.
    pub kind: PhotoStepKind,
    /// Local fingerprint at planning time (checked again before executing).
    pub(crate) local: Option<Fingerprint>,
}

/// The complete plan of a run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhotoPlan {
    /// Steps in execution order.
    pub steps: Vec<PhotoStep>,
    /// Skipped paths with the reason (e.g. photos edited locally).
    pub skipped: Vec<SkippedPath>,
    /// Changes not transferred because of the direction.
    pub ignored: usize,
}

impl PhotoPlan {
    /// `true` if there is nothing to do.
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

/// Everything the planning needs.
pub(crate) struct PlanInput<'a, H> {
    /// Photos of the library.
    pub photos: Vec<&'a Photo>,
    /// Local files in the photo folder (all, not only media).
    pub local: BTreeMap<RelativePath, Fingerprint>,
    /// Stored base.
    pub base: Vec<BaseEntry>,
    /// Direction.
    pub direction: SyncDirection,
    /// For the monthly folders.
    pub calendar: &'a dyn Calendar,
    /// SHA-1 of a local file (only called when needed).
    pub hash: H,
}

/// Creates the plan.
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
    /// Paths that are already taken (base or planned).
    taken: BTreeSet<RelativePath>,
    /// Photos that are already matched.
    claimed_photos: BTreeSet<NodeId>,
    /// Local files that are already matched.
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

    /// Photos with a base: three-way comparison.
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

        // Whatever stays matched here is no longer available for the second round.
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
                    "edited locally; Proton Photos does not accept a new version",
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
                // Keep both versions: the base is forgotten, then
                // photo and file count as new and are handled separately below.
                self.plan.step(&base.path, PhotoStepKind::Forget, None);
                keep_claim = false;
            }
        }
        if keep_claim {
            self.taken.insert(base.path.clone());
            self.claimed_photos.insert(base.remote_id.clone());
            self.claimed_local.insert(base.path.clone());
        } else if local.is_some() {
            // The file stays; its path remains taken.
            self.taken.insert(base.path.clone());
        }
    }

    /// Photos without a base: link if the content is already local, otherwise download.
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
                        .skip(&alternative, "name already taken in the photo folder");
                    continue;
                }
                alternative
            };
            self.claim(&path, photo);
            self.plan
                .step(&path, PhotoStepKind::Download(photo.clone()), None);
        }
    }

    /// Local media files without a base: upload, unless the content is already there.
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
                self.plan.skip(&path, "file not readable");
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
                    format!("the same photo is already linked ({})", photo.name),
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
                "capture time not representable",
            );
            return None;
        };
        Some(month_folder(captured).join(&photo.name))
    }

    /// A local file with the same content that is not matched yet; the one at
    /// the target path is preferred. Only files of the same size are hashed.
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
