//! Versionierte API zwischen dem Daemon und seinen Oberflächen (CLI, Web, GUI).
//!
//! Übertragung: eine Nachricht pro Zeile als JSON über einen Unix-Socket
//! ([`wire`]). Jede Nachricht trägt die [`PROTOCOL_VERSION`]; der Daemon lehnt
//! fremde Versionen mit [`ErrorCode::VersionMismatch`] ab.
//!
//! Ablauf: Der Client schickt eine [`ClientMessage`] und bekommt genau eine
//! [`ServerMessage::Response`]. Nach [`Request::Subscribe`] bleibt die
//! Verbindung offen und der Daemon schickt [`ServerMessage::Event`]s.
//!
//! Änderungen an diesen Typen sind Vertragsänderungen: Die Tests in
//! `tests/contract.rs` halten das JSON-Format fest.

mod dto;
mod jobs;
pub mod wire;

pub use dto::*;
pub use jobs::*;

use serde::{Deserialize, Serialize};

/// Version des Protokolls. Bei inkompatiblen Änderungen erhöhen.
pub const PROTOCOL_VERSION: u32 = 1;

/// Nachricht vom Client an den Daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientMessage {
    /// Protokollversion des Clients.
    pub version: u32,
    /// Die Anfrage.
    pub request: Request,
}

impl ClientMessage {
    /// Verpackt eine Anfrage mit der aktuellen Protokollversion.
    #[must_use]
    pub fn new(request: Request) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request,
        }
    }
}

/// Nachricht vom Daemon an den Client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "body", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Antwort auf die letzte Anfrage.
    Response(Response),
    /// Ereignis nach [`Request::Subscribe`].
    Event(Event),
}

/// Anfragen an den Daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Gesamtzustand abfragen.
    Status,
    /// Anmeldung beginnen.
    LoginStart {
        /// Proton-Benutzername oder E-Mail.
        username: String,
        /// Login-Passwort.
        password: Password,
    },
    /// 2FA-Code (TOTP) übermitteln.
    LoginSecondFactor {
        /// Sechsstelliger Code der Authenticator-App.
        code: String,
    },
    /// Anmeldung abschließen. `data_password` nur im Zwei-Passwort-Modus.
    LoginFinish {
        /// Mailbox-/Datenpasswort; ohne Angabe gilt das Login-Passwort.
        data_password: Option<Password>,
    },
    /// Abmelden und gespeicherte Sitzung löschen.
    Logout,
    /// Sofort abgleichen. Ohne `apply` nur planen (Probelauf).
    Sync {
        /// Nur dieses Sync-Paar; sonst alle.
        pair: Option<String>,
        /// Änderungen wirklich ausführen.
        apply: bool,
    },
    /// Offene Konflikte aller Sync-Paare.
    Conflicts,
    /// Einen Konflikt entscheiden.
    ResolveConflict {
        /// Sync-Paar.
        pair: String,
        /// Kennung aus [`ConflictInfo::id`].
        id: i64,
        /// Entscheidung.
        resolution: ResolutionChoice,
    },
    /// Papierkorb aller Sync-Paare.
    Trash,
    /// Einen Papierkorbeintrag wiederherstellen.
    RestoreTrash {
        /// Sync-Paar.
        pair: String,
        /// Kennung aus [`TrashInfo::id`].
        id: i64,
    },
    /// Abgelaufene Papierkorbeinträge endgültig löschen. Ohne `apply` nur auflisten.
    PurgeTrash {
        /// Wirklich löschen.
        apply: bool,
    },
    /// Ereignisse abonnieren; die Verbindung bleibt danach offen.
    Subscribe,
    /// Alle Upload-Jobs.
    Jobs,
    /// Einen Upload-Job sofort ausführen. Ohne `apply` nur planen.
    RunJob {
        /// Job.
        job: String,
        /// Wirklich hochladen und aufräumen.
        apply: bool,
    },
    /// Verlauf eines Jobs, neueste Läufe zuerst.
    JobHistory {
        /// Job.
        job: String,
        /// Höchstens so viele Einträge.
        limit: u32,
    },
}

/// Antworten des Daemons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    /// Antwort auf [`Request::Status`].
    Status(StatusInfo),
    /// Nächster Schritt der Anmeldung.
    LoginStep(LoginStep),
    /// Anmeldung abgeschlossen.
    LoggedIn {
        /// Angemeldeter Benutzer.
        username: String,
    },
    /// Ergebnis von [`Request::Sync`], ein Eintrag je Sync-Paar.
    Sync {
        /// Ergebnisse.
        results: Vec<PairSyncResult>,
    },
    /// Antwort auf [`Request::Conflicts`].
    Conflicts {
        /// Offene Konflikte.
        conflicts: Vec<ConflictInfo>,
    },
    /// Antwort auf [`Request::Trash`] und [`Request::PurgeTrash`].
    Trash {
        /// Einträge.
        entries: Vec<TrashInfo>,
    },
    /// Antwort auf [`Request::Jobs`].
    Jobs {
        /// Jobs.
        jobs: Vec<JobInfo>,
    },
    /// Antwort auf [`Request::RunJob`].
    JobRun {
        /// Der Lauf.
        run: JobRunInfo,
        /// Einzelheiten.
        details: JobDetails,
    },
    /// Antwort auf [`Request::JobHistory`].
    JobHistory {
        /// Läufe, neueste zuerst.
        runs: Vec<JobRunInfo>,
    },
    /// Anfrage ausgeführt, nichts weiter zu melden.
    Done,
    /// Anfrage gescheitert.
    Error(ApiError),
}

/// Ereignisse für Abonnenten (GUI, Web).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// Ein Abgleich hat begonnen.
    SyncStarted {
        /// Sync-Paar.
        pair: String,
    },
    /// Ein Abgleich ist fertig.
    SyncFinished(PairSyncResult),
    /// Ein neuer Konflikt wartet auf eine Entscheidung.
    ConflictDetected(ConflictInfo),
    /// Die Anmeldung ist abgelaufen oder ungültig; Nutzer muss sich neu anmelden.
    LoginRequired {
        /// Grund.
        message: String,
    },
    /// Ein Upload-Job hat begonnen.
    JobStarted {
        /// Job.
        job: String,
    },
    /// Ein Upload-Job ist fertig (auch bei Fehlschlag).
    JobFinished(JobRunInfo),
    /// Ein Upload-Job ist nach allen Versuchen gescheitert; Nutzer benachrichtigen.
    JobGaveUp {
        /// Job.
        job: String,
        /// Letzte Fehlermeldung.
        message: String,
        /// Anzahl der Versuche.
        attempts: u32,
    },
}
