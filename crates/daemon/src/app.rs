//! Wiring (composition root): the real adapters are created here and handed
//! to the core. Only this part knows all concrete types.

use std::path::PathBuf;
use std::sync::Arc;

use protonsync_adapter_fs::{FsContentCache, FsDiskSpace, FsLocal, LocalWatcher};
use protonsync_adapter_notify::{
    Channel, MailNotifier, MailSettings, WebhookNotifier, WebhookSettings,
};
use protonsync_adapter_proton::ProtonAccount;
use protonsync_adapter_secrets::{EncryptedFileStore, KeyError, MasterKey};
use protonsync_adapter_state::SqliteStateRepo;
use protonsync_application::ports::{
    Clock, LocalError, NotifyError, SecretError, SecretStore, StateError, SystemClock,
};
use protonsync_application::use_cases::WatchedFolder;
use protonsync_domain::job::JobTrigger;
use thiserror::Error;
use tokio::sync::watch;

#[cfg(feature = "keyring")]
use crate::blocking_secrets::BlockingSecrets;
use crate::config::{Config, ConfigError, Environment, SecretsBackend};
use crate::config_file::ConfigFile;
use crate::core::{
    Core, DaemonDrive, DiskWatchSetup, DriveMounter, JobSetup, MountSetup, PairSetup, PhotoSetup,
    Settings,
};
use crate::local_time::LocalZone;
use crate::notify::{Dispatcher, Notifications, NotifyLoop, Route};
use crate::notify_config::{NotifyConfig, ProcessEnvironment};
use crate::{notify, scheduler, server, web};

#[cfg(unix)]
const DATA_DIR_MODE: u32 = 0o700;
/// Names of the photo folder and the data directory in disk space warnings.
const PHOTOS_FOLDER_NAME: &str = "photos";
const DATA_FOLDER_NAME: &str = "data";

