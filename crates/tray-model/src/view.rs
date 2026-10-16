//! Was die Tray-App anzeigt: ein Abbild von Status, Konflikten und
//! Laufwerken des Daemons, nachgeführt über seine Ereignisse.
//!
//! [`TrayModel`] hält den letzten bekannten Stand; [`TrayModel::view`] macht
//! daraus eine fertig beschriftete [`TrayView`], die QML nur noch anzeigt.

use std::collections::BTreeMap;

use jiff::tz::TimeZone;
use protonsync_api::{
    ConflictInfo, ConflictKindInfo, Direction, Event, JobOutcomeInfo, MountInfo, PairStatus,
    PairSyncResult, Request, Response, StatusInfo, SyncOutcomeInfo,
};
use protonsync_client::i18n::{format_bytes, format_date_time};
use protonsync_client::{ClientError, Lang, Transport};
use serde::Serialize;

/// Gesamtzustand, nach dem sich Icon und Tooltip richten.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrayState {
    /// Kein Daemon erreichbar.
    DaemonOffline,
    /// Daemon läuft, aber niemand ist angemeldet.
    LoggedOut,
    /// Mindestens ein Sync-Paar gleicht gerade ab.
    Syncing,
    /// Der letzte Abgleich oder ein Upload-Job ist gescheitert.
    Error,
    /// Konflikte warten auf eine Entscheidung.
    Attention,
    /// Alles aktuell.
    Idle,
}

impl TrayState {
    /// Name der Icon-Datei (ohne Endung) in den Ressourcen der Tray-App.
    #[must_use]
    pub fn icon(self) -> &'static str {
        match self {
            Self::DaemonOffline | Self::LoggedOut => "offline",
            Self::Syncing => "syncing",
            Self::Error => "error",
            Self::Attention => "attention",
            Self::Idle => "idle",
        }
    }
}

/// Zustand eines Sync-Paares in der Übersicht.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PairState {
    /// Noch nie abgeglichen.
    Never,
    /// Gleicht gerade ab.
    Syncing,
    /// Zuletzt ausgeführt, ohne Fehler.
    Ok,
    /// Zuletzt nur geplant (Probelauf).
    Planned,
    /// Zuletzt gescheitert oder mit Fehlern.
    Error,
}

/// Ein Sync-Paar, fertig beschriftet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PairView {
    /// Kennung.
    pub id: String,
    /// Lokaler Ordner.
    pub local: String,
    /// Ordner in Proton, lesbar.
    pub remote: String,
    /// Richtung, lesbar.
    pub direction: String,
    /// Zustand.
    pub state: PairState,
    /// Eine Zeile zum letzten Abgleich.
    pub status: String,
    /// Offene Konflikte.
    pub open_conflicts: usize,
}

/// Ein FUSE-Laufwerk, fertig beschriftet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MountView {
    /// Kennung.
    pub id: String,
    /// Einhängepunkt.
    pub mountpoint: String,
    /// Ob eingehängt.
    pub mounted: bool,
    /// Eine Zeile zu Zustand und Zwischenspeicher.
    pub status: String,
}

/// Ein offener Konflikt, fertig beschriftet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflictView {
    /// Sync-Paar.
    pub pair: String,
    /// Kennung zum Entscheiden.
    pub id: i64,
    /// Betroffener Pfad.
    pub path: String,
    /// Was passiert ist.
    pub kind: String,
    /// Konfliktkopie, falls vorhanden.
    pub copy: Option<String>,
    /// Wann erkannt.
    pub detected: String,
}

/// Alles, was die Oberfläche gerade zeigt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrayView {
    /// Gesamtzustand.
    pub state: TrayState,
    /// Icon-Name.
    pub icon: &'static str,
    /// Eine Zeile oben im Menü und im Fenster.
    pub headline: String,
    /// Tooltip des Tray-Icons (mehrzeilig).
    pub tooltip: String,
    /// Ob ein Daemon erreichbar ist.
    pub daemon_online: bool,
    /// Version des Daemons.
    pub daemon_version: String,
    /// Angemeldetes Konto.
    pub account: Option<String>,
    /// Ob der Daemon nur plant.
    pub dry_run: bool,
    /// Sync-Paare.
    pub pairs: Vec<PairView>,
    /// Laufwerke.
    pub mounts: Vec<MountView>,
    /// Offene Konflikte.
    pub conflicts: Vec<ConflictView>,
}

