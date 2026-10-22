//! Command-line commands and their execution against the daemon.

use std::io::{self, Write};
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use jiff::tz::TimeZone;
use protonsync_api::{
    ErrorCode, JobOutcomeInfo, Password, PhotoOutcomeInfo, PinChoice, Request, ResolutionChoice,
    Response, SyncOutcomeInfo,
};

use crate::client::{ClientError, Transport};
use crate::i18n::Lang;
use crate::render;

/// Exit codes (also meant for scripts, hence stable).
pub mod exit {
    /// Success.
    pub const OK: u8 = 0;
    /// General error.
    pub const ERROR: u8 = 1;
    /// Wrong usage (arguments).
    pub const USAGE: u8 = 2;
    /// Daemon not reachable.
    pub const DAEMON_UNREACHABLE: u8 = 3;
    /// Not logged in.
    pub const NOT_LOGGED_IN: u8 = 4;
    /// Login rejected.
    pub const LOGIN_REJECTED: u8 = 5;
    /// CLI and daemon do not match.
    pub const VERSION_MISMATCH: u8 = 6;
    /// Sync ran, but not everything worked.
    pub const PARTIAL: u8 = 7;
}

/// Unofficial Proton Drive client for Linux (not supported by Proton).
#[derive(Debug, Parser)]
#[command(name = "protonsync", version)]
pub struct Cli {
    /// Output as JSON (for scripts)
    #[arg(long, global = true)]
    pub json: bool,
    /// Daemon socket (default: `$PROTONSYNC_SOCKET` or `$XDG_RUNTIME_DIR/protonsync/daemon.sock`)
    #[arg(long, global = true)]
    pub socket: Option<PathBuf>,
    /// Command
    #[command(subcommand)]
    pub command: Command,
}

/// Commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Log in to Proton (password, 2FA, data password if needed)
    Login {
        /// Username; asked for otherwise
        #[arg(long)]
        username: Option<String>,
        /// Read passwords line by line from standard input instead of the terminal
        #[arg(long)]
        password_stdin: bool,
    },
    /// Log out and delete the stored session
    Logout,
    /// State of the account and sync pairs
    Status,
    /// Sync now (without --apply only plan)
    Sync {
        /// Only this sync pair
        #[arg(long)]
        pair: Option<String>,
        /// Actually apply the changes
        #[arg(long)]
        apply: bool,
    },
    /// Show and resolve conflicts
    Conflicts {
        /// Action (default: list)
        #[command(subcommand)]
        action: Option<ConflictCommand>,
    },
    /// Show, restore and empty the trash
    Trash {
        /// Action (default: list)
        #[command(subcommand)]
        action: Option<TrashCommand>,
    },
    /// Show and run upload jobs, history
    Jobs {
        /// Action (default: list)
        #[command(subcommand)]
        action: Option<JobCommand>,
    },
    /// Show FUSE drives with cache and rules
    Mounts,
    /// "Always keep local": fully download files or folders in a drive
    Pin {
        /// Paths in the drive (e.g. ~/ProtonDrive/Photos)
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },
    /// "On demand only": files or folders may be freed again
    Unpin {
        /// Paths in the drive
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },
    /// State of files and folders (drive, sync or photo folder), e.g. for file managers
    FileStatus {
        /// Paths
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },
    /// Free up space (without --apply only plan). With a path, everything below it
    /// right away, otherwise by the eviction rule (`[cache]`)
    Free {
        /// Path in the drive; without: all drives by the eviction rule
        path: Option<PathBuf>,
        /// Actually free space
        #[arg(long)]
        apply: bool,
    },
    /// Photos (Proton Photos): state, sync, duplicates
    Photos {
        /// Action (default: status)
        #[command(subcommand)]
        action: Option<PhotoCommand>,
    },
    /// Configuration
    Config {
        /// Action
        #[command(subcommand)]
        action: ConfigCommand,
    },
}

