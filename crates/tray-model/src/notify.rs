//! Which events trigger a desktop notification.
//!
//! Only what the user has to know or decide is reported: conflicts, an
//! expired login, failed syncs and jobs, and a disk running full. Successful runs stay quiet (they
//! show up in the tooltip).

use protonsync_api::{
    DiskLevelInfo, Event, PairSyncResult, PhotoOutcomeInfo, PhotoRunInfo, SyncOutcomeInfo,
};
use protonsync_client::Lang;
use serde::{Deserialize, Serialize};

/// How urgent a message is (determines the notification icon).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Information.
    Information,
    /// Something is waiting for the user.
    Warning,
    /// Something does not work.
    Critical,
}

/// What a click on the notification opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationAction {
    /// The conflict list.
    Conflicts,
    /// The login dialog.
    Login,
    /// The overview.
    Overview,
}

/// What a notification is about; each topic can be switched off in `tray.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationTopic {
    /// New conflicts.
    Conflicts,
    /// A sync (also of photos) failed or had errors.
    SyncFailure,
    /// An upload job gave up.
    JobFailure,
    /// The login has to be renewed.
    LoginRequired,
    /// A disk is almost full or full (or has space again).
    DiskSpace,
}

/// Which desktop notifications are shown (section `[notification_topics]` in `tray.toml`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)] // One switch per topic is the point here.
pub struct NotificationTopics {
    /// New conflicts.
    pub conflicts: bool,
    /// Failed syncs.
    pub sync_failures: bool,
    /// Failed upload jobs.
    pub job_failures: bool,
    /// Expired login.
    pub login_required: bool,
    /// Disk almost full or full.
    pub disk_space: bool,
}

impl Default for NotificationTopics {
    fn default() -> Self {
        Self {
            conflicts: true,
            sync_failures: true,
            job_failures: true,
            login_required: true,
            disk_space: true,
        }
    }
}

impl NotificationTopics {
    /// Is `topic` shown?
    #[must_use]
    pub const fn shows(&self, topic: NotificationTopic) -> bool {
        match topic {
            NotificationTopic::Conflicts => self.conflicts,
            NotificationTopic::SyncFailure => self.sync_failures,
            NotificationTopic::JobFailure => self.job_failures,
            NotificationTopic::LoginRequired => self.login_required,
            NotificationTopic::DiskSpace => self.disk_space,
        }
    }
}

/// A desktop notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notification {
    /// Title.
    pub title: String,
    /// Body text.
    pub body: String,
    /// Urgency.
    pub severity: Severity,
    /// Target of a click.
    pub action: NotificationAction,
    /// Topic (for switching it off).
    pub topic: NotificationTopic,
}

