//! Use Case „Upload-Job ausführen“: passende Dateien aus dem Quellordner
//! hochladen, prüfen und die Aufbewahrungsregel im Zielordner anwenden.
//!
//! Abläufe in Kürze:
//! 1. Quellordner lesen (nur die oberste Ebene), Muster und Ruhezeit anwenden.
//! 2. Zielordner lesen; gleich große Dateien mit gleicher Prüfsumme gelten als
//!    schon hochgeladen. Abweichende Dateien werden als neue Revision geladen
//!    (die alte Version bleibt in Proton im Versionsverlauf).
//! 3. Nach jedem Upload Größe und ggf. Prüfsumme in Proton kontrollieren.
//! 4. Nur wenn alle Uploads gelungen sind: abgelaufene Dateien in den
//!    Proton-Papierkorb legen und im Zustand vermerken (wiederherstellbar).

use std::collections::BTreeMap;

use protonsync_domain::job::{UploadJob, Verification};
use protonsync_domain::node::{Fingerprint, Node, NodeName};
use protonsync_domain::path::RelativePath;
use protonsync_domain::sync_pair::SyncPairId;
use protonsync_domain::time::Timestamp;
use protonsync_domain::trash::{TrashEntry, TrashOrigin};

use super::UseCaseError;
use crate::ports::{Clock, LocalKind, LocalStorage, RemoteError, RemoteStorage, StateRepo};

/// Eine geplante Übertragung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedUpload {
    /// Dateiname.
    pub name: NodeName,
    /// Größe in Bytes.
    pub size: u64,
    /// `true`, wenn eine vorhandene Datei eine neue Revision bekommt.
    pub replaces: bool,
}

/// Was ein Lauf tun würde (Probelauf) bzw. getan hat.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobPlan {
    /// Hochzuladende Dateien.
    pub uploads: Vec<PlannedUpload>,
    /// Schon vorhandene, unveränderte Dateien.
    pub unchanged: Vec<NodeName>,
    /// Dateien, die noch geschrieben werden könnten (Ruhezeit nicht vorbei).
    pub waiting: Vec<NodeName>,
    /// Dateien im Zielordner, die die Aufbewahrung in den Papierkorb legt.
    pub expired: Vec<NodeName>,
}

/// Eine Datei, deren Upload oder Prüfung fehlgeschlagen ist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobFailure {
    /// Dateiname.
    pub name: NodeName,
    /// Fehlermeldung.
    pub message: String,
}

/// Ergebnis eines echten Laufs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobReport {
    /// Hochgeladen und geprüft.
    pub uploaded: Vec<NodeName>,
    /// Schon vorhanden und unverändert.
    pub unchanged: usize,
    /// Wegen der Ruhezeit noch nicht hochgeladen.
    pub waiting: usize,
    /// In den Papierkorb gelegt (Aufbewahrung).
    pub expired: Vec<NodeName>,
    /// Fehlgeschlagene Dateien. Ist die Liste nicht leer, wurde die
    /// Aufbewahrung zur Sicherheit nicht angewendet.
    pub failures: Vec<JobFailure>,
}

impl JobReport {
    /// `true`, wenn alles gelungen ist.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Ergebnis von [`UploadJobRunner::run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobResult {
    /// Probelauf.
    Planned(JobPlan),
    /// Ausgeführt.
    Applied(JobReport),
}

/// Führt einen Upload-Job aus.
pub struct UploadJobRunner<'a, R, L, S, C> {
    /// Proton, mit dem Zielordner als Wurzel.
    pub remote: &'a R,
    /// Der Quellordner.
    pub source: &'a L,
    /// Zustand (für Papierkorbeinträge der Aufbewahrung).
    pub state: &'a S,
    /// Uhr.
    pub clock: &'a C,
    /// Der Job.
    pub job: &'a UploadJob,
    /// Unter dieser Kennung landen Papierkorbeinträge des Jobs.
    pub trash_owner: &'a SyncPairId,
}

/// Eine Datei im Quellordner, die zum Muster passt.
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
    /// Führt den Job einmal aus. Mit `dry_run = true` wird nur geplant.
    ///
    /// # Errors
    /// [`UseCaseError`], wenn Quell- oder Zielordner nicht lesbar sind.
    /// Fehler einzelner Dateien stehen in [`JobReport::failures`].
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
            waiting: plan.waiting.len(),
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
                // Ohne Anmeldung hat kein weiterer Versuch in diesem Lauf Sinn.
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

    /// Passende Dateien der obersten Ebene, getrennt nach „bereit“ und „noch in Ruhezeit“.
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

    /// Alle Einträge des Zielordners nach Namen.
    async fn remote_files(&self, root: &Node) -> Result<BTreeMap<NodeName, Node>, UseCaseError> {
        Ok(self
            .remote
            .list_children(&root.id)
            .await?
            .into_iter()
            .map(|node| (node.name.clone(), node))
            .collect())
    }

    /// Gilt eine vorhandene Datei in Proton als derselbe Inhalt?
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
            // Ohne bekannte Prüfsumme genügt die Größe; sonst würde jeder Lauf
            // dieselbe Datei erneut hochladen.
            _ => Ok(true),
        }
    }

    /// Lädt eine Datei hoch und prüft das Ergebnis.
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

    /// Kontrolliert Größe und ggf. Prüfsumme der Datei in Proton.
    async fn verify(
        &self,
        node: Node,
        size: u64,
        hash: Option<&str>,
    ) -> Result<Node, UseCaseError> {
        // Fehlt die Prüfsumme in der Antwort des Uploads, frisch nachfragen.
        let node = match (
            hash,
            node.fingerprint().and_then(|fp| fp.content_hash.as_ref()),
        ) {
            (Some(_), None) => self.remote.node(&node.id).await?,
            _ => node,
        };
        let remote = node
            .fingerprint()
            .ok_or_else(|| verification_failed("in Proton ist keine Datei angekommen"))?;
        if remote.size != size {
            return Err(verification_failed(&format!(
                "Größe in Proton {} statt {size} Bytes",
                remote.size
            )));
        }
        if let Some(expected) = hash
            && remote.content_hash.as_deref() != Some(expected)
        {
            return Err(verification_failed("Prüfsumme in Proton weicht ab"));
        }
        Ok(node)
    }

    /// Namen der abgelaufenen Dateien unter den zum Muster passenden.
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
        "Prüfung fehlgeschlagen: {reason}"
    )))
}
