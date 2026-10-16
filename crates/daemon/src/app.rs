//! Zusammenbau (Composition Root): Hier werden die echten Adapter erzeugt und
//! an den Kern übergeben. Nur dieser Teil kennt alle konkreten Typen.

use std::path::PathBuf;
use std::sync::Arc;

use protonsync_adapter_fs::{FsContentCache, FsLocal, LocalWatcher};
use protonsync_adapter_proton::ProtonAccount;
use protonsync_adapter_secrets::{EncryptedFileStore, KeyError, MasterKey};
use protonsync_adapter_state::SqliteStateRepo;
use protonsync_application::ports::{
    LocalError, SecretError, SecretStore, StateError, SystemClock,
};
use protonsync_domain::job::JobTrigger;
use thiserror::Error;
use tokio::sync::watch;

use crate::config::{Config, ConfigError, Environment, SecretsBackend};
use crate::core::{Core, DaemonDrive, DriveMounter, JobSetup, MountSetup, PairSetup, Settings};
use crate::local_time::LocalZone;
use crate::{scheduler, server, web};

#[cfg(unix)]
const DATA_DIR_MODE: u32 = 0o700;

/// Fehler beim Start.
#[derive(Debug, Error)]
pub enum StartupError {
    /// Konfiguration fehlerhaft.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// Datenverzeichnis oder Socket nicht nutzbar.
    #[error("{0}")]
    Io(String),
    /// Zustandsdatenbank nicht nutzbar.
    #[error(transparent)]
    State(#[from] StateError),
    /// Lokaler Ordner eines Sync-Paares nicht nutzbar.
    #[error(transparent)]
    Local(#[from] LocalError),
    /// Schlüssel für die Geheimnisdatei fehlt.
    #[error(transparent)]
    Key(#[from] KeyError),
    /// Geheimnisspeicher nicht nutzbar.
    #[error(transparent)]
    Secrets(#[from] SecretError),
    /// Secret Service gewählt, aber nicht eingebaut.
    #[error(
        "secrets = \"keyring\" ist in diesem Build nicht verfügbar; secrets = \"file\" verwenden"
    )]
    KeyringNotBuilt,
}

/// Startoptionen von der Kommandozeile.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Konfigurationsdatei statt des Standardpfads.
    pub config: Option<PathBuf>,
    /// Socket statt des Standardpfads.
    pub socket: Option<PathBuf>,
}

type Account = ProtonAccount<dyn SecretStore>;
type DaemonCore = Core<Account, FsLocal, SqliteStateRepo, SystemClock>;
type Remote = <Account as protonsync_application::ports::Account>::Remote;
type Mounter = dyn DriveMounter<DaemonDrive<Remote, SqliteStateRepo, SystemClock>>;

/// Startet den Daemon und läuft bis SIGINT/SIGTERM.
///
/// # Errors
/// [`StartupError`], wenn der Start scheitert.
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
        dry_run = config.dry_run,
        "Start"
    );
    create_private_dir(&config.data_dir)?;

    let zone = LocalZone::system();
    let core = Arc::new(build_core(&config, zone.clone())?);
    core.startup().await;

    let socket = options
        .socket
        .unwrap_or_else(protonsync_api::wire::default_socket_path);
    let listener = server::bind(&socket)
        .await
        .map_err(|e| StartupError::Io(format!("Socket {}: {e}", socket.display())))?;
    tracing::info!(socket = %socket.display(), "API bereit");

    let watchers = start_watchers(&config, &core);
    let (stop, shutdown) = watch::channel(false);
    let serving = tokio::spawn(server::serve(listener, Arc::clone(&core), shutdown.clone()));
    let web = config
        .web
        .as_ref()
        .map(|settings| start_web(web::web_config(settings, &zone), &core, shutdown.clone()));
    let scheduling = tokio::spawn(scheduler::run(Arc::clone(&core), config.timing, shutdown));

    wait_for_shutdown_signal().await;
    tracing::info!("Beende");
    let _ = stop.send(true);
    let _ = serving.await;
    let _ = scheduling.await;
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
    // Quellordner von Jobs dürfen beim Start fehlen (Netzlaufwerk noch nicht
    // eingehängt); der Lauf meldet das dann als Fehler.
    let jobs = config
        .jobs
        .iter()
        .map(|job| JobSetup {
            source: FsLocal::new_unchecked(&job.source),
            job: job.clone(),
        })
        .collect();
    if !config.jobs.is_empty() {
        tracing::info!(zone = zone.name(), "Zeitzone für Job-Zeitpläne");
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
    Ok(Core::new(account, state, SystemClock, settings, pairs)
        .with_jobs(jobs, zone)
        .with_mounts(mounts, config.cache.policy, mounter))
}

#[cfg(all(feature = "fuse", target_os = "linux"))]
#[allow(clippy::unnecessary_wraps)] // Ohne FUSE liefert die andere Fassung `None`.
fn mounter() -> Option<Arc<Mounter>> {
    Some(Arc::new(crate::fuse_mount::FuseMounter))
}

#[cfg(not(all(feature = "fuse", target_os = "linux")))]
fn mounter() -> Option<Arc<Mounter>> {
    tracing::warn!(
        "Dieser Build kann keine Laufwerke einhängen (ohne FUSE); [[mount]] wird nur verbunden"
    );
    None
}

/// Startet die Web-Oberfläche. Scheitert sie (Port belegt, TLS-Dateien
/// unbrauchbar), läuft der Daemon ohne sie weiter; der Fehler steht im Protokoll.
fn start_web(
    config: protonsync_web::WebConfig,
    core: &Arc<DaemonCore>,
    shutdown: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    let core = Arc::clone(core);
    tokio::spawn(async move {
        if let Err(error) = protonsync_web::serve(config, core, shutdown).await {
            tracing::error!(%error, "Web-Oberfläche nicht gestartet");
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
        SecretsBackend::Keyring => Ok(Arc::new(protonsync_adapter_secrets::KeyringStore::new())),
        #[cfg(not(feature = "keyring"))]
        SecretsBackend::Keyring => Err(StartupError::KeyringNotBuilt),
    }
}

/// Lokale Änderungen stoßen den Zeitplan an. Ohne Watcher (z. B. inotify-Limit
/// erreicht) bleibt der regelmäßige Abgleich; das wird gemeldet.
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
                tracing::warn!(pair = %pair.id, %error, "Keine lokale Änderungserkennung");
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
            // Ohne Watcher prüft der Zeitplan alle fünf Minuten.
            Err(error) => {
                tracing::info!(job = job.id.as_str(), %error, "Quellordner ohne Änderungserkennung");
            }
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
