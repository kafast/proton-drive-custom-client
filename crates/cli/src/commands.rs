//! Befehle der Kommandozeile und ihre Ausführung gegen den Daemon.

use std::io::{self, Write};
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use jiff::tz::TimeZone;
use protonsync_api::{
    ErrorCode, JobOutcomeInfo, Password, PinChoice, Request, ResolutionChoice, Response,
    SyncOutcomeInfo,
};

use crate::client::{ClientError, Transport};
use crate::i18n::Lang;
use crate::render;

/// Exit-Codes (auch für Skripte gedacht, daher stabil).
pub mod exit {
    /// Erfolg.
    pub const OK: u8 = 0;
    /// Allgemeiner Fehler.
    pub const ERROR: u8 = 1;
    /// Falsche Bedienung (Argumente).
    pub const USAGE: u8 = 2;
    /// Daemon nicht erreichbar.
    pub const DAEMON_UNREACHABLE: u8 = 3;
    /// Nicht angemeldet.
    pub const NOT_LOGGED_IN: u8 = 4;
    /// Anmeldung abgelehnt.
    pub const LOGIN_REJECTED: u8 = 5;
    /// CLI und Daemon passen nicht zusammen.
    pub const VERSION_MISMATCH: u8 = 6;
    /// Abgleich lief, aber nicht alles hat geklappt.
    pub const PARTIAL: u8 = 7;
}

/// Inoffizieller Proton-Drive-Client für Linux (nicht von Proton unterstützt).
#[derive(Debug, Parser)]
#[command(name = "protonsync", version)]
pub struct Cli {
    /// Ausgabe als JSON (für Skripte)
    #[arg(long, global = true)]
    pub json: bool,
    /// Socket des Daemons (Standard: `$PROTONSYNC_SOCKET` bzw. `$XDG_RUNTIME_DIR/protonsync/daemon.sock`)
    #[arg(long, global = true)]
    pub socket: Option<PathBuf>,
    /// Befehl
    #[command(subcommand)]
    pub command: Command,
}

/// Befehle.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Bei Proton anmelden (Passwort, 2FA, ggf. Datenpasswort)
    Login {
        /// Benutzername; sonst wird gefragt
        #[arg(long)]
        username: Option<String>,
        /// Passwörter zeilenweise von der Standardeingabe lesen statt vom Terminal
        #[arg(long)]
        password_stdin: bool,
    },
    /// Abmelden und gespeicherte Sitzung löschen
    Logout,
    /// Zustand von Konto und Sync-Paaren
    Status,
    /// Jetzt abgleichen (ohne --apply nur planen)
    Sync {
        /// Nur dieses Sync-Paar
        #[arg(long)]
        pair: Option<String>,
        /// Änderungen wirklich ausführen
        #[arg(long)]
        apply: bool,
    },
    /// Konflikte anzeigen und entscheiden
    Conflicts {
        /// Aktion (Standard: list)
        #[command(subcommand)]
        action: Option<ConflictCommand>,
    },
    /// Papierkorb anzeigen, wiederherstellen, leeren
    Trash {
        /// Aktion (Standard: list)
        #[command(subcommand)]
        action: Option<TrashCommand>,
    },
    /// Upload-Jobs anzeigen, ausführen, Verlauf
    Jobs {
        /// Aktion (Standard: list)
        #[command(subcommand)]
        action: Option<JobCommand>,
    },
    /// FUSE-Laufwerke mit Zwischenspeicher und Regeln anzeigen
    Mounts,
    /// „Immer lokal halten“: Datei oder Ordner in einem Laufwerk vollständig laden
    Pin {
        /// Pfad im Laufwerk (z. B. ~/ProtonDrive/Fotos)
        path: PathBuf,
    },
    /// „Nur bei Bedarf“: Datei oder Ordner darf wieder freigegeben werden
    Unpin {
        /// Pfad im Laufwerk
        path: PathBuf,
    },
    /// Speicher freigeben (ohne --apply nur planen). Mit Pfad sofort alles
    /// darunter, sonst nach der Freigaberegel ([cache])
    Free {
        /// Pfad im Laufwerk; ohne: alle Laufwerke nach der Freigaberegel
        path: Option<PathBuf>,
        /// Wirklich freigeben
        #[arg(long)]
        apply: bool,
    },
    /// Konfiguration
    Config {
        /// Aktion
        #[command(subcommand)]
        action: ConfigCommand,
    },
}