/// Status, Konflikte und Laufwerke, wie der Daemon sie gerade meldet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Gesamtzustand.
    pub status: StatusInfo,
    /// Offene Konflikte.
    pub conflicts: Vec<ConflictInfo>,
    /// Laufwerke.
    pub mounts: Vec<MountInfo>,
}

impl Snapshot {
    /// Fragt den Daemon ab. Läuft ohne das Modell, damit dessen Sperre nicht
    /// während der Netzwerkzugriffe gehalten wird.
    ///
    /// # Errors
    /// [`ClientError`], wenn der Daemon nicht oder unerwartet antwortet.
    pub fn fetch(transport: &mut dyn Transport) -> Result<Self, ClientError> {
        let status = match transport.request(Request::Status)? {
            Response::Status(status) => status,
            other => return Err(ClientError::Protocol(format!("{other:?}"))),
        };
        // Ohne Anmeldung kennt der Daemon keine Konflikte; eine Fehlerantwort
        // heißt dann einfach „keine“.
        let conflicts = match transport.request(Request::Conflicts)? {
            Response::Conflicts { conflicts } => conflicts,
            _ => Vec::new(),
        };
        let mounts = match transport.request(Request::Mounts)? {
            Response::Mounts { mounts } => mounts,
            _ => Vec::new(),
        };
        Ok(Self {
            status,
            conflicts,
            mounts,
        })
    }
}

/// Letzter bekannter Stand des Daemons.
pub struct TrayModel {
    lang: Lang,
    zone: TimeZone,
    status: Option<StatusInfo>,
    conflicts: Vec<ConflictInfo>,
    mounts: Vec<MountInfo>,
    /// Gescheiterte Upload-Jobs mit letzter Meldung, bis zum nächsten guten Lauf.
    failed_jobs: BTreeMap<String, String>,
}

impl TrayModel {
    /// Leeres Modell (Daemon noch nicht erreicht).
    #[must_use]
    pub fn new(lang: Lang, zone: TimeZone) -> Self {
        Self {
            lang,
            zone,
            status: None,
            conflicts: Vec::new(),
            mounts: Vec::new(),
            failed_jobs: BTreeMap::new(),
        }
    }

    /// Sprache der Beschriftungen.
    #[must_use]
    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// Wechselt die Sprache.
    pub fn set_lang(&mut self, lang: Lang) {
        self.lang = lang;
    }

    /// Übernimmt einen frisch abgefragten Stand.
    pub fn set_snapshot(&mut self, snapshot: Snapshot) {
        self.status = Some(snapshot.status);
        self.conflicts = snapshot.conflicts;
        self.mounts = snapshot.mounts;
    }

    /// Der Daemon ist weg: alles Bekannte verwerfen.
    pub fn mark_offline(&mut self) {
        self.status = None;
        self.conflicts.clear();
        self.mounts.clear();
    }

    /// Führt ein Ereignis sofort nach, ohne auf die nächste Abfrage zu warten.
    pub fn apply_event(&mut self, event: &Event) {
        match event {
            Event::SyncStarted { pair } => self.update_pair(pair, |p| p.syncing = true),
            Event::SyncFinished(result) => {
                let result = result.clone();
                self.update_pair(&result.pair.clone(), move |p| {
                    p.syncing = false;
                    p.last_sync = Some(result);
                });
            }
            Event::ConflictDetected(conflict) => {
                if !self
                    .conflicts
                    .iter()
                    .any(|c| c.pair == conflict.pair && c.id == conflict.id)
                {
                    self.conflicts.push(conflict.clone());
                }
            }
            Event::LoginRequired { .. } => {
                if let Some(status) = &mut self.status {
                    status.account = None;
                }
            }
            Event::JobGaveUp { job, message, .. } => {
                self.failed_jobs.insert(job.clone(), message.clone());
            }
            Event::JobFinished(run) => {
                if !matches!(run.outcome, JobOutcomeInfo::Failed { .. }) {
                    self.failed_jobs.remove(&run.job);
                }
            }
            Event::JobStarted { .. } | Event::PhotoSyncStarted | Event::PhotoSyncFinished(_) => {}
        }
    }