/// Notification for `event`, if it deserves one.
#[must_use]
pub fn notification_for(event: &Event, lang: Lang) -> Option<Notification> {
    match event {
        Event::ConflictDetected(conflict) => Some(Notification {
            title: lang.pick("Konflikt", "Conflict").into(),
            body: format!(
                "{}: {} {}",
                conflict.pair,
                conflict.path,
                lang.pick(
                    "wartet auf deine Entscheidung. Beide Versionen sind erhalten.",
                    "is waiting for your decision. Both versions are kept."
                )
            ),
            severity: Severity::Warning,
            action: NotificationAction::Conflicts,
            topic: NotificationTopic::Conflicts,
        }),
        Event::LoginRequired { message } => Some(Notification {
            title: lang.pick("Anmeldung erforderlich", "Login required").into(),
            body: format!(
                "{} ({message})",
                lang.pick(
                    "protonsync gleicht nicht mehr ab, bis du dich neu anmeldest.",
                    "protonsync stops syncing until you log in again."
                )
            ),
            severity: Severity::Critical,
            action: NotificationAction::Login,
            topic: NotificationTopic::LoginRequired,
        }),
        Event::SyncFinished(PairSyncResult {
            pair,
            outcome: SyncOutcomeInfo::Failed { message },
            ..
        }) => Some(Notification {
            title: lang.pick("Abgleich gescheitert", "Sync failed").into(),
            body: format!("{pair}: {message}"),
            severity: Severity::Critical,
            action: NotificationAction::Overview,
            topic: NotificationTopic::SyncFailure,
        }),
        Event::SyncFinished(PairSyncResult {
            pair,
            outcome: SyncOutcomeInfo::Applied(summary),
            ..
        }) if !summary.failures.is_empty() => Some(Notification {
            title: lang.pick("Abgleich mit Fehlern", "Sync with errors").into(),
            body: format!(
                "{pair}: {} {}",
                summary.failures.len(),
                lang.pick(
                    "Dateien konnten nicht übertragen werden.",
                    "files could not be transferred."
                )
            ),
            severity: Severity::Warning,
            action: NotificationAction::Overview,
            topic: NotificationTopic::SyncFailure,
        }),
        Event::JobGaveUp {
            job,
            message,
            attempts,
        } => Some(Notification {
            title: lang
                .pick("Upload-Job gescheitert", "Upload job failed")
                .into(),
            body: format!(
                "{job}: {message} ({attempts} {})",
                lang.pick("Versuche", "attempts")
            ),
            severity: Severity::Critical,
            action: NotificationAction::Overview,
            topic: NotificationTopic::JobFailure,
        }),
        Event::PhotoSyncFinished(run) => photo_notification(run, lang),
        Event::DiskSpaceChanged(info) => {
            let (title, body) = protonsync_client::i18n::disk_space_message(lang, info);
            Some(Notification {
                title,
                body,
                severity: match info.level {
                    DiskLevelInfo::Ok => Severity::Information,
                    DiskLevelInfo::Low => Severity::Warning,
                    DiskLevelInfo::Full => Severity::Critical,
                },
                action: NotificationAction::Overview,
                topic: NotificationTopic::DiskSpace,
            })
        }
        Event::SyncStarted { .. }
        | Event::SyncFinished(_)
        | Event::JobStarted { .. }
        | Event::JobFinished(_)
        | Event::PhotoSyncStarted => None,
    }
}