/// Error during startup.
#[derive(Debug, Error)]
pub enum StartupError {
    /// Faulty configuration.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// Data directory or socket not usable.
    #[error("{0}")]
    Io(String),
    /// State database not usable.
    #[error(transparent)]
    State(#[from] StateError),
    /// Local folder of a sync pair not usable.
    #[error(transparent)]
    Local(#[from] LocalError),
    /// Key for the secrets file is missing.
    #[error(transparent)]
    Key(#[from] KeyError),
    /// Secret store not usable.
    #[error(transparent)]
    Secrets(#[from] SecretError),
    /// A notification channel cannot be set up (address, URL, missing password).
    #[error("[notify]: {0}")]
    Notify(String),
    /// Secret Service chosen but not built in.
    #[error("secrets = \"keyring\" is not available in this build; use secrets = \"file\"")]
    KeyringNotBuilt,
}

/// Startup options from the command line.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Configuration file instead of the default path.
    pub config: Option<PathBuf>,
    /// Socket instead of the default path.
    pub socket: Option<PathBuf>,
}

type Account = ProtonAccount<dyn SecretStore>;
type DaemonCore = Core<Account, FsLocal, SqliteStateRepo, SystemClock>;
type Remote = <Account as protonsync_application::ports::Account>::Remote;
type Mounter = dyn DriveMounter<DaemonDrive<Remote, SqliteStateRepo, SystemClock>>;

/// Starts the daemon and runs until SIGINT/SIGTERM.
///
/// # Errors
/// [`StartupError`] if startup fails.
pub async fn run(options: Options) -> Result<(), StartupError> {
    let mut env = Environment::from_process();
    if options.config.is_some() {
        env.config_file.clone_from(&options.config);
    }
    let config = Config::load(&env)?;
    tracing::info!(
        config = %env.config_path().display(),
        pairs = config.pairs.len(),
        jobs = config.jobs.len(),
        mounts = config.mounts.len(),
        photos = config.photos.is_some(),
        dry_run = config.dry_run,
        "starting"
    );
    create_private_dir(&config.data_dir)?;

    let zone = LocalZone::system();
    let core = Arc::new(
        with_disk_check(build_core(&config, zone.clone())?, &config)
            .with_settings_file(Arc::new(ConfigFile::new(env.clone()))),
    );
    let channels = config.notify.as_ref().map(build_channels).transpose()?;
    // Subscribe before the first sync so that no conflict slips through.
    let notify_events = channels.as_ref().map(|_| core.subscribe());
    core.startup().await;

    let socket = options
        .socket
        .unwrap_or_else(protonsync_api::wire::default_socket_path);
    let listener = server::bind(&socket)
        .await
        .map_err(|e| StartupError::Io(format!("Socket {}: {e}", socket.display())))?;
    tracing::info!(socket = %socket.display(), "API ready");

    let watchers = start_watchers(&config, &core);
    let (stop, shutdown) = watch::channel(false);
    let serving = tokio::spawn(server::serve(listener, Arc::clone(&core), shutdown.clone()));
    let web = config
        .web
        .as_ref()
        .map(|settings| start_web(web::web_config(settings, &zone), &core, shutdown.clone()));
    let notifying = match (&config.notify, channels, notify_events) {
        (Some(settings), Some(dispatcher), Some(events)) => Some(tokio::spawn(notify::run(
            NotifyLoop {
                backend: Arc::clone(&core),
                clock: Arc::new(SystemClock),
                zone: zone.clone(),
                report_schedule: settings.report_schedule.clone(),
                daily_changes_schedule: settings.daily_changes_schedule.clone(),
                notifications: Notifications::new(settings.lang, zone.clone(), SystemClock.now()),
            },
            events,
            Arc::new(dispatcher),
            shutdown.clone(),
        ))),
        _ => None,
    };
    let scheduling = tokio::spawn(scheduler::run(Arc::clone(&core), config.timing, shutdown));

    wait_for_shutdown_signal().await;
    tracing::info!("shutting down");
    let _ = stop.send(true);
    let _ = serving.await;
    let _ = scheduling.await;
    if let Some(notifying) = notifying {
        let _ = notifying.await;
    }
    if let Some(web) = web {
        let _ = web.await;
    }
    core.disconnect_mounts().await;
    drop(watchers);
    let _ = std::fs::remove_file(&socket);
    Ok(())
}

fn build_core(config: &Config, zone: LocalZone) -> Result<DaemonCore, StartupError> {
    let state = SqliteStateRepo::open(&config.state_path())?;
    let account = ProtonAccount::new(secret_store(config)?);
    let mut pairs = Vec::with_capacity(config.pairs.len());
    for pair in &config.pairs {
        std::fs::create_dir_all(&pair.local)
            .map_err(|e| StartupError::Io(format!("{}: {e}", pair.local.display())))?;
        pairs.push(PairSetup {
            config: pair.clone(),
            local: FsLocal::new(&pair.local)?,
        });
    }
    let settings = Settings {
        dry_run: config.dry_run,
        retention: config.retention,
    };
    // Job source folders may be missing at startup (network drive not mounted
    // yet); the run then reports this as an error.
    let jobs = config
        .jobs
        .iter()
        .map(|job| JobSetup {
            source: FsLocal::new_unchecked(&job.source),
            job: job.clone(),
        })
        .collect();
    if !config.jobs.is_empty() {
        tracing::info!(zone = zone.name(), "time zone for job schedules");
    }
    let mounts = config
        .mounts
        .iter()
        .map(|mount| {
            Ok(MountSetup {
                config: mount.clone(),
                cache: FsContentCache::open(config.cache_dir(mount))?,
            })
        })
        .collect::<Result<Vec<_>, StartupError>>()?;
    let mounter = if config.mounts.is_empty() {
        None
    } else {
        mounter()
    };
    let core = Core::new(account, state, SystemClock, settings, pairs)
        .with_change_log(config.change_log_days)
        .with_jobs(jobs, zone)
        .with_mounts(mounts, config.cache.policy, mounter);
    let Some(photos) = &config.photos else {
        return Ok(core);
    };
    std::fs::create_dir_all(&photos.folder)
        .map_err(|e| StartupError::Io(format!("{}: {e}", photos.folder.display())))?;
    Ok(core.with_photos(PhotoSetup {
        local: FsLocal::new(&photos.folder)?,
        config: photos.clone(),
    }))
}

/// Creates the configured channels; secrets are read here, once.
/// Watches the free space where the daemon writes: the sync folders, the
/// photo folder and the data directory (state, drive caches).
fn with_disk_check(core: DaemonCore, config: &Config) -> DaemonCore {
    let Some(every) = config.disk.every else {
        return core;
    };
    let mut folders: Vec<WatchedFolder> = config
        .pairs
        .iter()
        .map(|pair| WatchedFolder {
            name: pair.id.clone(),
            path: pair.local.clone(),
        })
        .collect();
    if let Some(photos) = &config.photos {
        folders.push(WatchedFolder {
            name: PHOTOS_FOLDER_NAME.into(),
            path: photos.folder.clone(),
        });
    }
    folders.push(WatchedFolder {
        name: DATA_FOLDER_NAME.into(),
        path: config.data_dir.clone(),
    });
    core.with_disk_check(DiskWatchSetup {
        probe: Arc::new(FsDiskSpace),
        thresholds: config.disk.thresholds,
        folders,
        every,
    })
}

fn build_channels(config: &NotifyConfig) -> Result<Dispatcher<Channel>, StartupError> {
    let notify_error = |e: NotifyError| StartupError::Notify(e.to_string());
    let mut routes = Vec::new();
    if let Some(mail) = &config.mail {
        let login = match &mail.username {
            Some(user) => {
                let password = mail.password.read(&ProcessEnvironment)?.ok_or_else(|| {
                    StartupError::Notify(format!(
                        "mail: no password for {user} (systemd credential {}, password_file or {})",
                        mail.password.credential, mail.password.env
                    ))
                })?;
                Some((user.clone(), password.to_string()))
            }
            None => None,
        };
        let settings = MailSettings {
            host: mail.host.clone(),
            port: mail.port,
            security: mail.security,
            login,
            from: mail.from.clone(),
            to: mail.to.clone(),
        };
        routes.push(Route {
            notifier: Channel::Mail(MailNotifier::new(&settings).map_err(notify_error)?),
            topics: mail.topics,
        });
    }
    if let Some(webhook) = &config.webhook {
        let settings = WebhookSettings {
            url: webhook.url.clone(),
            format: webhook.format,
            token: webhook
                .token
                .read(&ProcessEnvironment)?
                .map(|token| token.to_string()),
        };
        routes.push(Route {
            notifier: Channel::Webhook(WebhookNotifier::new(settings).map_err(notify_error)?),
            topics: webhook.topics,
        });
    }
    tracing::info!(channels = routes.len(), "notifications enabled");
    Ok(Dispatcher::new(routes))
}

#[cfg(all(feature = "fuse", target_os = "linux"))]
#[allow(clippy::unnecessary_wraps)] // Without FUSE the other version returns `None`.
fn mounter() -> Option<Arc<Mounter>> {
    Some(Arc::new(crate::fuse_mount::FuseMounter))
}

#[cfg(not(all(feature = "fuse", target_os = "linux")))]
fn mounter() -> Option<Arc<Mounter>> {
    tracing::warn!("this build cannot mount drives (no FUSE); [[mount]] is only connected");
    None
}

/// Starts the web interface. If it fails (port in use, TLS files unusable),
/// the daemon keeps running without it; the error is in the log.
fn start_web(
    config: protonsync_web::WebConfig,
    core: &Arc<DaemonCore>,
    shutdown: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    let core = Arc::clone(core);
    tokio::spawn(async move {
        if let Err(error) = protonsync_web::serve(config, core, shutdown).await {
            tracing::error!(%error, "web interface not started");
        }
    })
}

fn secret_store(config: &Config) -> Result<Arc<dyn SecretStore>, StartupError> {
    match config.secrets {
        SecretsBackend::File => {
            let key = MasterKey::from_environment()?;
            Ok(Arc::new(EncryptedFileStore::open(
                config.secrets_dir(),
                &key,
            )?))
        }
        #[cfg(feature = "keyring")]
        SecretsBackend::Keyring => Ok(Arc::new(BlockingSecrets::new(
            protonsync_adapter_secrets::KeyringStore::new(),
        ))),
        #[cfg(not(feature = "keyring"))]
        SecretsBackend::Keyring => Err(StartupError::KeyringNotBuilt),
    }
}

/// Local changes nudge the schedule. Without a watcher (e.g. inotify limit
/// reached) the periodic sync remains; this is reported.
fn start_watchers(config: &Config, core: &Arc<DaemonCore>) -> Vec<LocalWatcher> {
    let mut watchers = Vec::new();
    for (index, pair) in config.pairs.iter().enumerate() {
        let core = Arc::clone(core);
        let watcher = LocalWatcher::start(&pair.local, move || {
            if let Some(trigger) = core.trigger(index) {
                trigger.notify_one();
            }
        });
        match watcher {
            Ok(watcher) => watchers.push(watcher),
            Err(error) => {
                tracing::warn!(pair = %pair.id, %error, "no local change detection");
            }
        }
    }
    for (index, job) in config.jobs.iter().enumerate() {
        if job.trigger != JobTrigger::OnNewFile {
            continue;
        }
        let core = Arc::clone(core);
        let watcher = LocalWatcher::start(&job.source, move || {
            if let Some(trigger) = core.job_trigger(index) {
                trigger.notify_one();
            }
        });
        match watcher {
            Ok(watcher) => watchers.push(watcher),
            // Without a watcher the schedule checks every five minutes.
            Err(error) => {
                tracing::info!(job = job.id.as_str(), %error, "source folder without change detection");
            }
        }
    }
    if let Some(photos) = &config.photos {
        let core = Arc::clone(core);
        let watcher = LocalWatcher::start(&photos.folder, move || {
            if let Some(trigger) = core.photo_trigger() {
                trigger.notify_one();
            }
        });
        match watcher {
            Ok(watcher) => watchers.push(watcher),
            Err(error) => tracing::warn!(%error, "photo folder without change detection"),
        }
    }
    watchers
}

fn create_private_dir(dir: &std::path::Path) -> Result<(), StartupError> {
    let io = |e: std::io::Error| StartupError::Io(format!("{}: {e}", dir.display()));
    std::fs::create_dir_all(dir).map_err(io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(DATA_DIR_MODE))
            .map_err(io)?;
    }
    Ok(())
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut terminate) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