/// Conflict commands.
#[derive(Debug, Subcommand)]
pub enum ConflictCommand {
    /// List open conflicts
    List,
    /// Resolve a conflict
    Resolve {
        /// Sync-Paar
        pair: String,
        /// ID from `conflicts list`
        id: i64,
        /// Decision
        choice: Choice,
    },
}

/// Decision on a conflict.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Choice {
    /// Keep the local version, the remote one goes to the trash
    KeepLocal,
    /// Keep the remote version, the local one goes to the trash
    KeepRemote,
    /// Keep both (the conflict copy stays)
    KeepBoth,
}

/// Trash commands.
#[derive(Debug, Subcommand)]
pub enum TrashCommand {
    /// List entries
    List,
    /// Restore an entry
    Restore {
        /// Sync-Paar
        pair: String,
        /// ID from `trash list`
        id: i64,
    },
    /// Permanently delete expired entries (without --apply only show them)
    Purge {
        /// Actually delete
        #[arg(long)]
        apply: bool,
    },
}

/// Commands for upload jobs.
#[derive(Debug, Subcommand)]
pub enum JobCommand {
    /// List jobs with their last and next run
    List,
    /// Run a job now (without --apply only plan)
    Run {
        /// ID from the configuration
        job: String,
        /// Actually upload and clean up
        #[arg(long)]
        apply: bool,
    },
    /// History of the latest runs
    History {
        /// ID from the configuration
        job: String,
        /// At most this many entries
        #[arg(long, default_value_t = DEFAULT_HISTORY)]
        limit: u32,
    },
}

/// Commands for photos.
#[derive(Debug, Subcommand)]
pub enum PhotoCommand {
    /// Photo folder, number of photos, last sync
    Status,
    /// Sync now (without --apply only plan)
    Sync {
        /// Actually apply the changes
        #[arg(long)]
        apply: bool,
    },
    /// Find duplicates (suggestions only) and delete selected ones
    Duplicates {
        /// Action (default: list)
        #[command(subcommand)]
        action: Option<DuplicateCommand>,
    },
}

/// Commands for duplicates.
#[derive(Debug, Subcommand)]
pub enum DuplicateCommand {
    /// List groups with a suggestion of which photo to keep
    List,
    /// Move selected photos to the trash (without --apply only check);
    /// at least one photo of each group remains
    Trash {
        /// IDs from `photos duplicates`
        #[arg(required = true)]
        ids: Vec<String>,
        /// Actually move to the trash
        #[arg(long)]
        apply: bool,
    },
}

/// Default number of entries for `jobs history`.
pub const DEFAULT_HISTORY: u32 = 10;

/// Configuration commands.
#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print an example configuration
    Example,
}

/// Asks the user (replaceable in tests).
pub trait Prompter {
    /// Asks for a visible value.
    ///
    /// # Errors
    /// Input or output error.
    fn ask(&mut self, label: &str) -> io::Result<String>;
    /// Asks for a secret (without echo).
    ///
    /// # Errors
    /// Input or output error.
    fn ask_secret(&mut self, label: &str) -> io::Result<String>;
}

/// Example configuration for `config example` (German comments).
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
# true: lokale Löschungen und Ersetzungen gehen nicht nach Proton (Löschschutz);
# eine lokale Datei, die eine vorhandene in Proton ersetzen würde, wird ein Konflikt.
protect_remote = false

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

# Fotos (Proton Photos) in einen eigenen Ordner, sortiert nach JJJJ/MM.
# [photos]
# folder = "/home/BENUTZER/Bilder/Proton"
# direction = "download_only"     # Standard; oder "two_way", "upload_only"
# protect_remote = false          # true: lokal gelöschte Fotos bleiben in Proton