/// Konflikt-Befehle.
#[derive(Debug, Subcommand)]
pub enum ConflictCommand {
    /// Offene Konflikte auflisten
    List,
    /// Einen Konflikt entscheiden
    Resolve {
        /// Sync-Paar
        pair: String,
        /// Kennung aus `conflicts list`
        id: i64,
        /// Entscheidung
        choice: Choice,
    },
}

/// Entscheidung über einen Konflikt.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Choice {
    /// Lokale Version behalten, die entfernte geht in den Papierkorb
    KeepLocal,
    /// Entfernte Version behalten, die lokale geht in den Papierkorb
    KeepRemote,
    /// Beide behalten (Konfliktkopie bleibt)
    KeepBoth,
}

/// Papierkorb-Befehle.
#[derive(Debug, Subcommand)]
pub enum TrashCommand {
    /// Einträge auflisten
    List,
    /// Einen Eintrag wiederherstellen
    Restore {
        /// Sync-Paar
        pair: String,
        /// Kennung aus `trash list`
        id: i64,
    },
    /// Abgelaufene Einträge endgültig löschen (ohne --apply nur anzeigen)
    Purge {
        /// Wirklich löschen
        #[arg(long)]
        apply: bool,
    },
}

/// Befehle für Upload-Jobs.
#[derive(Debug, Subcommand)]
pub enum JobCommand {
    /// Jobs mit letztem und nächstem Lauf auflisten
    List,
    /// Einen Job jetzt ausführen (ohne --apply nur planen)
    Run {
        /// Kennung aus der Konfiguration
        job: String,
        /// Wirklich hochladen und aufräumen
        #[arg(long)]
        apply: bool,
    },
    /// Verlauf der letzten Läufe
    History {
        /// Kennung aus der Konfiguration
        job: String,
        /// Höchstens so viele Einträge
        #[arg(long, default_value_t = DEFAULT_HISTORY)]
        limit: u32,
    },
}

/// Standardanzahl der Einträge bei `jobs history`.
pub const DEFAULT_HISTORY: u32 = 10;

/// Konfigurations-Befehle.
#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Beispielkonfiguration ausgeben
    Example,
}

/// Fragt den Benutzer (im Test ersetzbar).
pub trait Prompter {
    /// Fragt nach einem sichtbaren Wert.
    ///
    /// # Errors
    /// Ein- oder Ausgabefehler.
    fn ask(&mut self, label: &str) -> io::Result<String>;
    /// Fragt nach einem Geheimnis (ohne Echo).
    ///
    /// # Errors
    /// Ein- oder Ausgabefehler.
    fn ask_secret(&mut self, label: &str) -> io::Result<String>;
}

/// Beispielkonfiguration für `config example`.
pub const EXAMPLE_CONFIG: &str = r#"# protonsync – Konfiguration (~/.config/protonsync/config.toml)

[general]
# Automatische Abgleiche nur planen, bis alles passt. Danach auf false setzen.
dry_run = true
# Tage, die Gelöschtes im Papierkorb bleibt.
trash_retention_days = 30
# Geheimnisse: "keyring" (Desktop) oder "file" (Server, Schlüssel über PROTONSYNC_KEY_FILE).
secrets = "keyring"
# Abstand der Ereignisabfrage bei Proton (mindestens 30 Sekunden).
event_poll_seconds = 60
# Vollständiger Abgleich als Sicherheitsnetz.
full_sync_minutes = 60

[[pair]]
id = "main"
local = "/home/BENUTZER/Proton"
# Ordner in „Meine Dateien“; leer für „Meine Dateien“ selbst.
remote = "protonsync"
# "two_way", "download_only" oder "upload_only"
direction = "two_way"

# Upload-Job (z. B. Home-Assistant-Backups); beliebig viele [[job]]-Abschnitte.
# [[job]]
# id = "home-assistant"
# source = "/mnt/ha-backups"      # lokaler Ordner oder eingehängtes SMB/NFS
# pattern = "*.tar"               # Dateimuster (nur Dateinamen), Standard "*"
# target = "Backups/HomeAssistant"
# schedule = "0 3 * * *"          # Cron (Ortszeit, TZ beachten) oder "on_new_file"
# keep_last = 14                  # optional: die 14 neuesten behalten
# max_age_days = 90               # optional: ältere in den Papierkorb
# verify = "hash"                 # "hash" (Standard) oder "size"
# max_attempts = 5                # Versuche bei Fehlern
# settle_seconds = 60             # so lange muss eine Datei unverändert sein

