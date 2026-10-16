//! Use case "run upload job": upload matching files from the source folder,
//! verify them and apply the retention rule in the target folder.
//!
//! Steps in brief:
//! 1. Read the source folder (top level only), apply pattern and settle time.
//! 2. Read the target folder; files of the same size with the same checksum count as
//!    already uploaded. Differing files are uploaded as a new revision
//!    (the old version stays in Proton in the version history).
//! 3. After each upload, check size and, if available, checksum in Proton.
//! 4. Only if all uploads succeeded: move expired files to the
//!    Proton trash and record them in the state (restorable).

use std::collections::BTreeMap;

use protonsync_domain::job::{UploadJob, Verification};
use protonsync_domain::node::{Fingerprint, Node, NodeName};
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::time::Timestamp;
use protonsync_domain::trash::{TrashEntry, TrashOrigin};

use super::UseCaseError;
use crate::ports::{Clock, LocalKind, LocalStorage, RemoteError, RemoteStorage, StateRepo};

/// A planned transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedUpload {
    /// File name.
    pub name: NodeName,
    /// Size in bytes.
    pub size: u64,
    /// `true` if an existing file gets a new revision.
    pub replaces: bool,
}

/// What a run would do (dry run) or did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobPlan {
    /// Files to upload.
    pub uploads: Vec<PlannedUpload>,
    /// Files already present and unchanged.
    pub unchanged: Vec<NodeName>,
    /// Files that might still be written to (settle time not over).
    pub waiting: Vec<NodeName>,
    /// Files in the target folder that retention moves to the trash.
    pub expired: Vec<NodeName>,
}

/// A file whose upload or verification failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobFailure {
    /// File name.
    pub name: NodeName,
    /// Error message.
    pub message: String,
}

/// Result of a real run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobReport {
    /// Uploaded and verified.
    pub uploaded: Vec<NodeName>,
    /// Already present and unchanged.
    pub unchanged: usize,
    /// Not uploaded yet because of the settle time.
    pub waiting: Vec<NodeName>,
    /// Moved to the trash (retention).
    pub expired: Vec<NodeName>,
    /// Failed files. If the list is not empty, retention was not
    /// applied, to be safe.
    pub failures: Vec<JobFailure>,
}

impl JobReport {
    /// `true` if everything succeeded.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Result of [`UploadJobRunner::run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobResult {
    /// Dry run.
    Planned(JobPlan),
    /// Executed.
    Applied(JobReport),
}

/// Runs an upload job.
pub struct UploadJobRunner<'a, R, L, S, C> {
    /// Proton, with the target folder as root.
    pub remote: &'a R,
    /// The source folder.
    pub source: &'a L,
    /// State (for trash entries from retention).
    pub state: &'a S,
    /// Clock.
    pub clock: &'a C,
    /// The job.
    pub job: &'a UploadJob,
    /// Trash entries of the job are stored under this identifier.
    pub trash_owner: &'a SyncPairId,
}

/// A file in the source folder that matches the pattern.
struct SourceFile {
    name: NodeName,
    path: RelativePath,
    fingerprint: Fingerprint,
}

