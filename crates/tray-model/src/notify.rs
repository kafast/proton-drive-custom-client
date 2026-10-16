//! Welche Ereignisse eine Desktop-Benachrichtigung auslösen.
//!
//! Gemeldet wird nur, was der Nutzer wissen oder entscheiden muss:
//! Konflikte, abgelaufene Anmeldung, gescheiterte Abgleiche und Jobs.
//! Erfolgreiche Läufe bleiben still (sie stehen im Tooltip).

use protonsync_api::{Event, PairSyncResult, SyncOutcomeInfo};
use protonsync_client::Lang;
use serde::Serialize;

/// Wie dringend eine Meldung ist (bestimmt das Symbol der Benachrichtigung).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Hinweis.
    Information,
    /// Etwas wartet auf den Nutzer.
    Warning,
    /// Etwas funktioniert nicht.
    Critical,
}

/// Was ein Klick auf die Benachrichtigung öffnet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationAction {
    /// Die Konfliktliste.
    Conflicts,
    /// Den Anmeldedialog.
    Login,
    /// Die Übersicht.
    Overview,
}

/// Eine Desktop-Benachrichtigung.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notification {
    /// Titel.
    pub title: String,
    /// Text.
    pub body: String,
    /// Dringlichkeit.
    pub severity: Severity,
    /// Ziel eines Klicks.
    pub action: NotificationAction,
}

/// Benachrichtigung zu `event`, falls es eine verdient.
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
        }),
        Event::SyncStarted { .. }
        | Event::SyncFinished(_)
        | Event::JobStarted { .. }
        | Event::JobFinished(_) => None,
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
            message: "abgelaufen".into(),
        };
        let n = notification_for(&login, Lang::De).unwrap();
        assert_eq!(n.title, "Anmeldung erforderlich");
        assert_eq!(n.severity, Severity::Critical);
        assert_eq!(n.action, NotificationAction::Login);
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
            message: "voll".into(),
            attempts: 3,
        };
        assert_eq!(
            notification_for(&gave_up, Lang::De).unwrap().body,
            "ha: voll (3 Versuche)"
        );
    }
}