    /// Entfernt einen entschiedenen Konflikt aus der Liste.
    pub fn forget_conflict(&mut self, pair: &str, id: i64) {
        self.conflicts.retain(|c| !(c.pair == pair && c.id == id));
    }

    fn update_pair(&mut self, id: &str, change: impl FnOnce(&mut PairStatus)) {
        if let Some(pair) = self
            .status
            .as_mut()
            .and_then(|s| s.pairs.iter_mut().find(|p| p.id == id))
        {
            change(pair);
        }
    }

    /// Gesamtzustand.
    #[must_use]
    pub fn state(&self) -> TrayState {
        let Some(status) = &self.status else {
            return TrayState::DaemonOffline;
        };
        if status.account.is_none() {
            TrayState::LoggedOut
        } else if status.pairs.iter().any(|p| p.syncing) {
            TrayState::Syncing
        } else if status
            .pairs
            .iter()
            .any(|p| pair_state(p) == PairState::Error)
            || !self.failed_jobs.is_empty()
        {
            TrayState::Error
        } else if !self.conflicts.is_empty() {
            TrayState::Attention
        } else {
            TrayState::Idle
        }
    }

    /// Fertig beschriftete Ansicht.
    #[must_use]
    pub fn view(&self) -> TrayView {
        let state = self.state();
        let headline = self.headline(state);
        TrayView {
            state,
            icon: state.icon(),
            tooltip: self.tooltip(&headline),
            headline,
            daemon_online: self.status.is_some(),
            daemon_version: self
                .status
                .as_ref()
                .map(|s| s.daemon_version.clone())
                .unwrap_or_default(),
            account: self.status.as_ref().and_then(|s| s.account.clone()),
            dry_run: self.status.as_ref().is_some_and(|s| s.dry_run),
            pairs: self
                .status
                .iter()
                .flat_map(|s| &s.pairs)
                .map(|p| self.pair_view(p))
                .collect(),
            mounts: self.mounts.iter().map(|m| self.mount_view(m)).collect(),
            conflicts: self
                .conflicts
                .iter()
                .map(|c| self.conflict_view(c))
                .collect(),
        }
    }

    fn headline(&self, state: TrayState) -> String {
        let lang = self.lang;
        match state {
            TrayState::DaemonOffline => lang
                .pick("Daemon nicht erreichbar", "Daemon not reachable")
                .into(),
            TrayState::LoggedOut => lang.pick("Nicht angemeldet", "Not logged in").into(),
            TrayState::Syncing => {
                let pairs: Vec<&str> = self
                    .status
                    .iter()
                    .flat_map(|s| &s.pairs)
                    .filter(|p| p.syncing)
                    .map(|p| p.id.as_str())
                    .collect();
                format!(
                    "{}: {}",
                    lang.pick("Gleicht ab", "Syncing"),
                    pairs.join(", ")
                )
            }
            TrayState::Error => {
                if let Some((job, message)) = self.failed_jobs.iter().next() {
                    format!(
                        "{} {job}: {message}",
                        lang.pick("Upload-Job gescheitert:", "Upload job failed:")
                    )
                } else {
                    lang.pick("Fehler beim letzten Abgleich", "The last sync had errors")
                        .into()
                }
            }
            TrayState::Attention => {
                let count = self.conflicts.len();
                if count == 1 {
                    lang.pick("1 Konflikt wartet", "1 conflict waiting").into()
                } else {
                    format!(
                        "{count} {}",
                        lang.pick("Konflikte warten", "conflicts waiting")
                    )
                }
            }
            TrayState::Idle => lang.pick("Alles aktuell", "Everything up to date").into(),
        }
    }