impl<R, L, S, C> UploadJobRunner<'_, R, L, S, C>
where
    R: RemoteStorage,
    L: LocalStorage,
    S: StateRepo,
    C: Clock,
{
    /// Runs the job once. With `dry_run = true` it only plans.
    ///
    /// # Errors
    /// [`UseCaseError`] if the source or target folder cannot be read.
    /// Errors of individual files are in [`JobReport::failures`].
    pub async fn run(&self, dry_run: bool) -> Result<JobResult, UseCaseError> {
        let now = self.clock.now();
        let (ready, waiting) = self.source_files(now)?;
        let root = self.remote.root_folder().await?;
        let mut remote_files = self.remote_files(&root).await?;

        let mut plan = JobPlan {
            waiting,
            ..JobPlan::default()
        };
        let mut to_upload = Vec::new();
        for file in ready {
            match remote_files.get(&file.name) {
                Some(node) if node.is_folder() => {
                    return Err(UseCaseError::Remote(RemoteError::NotAFile(node.id.clone())));
                }
                Some(node) if self.is_unchanged(&file, node)? => plan.unchanged.push(file.name),
                existing => {
                    plan.uploads.push(PlannedUpload {
                        name: file.name.clone(),
                        size: file.fingerprint.size,
                        replaces: existing.is_some(),
                    });
                    to_upload.push(file);
                }
            }
        }

        if dry_run {
            let mut after: BTreeMap<NodeName, Timestamp> = remote_files
                .iter()
                .filter_map(|(name, node)| node.fingerprint().map(|fp| (name.clone(), fp.modified)))
                .collect();
            after.extend(
                to_upload
                    .iter()
                    .map(|f| (f.name.clone(), f.fingerprint.modified)),
            );
            plan.expired = self.expired(&after, now);
            return Ok(JobResult::Planned(plan));
        }

        let mut report = JobReport {
            unchanged: plan.unchanged.len(),
            waiting: plan.waiting.clone(),
            ..JobReport::default()
        };
        for file in to_upload {
            match self
                .upload(&root, remote_files.get(&file.name), &file)
                .await
            {
                Ok(node) => {
                    remote_files.insert(file.name.clone(), node);
                    report.uploaded.push(file.name);
                }
                // Without a login, further attempts in this run make no sense.
                Err(UseCaseError::Remote(RemoteError::Unauthorized)) => {
                    return Err(UseCaseError::Remote(RemoteError::Unauthorized));
                }
                Err(error) => report.failures.push(JobFailure {
                    name: file.name,
                    message: error.to_string(),
                }),
            }
        }
        if report.succeeded() {
            report.expired = self.apply_retention(&remote_files, now).await?;
        }
        Ok(JobResult::Applied(report))
    }

    /// Matching top-level files, split into "ready" and "still in settle time".
    fn source_files(
        &self,
        now: Timestamp,
    ) -> Result<(Vec<SourceFile>, Vec<NodeName>), UseCaseError> {
        let settle_until = now.unix_seconds() - i64::from(self.job.settle_seconds);
        let mut ready = Vec::new();
        let mut waiting = Vec::new();
        for entry in self.source.scan()? {
            let LocalKind::File(fingerprint) = entry.kind else {
                continue;
            };
            let Some(name) = entry.path.file_name() else {
                continue;
            };
            let top_level = entry.path.parent().is_some_and(|parent| parent.is_root());
            if !top_level || !self.job.pattern.matches(name.as_str()) {
                continue;
            }
            if fingerprint.modified.unix_seconds() > settle_until {
                waiting.push(name);
            } else {
                ready.push(SourceFile {
                    name,
                    path: entry.path,
                    fingerprint,
                });
            }
        }
        Ok((ready, waiting))
    }

    /// All entries of the target folder by name.
    async fn remote_files(&self, root: &Node) -> Result<BTreeMap<NodeName, Node>, UseCaseError> {
        Ok(self
            .remote
            .list_children(&root.id)
            .await?
            .into_iter()
            .map(|node| (node.name.clone(), node))
            .collect())
    }

    /// Does an existing file in Proton count as the same content?
    fn is_unchanged(&self, file: &SourceFile, node: &Node) -> Result<bool, UseCaseError> {
        let Some(remote) = node.fingerprint() else {
            return Ok(false);
        };
        if remote.size != file.fingerprint.size {
            return Ok(false);
        }
        match (self.job.verification, &remote.content_hash) {
            (Verification::Hash, Some(remote_hash)) => {
                Ok(&self.source.content_hash(&file.path)? == remote_hash)
            }
            // Without a known checksum the size is enough; otherwise every run would
            // upload the same file again.
            _ => Ok(true),
        }
    }

    /// Uploads a file and verifies the result.
    async fn upload(
        &self,
        root: &Node,
        existing: Option<&Node>,
        file: &SourceFile,
    ) -> Result<Node, UseCaseError> {
        let expected_hash = match self.job.verification {
            Verification::Hash => Some(self.source.content_hash(&file.path)?),
            Verification::Size => None,
        };
        let content = self.source.open(&file.path)?;
        let size = file.fingerprint.size;
        let modified = file.fingerprint.modified;
        let node = match existing {
            Some(node) => {
                self.remote
                    .upload_revision(&node.id, content, size, modified)
                    .await?
            }
            None => {
                self.remote
                    .upload_file(&root.id, &file.name, content, size, modified)
                    .await?
            }
        };
        self.verify(node, size, expected_hash.as_deref()).await
    }

    /// Checks size and, if available, checksum of the file in Proton.
    async fn verify(
        &self,
        node: Node,
        size: u64,
        hash: Option<&str>,
    ) -> Result<Node, UseCaseError> {
        // If the upload response lacks the checksum, ask again freshly.
        let node = match (
            hash,
            node.fingerprint().and_then(|fp| fp.content_hash.as_ref()),
        ) {
            (Some(_), None) => self.remote.node(&node.id).await?,
            _ => node,
        };
        let remote = node
            .fingerprint()
            .ok_or_else(|| verification_failed("no file arrived in Proton"))?;
        if remote.size != size {
            return Err(verification_failed(&format!(
                "size in Proton is {} instead of {size} bytes",
                remote.size
            )));
        }
        if let Some(expected) = hash
            && remote.content_hash.as_deref() != Some(expected)
        {
            return Err(verification_failed("checksum in Proton differs"));
        }
        Ok(node)
    }

    /// Names of the expired files among those matching the pattern.
    fn expired(&self, files: &BTreeMap<NodeName, Timestamp>, now: Timestamp) -> Vec<NodeName> {
        let candidates: Vec<(NodeName, Timestamp)> = files
            .iter()
            .filter(|(name, _)| self.job.pattern.matches(name.as_str()))
            .map(|(name, modified)| (name.clone(), *modified))
            .collect();
        self.job
            .retention
            .expired(&candidates, now)
            .into_iter()
            .cloned()
            .collect()
    }

    async fn apply_retention(
        &self,
        remote_files: &BTreeMap<NodeName, Node>,
        now: Timestamp,
    ) -> Result<Vec<NodeName>, UseCaseError> {
        let times: BTreeMap<NodeName, Timestamp> = remote_files
            .iter()
            .filter_map(|(name, node)| node.fingerprint().map(|fp| (name.clone(), fp.modified)))
            .collect();
        let mut trashed = Vec::new();
        for name in self.expired(&times, now) {
            let Some(node) = remote_files.get(&name) else {
                continue;
            };
            self.remote.trash(&node.id).await?;
            self.state.add_trash(
                self.trash_owner,
                &TrashEntry {
                    original_path: RelativePath::root().join(&name),
                    origin: TrashOrigin::Remote {
                        node: node.id.clone(),
                    },
                    trashed_at: now,
                },
            )?;
            trashed.push(name);
        }
        Ok(trashed)
    }
}

fn verification_failed(reason: &str) -> UseCaseError {
    UseCaseError::Remote(RemoteError::Transfer(format!(
        "verification failed: {reason}"
    )))
}