# Web-Oberfläche (vor allem für den Server), standardmäßig aus.
# [web]
# enabled = true
# bind = "127.0.0.1:8470"         # "0.0.0.0:8470" für das Heimnetz
# password_hash = "$argon2id$..." # mit `protonsyncd hash-password` erzeugen
# allowed_networks = ["192.168.178.0/24"]  # Standard: private Netze
# tls_cert = "/etc/protonsync/cert.pem"    # optional, nur zusammen mit tls_key
# tls_key = "/etc/protonsync/key.pem"
# session_hours = 12              # so lange gilt eine Anmeldung

# FUSE-Laufwerk (nur Desktop): Dateien erscheinen sofort, Inhalte laden bei Bedarf.
# [[mount]]
# id = "home"
# mountpoint = "/home/BENUTZER/ProtonDrive"
# remote = ""                     # Ordner in „Meine Dateien“; leer für alles
# keep_local = false              # Voreinstellung „Immer lokal halten“
#
# Zwischenspeicher aller Laufwerke.
# [cache]
# evict_after_days = 30           # unbenutzte Dateien danach freigeben
# max_size_gb = 10                # Größenlimit
"#;

/// Ein Fehler mit Exit-Code und Meldung.
struct Failure {
    code: u8,
    message: String,
}

/// Führt den Befehl aus und liefert den Exit-Code.
///
/// `connect` wird nur aufgerufen, wenn der Befehl den Daemon braucht.
///
/// Zeiten erscheinen in der Zeitzone `zone`.
pub fn run<T: Transport>(
    cli: &Cli,
    connect: impl FnOnce() -> Result<T, ClientError>,
    prompter: &mut dyn Prompter,
    out: &mut dyn Write,
    err: &mut dyn Write,
    lang: Lang,
    zone: &TimeZone,
) -> u8 {
    if let Command::Config {
        action: ConfigCommand::Example,
    } = cli.command
    {
        return match out.write_all(EXAMPLE_CONFIG.as_bytes()) {
            Ok(()) => exit::OK,
            Err(_) => exit::ERROR,
        };
    }
    let result = connect()
        .map_err(|e| client_failure(&e, lang))
        .and_then(|mut transport| execute(cli, &mut transport, prompter, out, lang, zone));
    match result {
        Ok(code) => code,
        Err(failure) => {
            let _ = writeln!(err, "{}", failure.message);
            failure.code
        }
    }
}

fn execute(
    cli: &Cli,
    transport: &mut dyn Transport,
    prompter: &mut dyn Prompter,
    out: &mut dyn Write,
    lang: Lang,
    zone: &TimeZone,
) -> Result<u8, Failure> {
    let response = match &cli.command {
        Command::Login { username, .. } => login(transport, prompter, username.clone(), lang)?,
        Command::Logout => send(transport, Request::Logout, lang)?,
        Command::Status => send(transport, Request::Status, lang)?,
        Command::Sync { pair, apply } => send(
            transport,
            Request::Sync {
                pair: pair.clone(),
                apply: *apply,
            },
            lang,
        )?,
        Command::Conflicts { action } => match action {
            None | Some(ConflictCommand::List) => send(transport, Request::Conflicts, lang)?,
            Some(ConflictCommand::Resolve { pair, id, choice }) => send(
                transport,
                Request::ResolveConflict {
                    pair: pair.clone(),
                    id: *id,
                    resolution: resolution(*choice),
                },
                lang,
            )?,
        },
        Command::Trash { action } => match action {
            None | Some(TrashCommand::List) => send(transport, Request::Trash, lang)?,
            Some(TrashCommand::Restore { pair, id }) => send(
                transport,
                Request::RestoreTrash {
                    pair: pair.clone(),
                    id: *id,
                },
                lang,
            )?,
            Some(TrashCommand::Purge { apply }) => {
                send(transport, Request::PurgeTrash { apply: *apply }, lang)?
            }
        },
        Command::Jobs { action } => match action {
            None | Some(JobCommand::List) => send(transport, Request::Jobs, lang)?,
            Some(JobCommand::Run { job, apply }) => send(
                transport,
                Request::RunJob {
                    job: job.clone(),
                    apply: *apply,
                },
                lang,
            )?,
            Some(JobCommand::History { job, limit }) => send(
                transport,
                Request::JobHistory {
                    job: job.clone(),
                    limit: *limit,
                },
                lang,
            )?,
        },
        Command::Mounts => send(transport, Request::Mounts, lang)?,
        Command::Pin { path } => send(transport, pin(path, PinChoice::Pinned)?, lang)?,
        Command::Unpin { path } => send(transport, pin(path, PinChoice::Unpinned)?, lang)?,
        Command::Free { path, apply } => send(
            transport,
            Request::Free {
                path: path.as_deref().map(absolute).transpose()?,
                apply: *apply,
            },
            lang,
        )?,
        Command::Config { .. } => Response::Done,
    };
    let code = success_code(&response);
    let text = if cli.json {
        serde_json::to_string_pretty(&response)
            .map(|json| json + "\n")
            .unwrap_or_default()
    } else {
        human(&cli.command, &response, lang, zone)
    };
    out.write_all(text.as_bytes()).map_err(|e| Failure {
        code: exit::ERROR,
        message: e.to_string(),
    })?;
    Ok(code)
}

