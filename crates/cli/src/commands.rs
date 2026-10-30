//! Command-line commands and their execution against the daemon.

use std::io::{self, Write};
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use jiff::tz::TimeZone;
use protonsync_api::{
    Direction, ErrorCode, JobOutcomeInfo, PairSettingsChange, Password, PhotoOutcomeInfo,
    PinChoice, Request, ResolutionChoice, Response, SettingsChange, SyncOutcomeInfo,
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
        /// Sync pair
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
        /// Sync pair
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
    /// Print the example configuration (all settings with defaults, comments de/en)
    Example,
    /// Show the settings that can be changed without editing the file
    Show,
    /// Change settings; the daemon saves them in config.toml and applies them at once
    #[command(arg_required_else_help = true)]
    Set {
        /// Only plan automatic syncs (true) or really apply them (false)
        #[arg(long, value_name = "BOOL")]
        dry_run: Option<bool>,
        /// Days after which trash entries expire
        #[arg(long, value_name = "DAYS")]
        trash_retention_days: Option<u32>,
        /// Sync pair for the following options
        #[arg(long)]
        pair: Option<String>,
        /// Direction of the sync pair
        #[arg(long, requires = "pair")]
        direction: Option<DirectionArg>,
        /// Never delete or replace anything in Proton (sync pair)
        #[arg(long, value_name = "BOOL", requires = "pair")]
        protect_remote: Option<bool>,
        /// Archive deleted and replaced versions in `_archiv` (sync pair)
        #[arg(long, value_name = "BOOL", requires = "pair")]
        archive_versions: Option<bool>,
    },
}

/// Direction of a sync pair.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DirectionArg {
    /// Both ways
    TwoWay,
    /// Only from Proton to local
    DownloadOnly,
    /// Only from local to Proton
    UploadOnly,
    /// Keep everything from Proton, never upload or delete, archive old versions
    BackupOnly,
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

/// Example configuration for `config example`: every setting with its default
/// and bilingual comments. The file lives in the daemon crate, whose tests keep
/// it in step with the configuration parser.
pub const EXAMPLE_CONFIG: &str = include_str!("../../daemon/config.example.toml");

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
        Command::Config { action } => send(transport, config_request(action)?, lang)?,
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

/// `config show` and `config set` (`config example` never reaches the daemon).
fn config_request(action: &ConfigCommand) -> Result<Request, Failure> {
    let ConfigCommand::Set {
        dry_run,
        trash_retention_days,
        pair,
        direction,
        protect_remote,
        archive_versions,
    } = action
    else {
        return Ok(Request::Settings);
    };
    let mut change = SettingsChange {
        dry_run: *dry_run,
        trash_retention_days: *trash_retention_days,
        pairs: Vec::new(),
    };
    if let Some(pair) = pair {
        change.pairs.push(PairSettingsChange {
            direction: direction.map(api_direction),
            protect_remote: *protect_remote,
            archive_versions: *archive_versions,
            ..PairSettingsChange::new(pair.clone())
        });
    }
    if change.is_empty() {
        return Err(Failure {
            code: exit::USAGE,
            message: "Nichts zu ändern / nothing to change (`protonsync config set --help`)".into(),
        });
    }
    Ok(Request::ChangeSettings { change })
}

fn api_direction(direction: DirectionArg) -> Direction {
    match direction {
        DirectionArg::TwoWay => Direction::TwoWay,
        DirectionArg::DownloadOnly => Direction::DownloadOnly,
        DirectionArg::UploadOnly => Direction::UploadOnly,
        DirectionArg::BackupOnly => Direction::BackupOnly,
    }
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
        Response::Settings(settings) => render::settings(lang, settings),
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