# Benachrichtigungen per E-Mail und/oder Webhook (Handbuch 4.9); jedes Thema einzeln schaltbar.
# [notify]
# language = "de"
# weekly_report_schedule = "0 8 * * 1"   # Wochenbericht montags 8:00
# [notify.mail]
# host = "smtp.example.org"
# username = "ich@example.org"           # Passwort: password_file oder PROTONSYNC_SMTP_PASSWORD
# password_file = "/etc/protonsync/smtp-password"
# from = "protonsync <ich@example.org>"
# to = ["ich@example.org"]
# conflicts = true
# sync_failures = true
# job_failures = true
# login_required = true
# weekly_report = true
# [notify.webhook]
# url = "https://ntfy.sh/mein-thema"
# format = "text"                        # "text" (ntfy) oder "json"
"#;

/// English version of [`EXAMPLE_CONFIG`]; same keys and values, only the comments differ.
pub const EXAMPLE_CONFIG_EN: &str = r#"# protonsync – configuration (~/.config/protonsync/config.toml)

[general]
# Only plan automatic syncs until everything looks right. Then set to false.
dry_run = true
# Days that deleted items stay in the trash.
trash_retention_days = 30
# Secrets: "keyring" (desktop) or "file" (server, key via PROTONSYNC_KEY_FILE).
secrets = "keyring"
# Interval of the Proton event query (at least 30 seconds).
event_poll_seconds = 60
# Full sync as a safety net.
full_sync_minutes = 60

[[pair]]
id = "main"
local = "/home/USER/Proton"
# Folder in "My files"; empty for "My files" itself.
remote = "protonsync"
# "two_way", "download_only" or "upload_only"
direction = "two_way"
# true: local deletions and replacements are not passed on to Proton (protection);
# a local file that would replace an existing one in Proton becomes a conflict.
protect_remote = false

# Upload job (e.g. Home Assistant backups); any number of [[job]] sections.
# [[job]]
# id = "home-assistant"
# source = "/mnt/ha-backups"      # local folder or mounted SMB/NFS
# pattern = "*.tar"               # file pattern (file names only), default "*"
# target = "Backups/HomeAssistant"
# schedule = "0 3 * * *"          # cron (local time, mind TZ) or "on_new_file"
# keep_last = 14                  # optional: keep the 14 newest
# max_age_days = 90               # optional: older ones go to the trash
# verify = "hash"                 # "hash" (default) or "size"
# max_attempts = 5                # attempts on errors
# settle_seconds = 60             # a file must stay unchanged this long

# Web interface (mainly for the server), off by default.
# [web]
# enabled = true
# bind = "127.0.0.1:8470"         # "0.0.0.0:8470" for the home network
# password_hash = "$argon2id$..." # create with `protonsyncd hash-password`
# allowed_networks = ["192.168.178.0/24"]  # default: private networks
# tls_cert = "/etc/protonsync/cert.pem"    # optional, only together with tls_key
# tls_key = "/etc/protonsync/key.pem"
# session_hours = 12              # how long a login stays valid

# FUSE drive (desktop only): files appear at once, contents load on demand.
# [[mount]]
# id = "home"
# mountpoint = "/home/USER/ProtonDrive"
# remote = ""                     # folder in "My files"; empty for everything
# keep_local = false              # default for "Always keep local"
#
# Cache of all drives.
# [cache]
# evict_after_days = 30           # free unused files after this
# max_size_gb = 10                # size limit

# Photos (Proton Photos) into a folder of their own, sorted by YYYY/MM.
# [photos]
# folder = "/home/USER/Pictures/Proton"
# direction = "download_only"     # default; or "two_way", "upload_only"
# protect_remote = false          # true: photos deleted locally stay in Proton

# Notifications by email and/or webhook (manual 4.9); every topic can be switched on its own.
# [notify]
# language = "en"
# weekly_report_schedule = "0 8 * * 1"   # weekly report on Mondays 8:00
# [notify.mail]
# host = "smtp.example.org"
# username = "me@example.org"            # password: password_file or PROTONSYNC_SMTP_PASSWORD
# password_file = "/etc/protonsync/smtp-password"
# from = "protonsync <me@example.org>"
# to = ["me@example.org"]
# conflicts = true
# sync_failures = true
# job_failures = true
# login_required = true
# weekly_report = true
# [notify.webhook]
# url = "https://ntfy.sh/my-topic"
# format = "text"                        # "text" (ntfy) or "json"
"#;