/// Der Daemon kennt das Arbeitsverzeichnis der CLI nicht; deshalb absolut schicken.
fn absolute(path: &std::path::Path) -> Result<String, Failure> {
    let usage = |message: String| Failure {
        code: exit::USAGE,
        message,
    };
    let path = std::path::absolute(path).map_err(|e| usage(e.to_string()))?;
    path.to_str().map(str::to_owned).ok_or_else(|| {
        usage(format!(
            "Pfad ist kein UTF-8 / path is not UTF-8: {}",
            path.display()
        ))
    })
}

fn pin(path: &std::path::Path, state: PinChoice) -> Result<Request, Failure> {
    Ok(Request::Pin {
        path: absolute(path)?,
        state,
    })
}

fn resolution(choice: Choice) -> ResolutionChoice {
    match choice {
        Choice::KeepLocal => ResolutionChoice::KeepLocal,
        Choice::KeepRemote => ResolutionChoice::KeepRemote,
        Choice::KeepBoth => ResolutionChoice::KeepBoth,
    }
}

/// Schickt eine Anfrage; Fehlerantworten werden zu [`Failure`].
fn send(transport: &mut dyn Transport, request: Request, lang: Lang) -> Result<Response, Failure> {
    match transport.request(request) {
        Ok(Response::Error(error)) => Err(Failure {
            code: error_code(error.code),
            message: error_message(error.code, &error.message, lang),
        }),
        Ok(response) => Ok(response),
        Err(error) => Err(client_failure(&error, lang)),
    }
}

fn login(
    transport: &mut dyn Transport,
    prompter: &mut dyn Prompter,
    username: Option<String>,
    lang: Lang,
) -> Result<Response, Failure> {
    let io_failure = |e: io::Error| Failure {
        code: exit::ERROR,
        message: e.to_string(),
    };
    let username = match username {
        Some(name) => name,
        None => prompter
            .ask(lang.pick("Benutzername", "Username"))
            .map_err(io_failure)?,
    };
    let password = prompter
        .ask_secret(lang.pick("Passwort", "Password"))
        .map_err(io_failure)?;
    let mut step = expect_step(send(
        transport,
        Request::LoginStart {
            username,
            password: Password::new(password),
        },
        lang,
    )?)?;
    if step.needs_second_factor {
        let code = prompter
            .ask(lang.pick(
                "2FA-Code (Authenticator-App)",
                "2FA code (authenticator app)",
            ))
            .map_err(io_failure)?;
        step = expect_step(send(transport, Request::LoginSecondFactor { code }, lang)?)?;
    }
    let data_password = if step.needs_data_password {
        let label = lang.pick(
            "Datenpasswort (Mailbox-Passwort)",
            "Data password (mailbox password)",
        );
        Some(Password::new(
            prompter.ask_secret(label).map_err(io_failure)?,
        ))
    } else {
        None
    };
    send(transport, Request::LoginFinish { data_password }, lang)
}

fn expect_step(response: Response) -> Result<protonsync_api::LoginStep, Failure> {
    match response {
        Response::LoginStep(step) => Ok(step),
        other => Err(unexpected(&other)),
    }
}

fn unexpected(response: &Response) -> Failure {
    Failure {
        code: exit::ERROR,
        message: format!("Unerwartete Antwort / unexpected response: {response:?}"),
    }
}