    fn tooltip(&self, headline: &str) -> String {
        let mut lines = vec![format!("protonsync: {headline}")];
        if let Some(status) = &self.status {
            if let Some(account) = &status.account {
                lines.push(account.clone());
            }
            if status.dry_run {
                lines.push(
                    self.lang
                        .pick("Probelauf: nur planen", "Dry run: plan only")
                        .into(),
                );
            }
        }
        lines.join("\n")
    }

    fn pair_view(&self, pair: &PairStatus) -> PairView {
        let lang = self.lang;
        PairView {
            id: pair.id.clone(),
            local: pair.local_root.clone(),
            remote: if pair.remote_path.is_empty() {
                lang.pick("Meine Dateien", "My files").into()
            } else {
                pair.remote_path.clone()
            },
            direction: match pair.direction {
                Direction::TwoWay => lang.pick("beide Richtungen", "two-way"),
                Direction::DownloadOnly => lang.pick("nur herunterladen", "download only"),
                Direction::UploadOnly => lang.pick("nur hochladen", "upload only"),
            }
            .into(),
            state: pair_state(pair),
            status: self.pair_status(pair),
            open_conflicts: pair.open_conflicts,
        }
    }

    fn pair_status(&self, pair: &PairStatus) -> String {
        let lang = self.lang;
        if pair.syncing {
            return lang.pick("Gleicht ab …", "Syncing …").into();
        }
        let Some(PairSyncResult {
            finished_at,
            outcome,
            ..
        }) = &pair.last_sync
        else {
            return lang.pick("Noch nicht abgeglichen", "Not synced yet").into();
        };
        let at = format_date_time(*finished_at, &self.zone);
        match outcome {
            SyncOutcomeInfo::Planned { steps, .. } => format!(
                "{} {at}: {} {}",
                lang.pick("Probelauf", "Dry run"),
                steps.len(),
                lang.pick("Schritte geplant", "steps planned")
            ),
            SyncOutcomeInfo::Applied(summary) => {
                let mut parts = Vec::new();
                for (count, de, en) in [
                    (summary.uploaded, "hoch", "up"),
                    (summary.downloaded, "herunter", "down"),
                    (
                        summary.trashed_local + summary.trashed_remote,
                        "in den Papierkorb",
                        "to trash",
                    ),
                    (summary.conflicts, "Konflikte", "conflicts"),
                    (summary.failures.len(), "Fehler", "errors"),
                ] {
                    if count > 0 {
                        parts.push(format!("{count} {}", lang.pick(de, en)));
                    }
                }
                let detail = if parts.is_empty() {
                    lang.pick("nichts zu tun", "nothing to do").to_owned()
                } else {
                    parts.join(", ")
                };
                format!("{} {at}: {detail}", lang.pick("Abgeglichen", "Synced"))
            }
            SyncOutcomeInfo::Failed { message } => {
                format!("{} {at}: {message}", lang.pick("Fehler", "Error"))
            }
        }
    }

    fn mount_view(&self, mount: &MountInfo) -> MountView {
        let lang = self.lang;
        let state = if mount.mounted {
            lang.pick("eingehängt", "mounted")
        } else if mount.cache.is_some() {
            lang.pick("nicht eingehängt", "not mounted")
        } else {
            lang.pick("nicht verbunden", "not connected")
        };
        let status = match mount.cache {
            Some(cache) => format!(
                "{state}, {} {}, {} {}",
                cache.files,
                lang.pick("Dateien", "files"),
                format_bytes(cache.bytes),
                lang.pick("lokal", "local")
            ),
            None => state.to_owned(),
        };
        MountView {
            id: mount.id.clone(),
            mountpoint: mount.mountpoint.clone(),
            mounted: mount.mounted,
            status,
        }
    }