/// An error with exit code and message.
struct Failure {
    code: u8,
    message: String,
}

/// Runs the command and returns the exit code.
///
/// `connect` is only called when the command needs the daemon.
///
/// Times are shown in the time zone `zone`.
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
        let example = lang.pick(EXAMPLE_CONFIG, EXAMPLE_CONFIG_EN);
        return match out.write_all(example.as_bytes()) {
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
        Command::Pin { paths } => pin_all(transport, paths, PinChoice::Pinned, lang)?,
        Command::Unpin { paths } => pin_all(transport, paths, PinChoice::Unpinned, lang)?,
        Command::FileStatus { paths } => send(
            transport,
            Request::PathStatus {
                paths: paths
                    .iter()
                    .map(|path| absolute(path))
                    .collect::<Result<_, _>>()?,
            },
            lang,
        )?,
        Command::Free { path, apply } => send(
            transport,
            Request::Free {
                path: path.as_deref().map(absolute).transpose()?,
                apply: *apply,
            },
            lang,
        )?,
        Command::Photos { action } => send(transport, photo_request(action.as_ref()), lang)?,
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

fn photo_request(action: Option<&PhotoCommand>) -> Request {
    match action {
        None | Some(PhotoCommand::Status) => Request::Photos,
        Some(PhotoCommand::Sync { apply }) => Request::SyncPhotos { apply: *apply },
        Some(PhotoCommand::Duplicates { action }) => match action {
            None | Some(DuplicateCommand::List) => Request::PhotoDuplicates,
            Some(DuplicateCommand::Trash { ids, apply }) => Request::TrashPhotoDuplicates {
                ids: ids.clone(),
                apply: *apply,
            },
        },
    }
}

/// The daemon does not know the CLI's working directory, so send absolute paths.
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

/// Sets the rule for every path; stops at the first error.
fn pin_all(
    transport: &mut dyn Transport,
    paths: &[PathBuf],
    state: PinChoice,
    lang: Lang,
) -> Result<Response, Failure> {
    for path in paths {
        let request = Request::Pin {
            path: absolute(path)?,
            state,
        };
        send(transport, request, lang)?;
    }
    Ok(Response::Done)
}

fn resolution(choice: Choice) -> ResolutionChoice {
    match choice {
        Choice::KeepLocal => ResolutionChoice::KeepLocal,
        Choice::KeepRemote => ResolutionChoice::KeepRemote,
        Choice::KeepBoth => ResolutionChoice::KeepBoth,
    }
}

/// Sends a request; error responses become a [`Failure`].
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

/// Exit code of a successful response: partial sync successes and
/// failed job runs are not a success.
fn success_code(response: &Response) -> u8 {
    if let Response::JobRun { run, .. } = response {
        return match run.outcome {
            JobOutcomeInfo::Failed { .. } => exit::PARTIAL,
            _ => exit::OK,
        };
    }
    if let Response::PhotoRun(run) = response {
        return match &run.outcome {
            PhotoOutcomeInfo::Failed { .. } => exit::PARTIAL,
            PhotoOutcomeInfo::Applied(counts) if !counts.failures.is_empty() => exit::PARTIAL,
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
        Response::Photos(status) => render::photos::status(lang, status),
        Response::PathStatus { entries } => render::files::path_status(lang, entries),
        Response::PhotoRun(run) => render::photos::run(lang, run),
        Response::PhotoDuplicates { groups } => render::photos::duplicates(lang, groups),
        Response::PhotosTrashed { dry_run, photos } => {
            render::photos::trashed(lang, *dry_run, photos)
        }
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