/// Exit-Code einer erfolgreichen Antwort: Teilerfolge beim Abgleich und
/// gescheiterte Job-Läufe sind kein Erfolg.
fn success_code(response: &Response) -> u8 {
    if let Response::JobRun { run, .. } = response {
        return match run.outcome {
            JobOutcomeInfo::Failed { .. } => exit::PARTIAL,
            _ => exit::OK,
        };
    }
    let Response::Sync { results } = response else {
        return exit::OK;
    };
    let partial = results.iter().any(|result| match &result.outcome {
        SyncOutcomeInfo::Failed { .. } => true,
        SyncOutcomeInfo::Applied(summary) => !summary.failures.is_empty(),
        SyncOutcomeInfo::Planned { .. } => false,
    });
    if partial { exit::PARTIAL } else { exit::OK }
}

fn human(command: &Command, response: &Response, lang: Lang, zone: &TimeZone) -> String {
    match response {
        Response::Status(status) => render::status(lang, status),
        Response::LoggedIn { username } => {
            format!(
                "{} {username}\n",
                lang.pick("Angemeldet als", "Logged in as")
            )
        }
        Response::Sync { results } => render::sync_results(lang, results),
        Response::Conflicts { conflicts } => render::conflicts(lang, conflicts),
        Response::Trash { entries } => {
            let empty = if matches!(
                command,
                Command::Trash {
                    action: Some(TrashCommand::Purge { .. })
                }
            ) {
                lang.pick("Nichts abgelaufen.", "Nothing expired.")
            } else {
                lang.pick("Papierkorb ist leer.", "Trash is empty.")
            };
            render::trash(lang, entries, empty)
        }
        Response::Jobs { jobs } => render::jobs::jobs(lang, zone, jobs),
        Response::JobRun { run, details } => render::jobs::job_run(lang, zone, run, details),
        Response::JobHistory { runs } => render::jobs::history(lang, zone, runs),
        Response::Mounts { mounts } => render::mounts::mounts(lang, mounts),
        Response::Freed { results } => render::mounts::freed(lang, results),
        Response::Done => match command {
            Command::Logout => format!("{}\n", lang.pick("Abgemeldet.", "Logged out.")),
            Command::Pin { .. } => format!(
                "{}\n",
                lang.pick(
                    "Wird immer lokal gehalten; der Inhalt lädt im Hintergrund.",
                    "Always kept local; the content downloads in the background."
                )
            ),
            Command::Unpin { .. } => format!(
                "{}\n",
                lang.pick(
                    "Nur noch bei Bedarf; Speicher wird nach der Freigaberegel frei (sofort: `protonsync free PFAD --apply`).",
                    "On demand only; space is freed by the eviction rule (right away: `protonsync free PATH --apply`)."
                )
            ),
            _ => format!("{}\n", lang.pick("Erledigt.", "Done.")),
        },
        Response::LoginStep(_) | Response::Error(_) => format!("{response:?}\n"),
    }
}

fn error_code(code: ErrorCode) -> u8 {
    match code {
        ErrorCode::NotLoggedIn => exit::NOT_LOGGED_IN,
        ErrorCode::LoginRejected => exit::LOGIN_REJECTED,
        ErrorCode::VersionMismatch => exit::VERSION_MISMATCH,
        ErrorCode::InvalidRequest => exit::USAGE,
        ErrorCode::NotFound | ErrorCode::Unavailable | ErrorCode::Internal => exit::ERROR,
    }
}

fn error_message(code: ErrorCode, message: &str, lang: Lang) -> String {
    match code {
        ErrorCode::NotLoggedIn => format!(
            "{} ({message})",
            lang.pick(
                "Nicht angemeldet. Bitte `protonsync login` ausführen.",
                "Not logged in. Please run `protonsync login`."
            )
        ),
        ErrorCode::VersionMismatch => format!(
            "{} ({message})",
            lang.pick(
                "CLI und Daemon haben verschiedene Versionen; bitte beide aktualisieren.",
                "CLI and daemon versions differ; please update both."
            )
        ),
        _ => message.to_owned(),
    }
}

fn client_failure(error: &ClientError, lang: Lang) -> Failure {
    match error {
        ClientError::Unreachable { path, message } => Failure {
            code: exit::DAEMON_UNREACHABLE,
            message: format!(
                "{} {} ({message}).\n{}",
                lang.pick("Daemon nicht erreichbar unter", "Daemon not reachable at"),
                path.display(),
                lang.pick(
                    "Läuft protonsyncd? Desktop: systemctl --user start protonsyncd",
                    "Is protonsyncd running? Desktop: systemctl --user start protonsyncd",
                )
            ),
        },
        other => Failure {
            code: exit::ERROR,
            message: other.to_string(),
        },
    }
}