    fn conflict_view(&self, conflict: &ConflictInfo) -> ConflictView {
        let lang = self.lang;
        ConflictView {
            pair: conflict.pair.clone(),
            id: conflict.id,
            path: conflict.path.clone(),
            kind: match conflict.kind {
                ConflictKindInfo::BothModified => {
                    lang.pick("Auf beiden Seiten geändert", "Changed on both sides")
                }
                ConflictKindInfo::LocalModifiedRemoteDeleted => lang.pick(
                    "Lokal geändert, in Proton gelöscht",
                    "Changed locally, deleted in Proton",
                ),
                ConflictKindInfo::LocalDeletedRemoteModified => lang.pick(
                    "Lokal gelöscht, in Proton geändert",
                    "Deleted locally, changed in Proton",
                ),
            }
            .into(),
            copy: conflict.conflict_copy.clone(),
            detected: format_date_time(conflict.detected_at, &self.zone),
        }
    }
}

fn pair_state(pair: &PairStatus) -> PairState {
    if pair.syncing {
        return PairState::Syncing;
    }
    match pair.last_sync.as_ref().map(|r| &r.outcome) {
        None => PairState::Never,
        Some(SyncOutcomeInfo::Planned { .. }) => PairState::Planned,
        Some(SyncOutcomeInfo::Applied(summary)) if summary.failures.is_empty() => PairState::Ok,
        Some(SyncOutcomeInfo::Applied(_) | SyncOutcomeInfo::Failed { .. }) => PairState::Error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protonsync_api::{CacheInfo, FailureInfo, JobRunInfo, SyncSummary};

    const AT: i64 = 1_760_000_000;

    fn pair(id: &str) -> PairStatus {
        PairStatus {
            id: id.into(),
            local_root: format!("/home/k/{id}"),
            remote_path: String::new(),
            direction: Direction::TwoWay,
            syncing: false,
            last_sync: None,
            open_conflicts: 0,
            trash_entries: 0,
        }
    }

    fn model(lang: Lang, pairs: Vec<PairStatus>) -> TrayModel {
        let mut model = TrayModel::new(lang, TimeZone::UTC);
        model.set_snapshot(Snapshot {
            status: StatusInfo {
                daemon_version: "0.1.0".into(),
                account: Some("k@pm.me".into()),
                login_pending: false,
                dry_run: false,
                pairs,
            },
            conflicts: vec![],
            mounts: vec![],
        });
        model
    }

    fn finished(pair: &str, outcome: SyncOutcomeInfo) -> Event {
        Event::SyncFinished(PairSyncResult {
            pair: pair.into(),
            finished_at: AT,
            outcome,
        })
    }

    fn conflict() -> ConflictInfo {
        ConflictInfo {
            pair: "docs".into(),
            id: 1,
            path: "Bericht.pdf".into(),
            kind: ConflictKindInfo::LocalModifiedRemoteDeleted,
            conflict_copy: Some("Bericht (Konflikt 1).pdf".into()),
            detected_at: AT,
        }
    }

    #[test]
    fn without_a_daemon_everything_is_offline() {
        let model = TrayModel::new(Lang::De, TimeZone::UTC);
        let view = model.view();
        assert_eq!(view.state, TrayState::DaemonOffline);
        assert_eq!(view.icon, "offline");
        assert_eq!(view.headline, "Daemon nicht erreichbar");
        assert!(!view.daemon_online);
    }

    #[test]
    fn states_follow_their_priority() {
        let mut model = model(Lang::En, vec![pair("docs"), pair("fotos")]);
        assert_eq!(model.view().headline, "Everything up to date");

        model.apply_event(&Event::ConflictDetected(conflict()));
        assert_eq!(model.state(), TrayState::Attention);
        assert_eq!(model.view().headline, "1 conflict waiting");

        model.apply_event(&finished(
            "fotos",
            SyncOutcomeInfo::Failed {
                message: "offline".into(),
            },
        ));
        assert_eq!(model.state(), TrayState::Error);

        model.apply_event(&Event::SyncStarted {
            pair: "docs".into(),
        });
        assert_eq!(model.state(), TrayState::Syncing);
        assert_eq!(model.view().headline, "Syncing: docs");
        assert_eq!(model.view().icon, "syncing");

        model.apply_event(&Event::LoginRequired {
            message: "abgelaufen".into(),
        });
        assert_eq!(model.state(), TrayState::LoggedOut);
        assert_eq!(model.view().tooltip, "protonsync: Not logged in");
    }

    #[test]
    fn pair_lines_describe_the_last_sync() {
        let mut model = model(Lang::De, vec![pair("docs")]);
        let line = |model: &TrayModel| model.view().pairs[0].status.clone();
        assert_eq!(line(&model), "Noch nicht abgeglichen");

        model.apply_event(&finished(
            "docs",
            SyncOutcomeInfo::Applied(SyncSummary {
                uploaded: 3,
                downloaded: 2,
                trashed_remote: 1,
                failures: vec![FailureInfo {
                    path: "x".into(),
                    message: "kaputt".into(),
                }],
                ..SyncSummary::default()
            }),
        ));
        assert_eq!(
            line(&model),
            "Abgeglichen 2025-10-09 08:53: 3 hoch, 2 herunter, 1 in den Papierkorb, 1 Fehler"
        );
        assert_eq!(model.view().pairs[0].state, PairState::Error);

        model.apply_event(&finished(
            "docs",
            SyncOutcomeInfo::Applied(SyncSummary::default()),
        ));
        assert_eq!(line(&model), "Abgeglichen 2025-10-09 08:53: nichts zu tun");
        assert_eq!(model.view().pairs[0].state, PairState::Ok);

        model.apply_event(&finished(
            "docs",
            SyncOutcomeInfo::Planned {
                steps: vec![],
                skipped: vec![],
            },
        ));
        assert_eq!(
            line(&model),
            "Probelauf 2025-10-09 08:53: 0 Schritte geplant"
        );

        model.apply_event(&Event::SyncStarted {
            pair: "docs".into(),
        });
        assert_eq!(line(&model), "Gleicht ab …");
        assert_eq!(model.view().pairs[0].remote, "Meine Dateien");
        assert_eq!(model.view().pairs[0].direction, "beide Richtungen");
    }

    #[test]
    fn failed_jobs_count_until_the_next_good_run() {
        let mut model = model(Lang::En, vec![]);
        model.apply_event(&Event::JobGaveUp {
            job: "ha".into(),
            message: "voll".into(),
            attempts: 3,
        });
        assert_eq!(model.view().headline, "Upload job failed: ha: voll");
        model.apply_event(&Event::JobFinished(JobRunInfo {
            job: "ha".into(),
            started_at: AT,
            finished_at: AT,
            outcome: JobOutcomeInfo::Succeeded {
                uploaded: 1,
                unchanged: 0,
                expired: 0,
            },
        }));
        assert_eq!(model.state(), TrayState::Idle);
    }

    #[test]
    fn conflicts_and_mounts_are_labelled() {
        let mut model = model(Lang::En, vec![]);
        model.set_snapshot(Snapshot {
            status: model.status.clone().unwrap(),
            conflicts: vec![conflict()],
            mounts: vec![MountInfo {
                id: "home".into(),
                mountpoint: "/home/k/ProtonDrive".into(),
                remote_path: String::new(),
                mounted: true,
                keep_local: false,
                cache: Some(CacheInfo {
                    files: 3,
                    bytes: 1_500_000,
                    pending_uploads: 0,
                }),
                pins: vec![],
            }],
        });
        let view = model.view();
        assert_eq!(view.conflicts[0].kind, "Changed locally, deleted in Proton");
        assert_eq!(view.conflicts[0].detected, "2025-10-09 08:53");
        assert_eq!(view.mounts[0].status, "mounted, 3 files, 1.5 MB local");

        model.forget_conflict("docs", 1);
        assert_eq!(model.view().conflicts, Vec::<ConflictView>::new());
        model.mark_offline();
        assert_eq!(model.view().mounts, Vec::<MountView>::new());
    }

    #[test]
    fn the_view_serializes_for_qml() {
        let json = serde_json::to_value(model(Lang::En, vec![pair("docs")]).view()).unwrap();
        assert_eq!(json["state"], "idle");
        assert_eq!(json["pairs"][0]["state"], "never");
        assert_eq!(json["dry_run"], false);
    }
}