fn photo_notification(run: &PhotoRunInfo, lang: Lang) -> Option<Notification> {
    match run {
        PhotoRunInfo {
            outcome: PhotoOutcomeInfo::Failed { message },
            ..
        } => Some(Notification {
            title: lang
                .pick("Foto-Abgleich gescheitert", "Photo sync failed")
                .into(),
            body: message.clone(),
            severity: Severity::Critical,
            action: NotificationAction::Overview,
            topic: NotificationTopic::SyncFailure,
        }),
        PhotoRunInfo {
            outcome: PhotoOutcomeInfo::Applied(counts),
            ..
        } if !counts.failures.is_empty() => Some(Notification {
            title: lang
                .pick("Foto-Abgleich mit Fehlern", "Photo sync with errors")
                .into(),
            body: format!(
                "{} {}",
                counts.failures.len(),
                lang.pick(
                    "Fotos konnten nicht übertragen werden.",
                    "photos could not be transferred."
                )
            ),
            severity: Severity::Warning,
            action: NotificationAction::Overview,
            topic: NotificationTopic::SyncFailure,
        }),
        PhotoRunInfo { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protonsync_api::{ConflictInfo, ConflictKindInfo, FailureInfo, SyncSummary};

    fn finished(outcome: SyncOutcomeInfo) -> Event {
        Event::SyncFinished(PairSyncResult {
            pair: "docs".into(),
            finished_at: 0,
            outcome,
        })
    }

    #[test]
    fn conflicts_and_logins_ask_for_the_user() {
        let conflict = Event::ConflictDetected(ConflictInfo {
            pair: "docs".into(),
            id: 1,
            path: "a.txt".into(),
            kind: ConflictKindInfo::BothModified,
            conflict_copy: None,
            detected_at: 0,
        });
        let n = notification_for(&conflict, Lang::En).unwrap();
        assert_eq!(n.action, NotificationAction::Conflicts);
        assert!(n.body.starts_with("docs: a.txt is waiting"), "{}", n.body);

        let login = Event::LoginRequired {
            message: "expired".into(),
        };
        let n = notification_for(&login, Lang::De).unwrap();
        assert_eq!(n.title, "Anmeldung erforderlich");
        assert_eq!(n.severity, Severity::Critical);
        assert_eq!(n.action, NotificationAction::Login);
        assert_eq!(n.topic, NotificationTopic::LoginRequired);
    }

    #[test]
    fn topics_can_be_switched_off_one_by_one() {
        let all = NotificationTopics::default();
        assert!(all.shows(NotificationTopic::Conflicts));
        let quiet = NotificationTopics {
            sync_failures: false,
            ..all
        };
        assert!(!quiet.shows(NotificationTopic::SyncFailure));
        assert!(quiet.shows(NotificationTopic::JobFailure));
    }

    #[test]
    fn only_failed_photo_syncs_are_reported() {
        let run = |outcome| {
            Event::PhotoSyncFinished(PhotoRunInfo {
                finished_at: 0,
                outcome,
            })
        };
        let failed = run(PhotoOutcomeInfo::Failed {
            message: "offline".into(),
        });
        assert_eq!(notification_for(&failed, Lang::En).unwrap().body, "offline");
        let partly = run(PhotoOutcomeInfo::Applied(protonsync_api::PhotoCounts {
            failures: vec![FailureInfo {
                path: "2026/10/a.jpg".into(),
                message: "m".into(),
            }],
            ..protonsync_api::PhotoCounts::default()
        }));
        assert_eq!(
            notification_for(&partly, Lang::De).unwrap().severity,
            Severity::Warning
        );
        let fine = run(PhotoOutcomeInfo::Applied(
            protonsync_api::PhotoCounts::default(),
        ));
        assert_eq!(notification_for(&fine, Lang::En), None);
        assert_eq!(notification_for(&Event::PhotoSyncStarted, Lang::En), None);
    }

    #[test]
    fn only_failed_syncs_and_jobs_are_reported() {
        let failed = finished(SyncOutcomeInfo::Failed {
            message: "offline".into(),
        });
        assert_eq!(
            notification_for(&failed, Lang::En).unwrap().body,
            "docs: offline"
        );
        let partly = finished(SyncOutcomeInfo::Applied(SyncSummary {
            failures: vec![FailureInfo {
                path: "x".into(),
                message: "y".into(),
            }],
            ..SyncSummary::default()
        }));
        assert_eq!(
            notification_for(&partly, Lang::En).unwrap().severity,
            Severity::Warning
        );
        let fine = finished(SyncOutcomeInfo::Applied(SyncSummary::default()));
        assert_eq!(notification_for(&fine, Lang::En), None);
        assert_eq!(
            notification_for(
                &Event::SyncStarted {
                    pair: "docs".into()
                },
                Lang::En
            ),
            None
        );
        let gave_up = Event::JobGaveUp {
            job: "ha".into(),
            message: "full".into(),
            attempts: 3,
        };
        assert_eq!(
            notification_for(&gave_up, Lang::De).unwrap().body,
            "ha: full (3 Versuche)"
        );
    }

    #[test]
    fn full_disks_are_critical() {
        use protonsync_api::DiskSpaceInfo;

        let event = Event::DiskSpaceChanged(DiskSpaceInfo {
            level: DiskLevelInfo::Full,
            folders: vec!["main".into()],
            path: "/home/k/Proton".into(),
            available_bytes: 1,
            total_bytes: 100,
        });
        let n = notification_for(&event, Lang::En).unwrap();
        assert_eq!(n.title, "Disk full");
        assert_eq!(n.severity, Severity::Critical);
        assert_eq!(n.topic, NotificationTopic::DiskSpace);
        assert!(NotificationTopics::default().shows(NotificationTopic::DiskSpace));
    }
}
