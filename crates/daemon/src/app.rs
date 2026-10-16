//! Zusammenbau (Composition Root): Hier werden die echten Adapter erzeugt und
//! an den Kern übergeben. Nur dieser Teil kennt alle konkreten Typen.

use std::path::PathBuf;
use std::sync::Arc;

use protonsync_adapter_fs::{FsLocal, LocalWatcher};
use protonsync_adapter_proton::ProtonAccount;
use protonsync_adapter_secrets::{EncryptedFileStore, KeyError, MasterKey};
use protonsync_adapter_state::SqliteStateRepo;
use protonsync_application::ports::{
    LocalError, SecretError, SecretStore, StateError, SystemClock,
};
use thiserror::Error;
use tokio::sync::watch;

use crate::config::{Config, ConfigError, Environment, SecretsBackend};
use crate::core::{Core, PairSetup, Settings};
use crate::{scheduler, server};

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

type DaemonCore = Core<ProtonAccount<dyn SecretStore>, FsLocal, SqliteStateRepo, SystemClock>;

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
    tracing::info!(config = %env.config_path().display(), pairs = config.pairs.len(), dry_run = config.dry_run, "Start");
    create_private_dir(&config.data_dir)?;

    let core = Arc::new(build_core(&config)?);
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
    let scheduling = tokio::spawn(scheduler::run(Arc::clone(&core), config.timing, shutdown));

    wait_for_shutdown_signal().await;
    tracing::info!("Beende");
    let _ = stop.send(true);
    let _ = serving.await;
    let _ = scheduling.await;
    drop(watchers);
    let _ = std::fs::remove_file(&socket);
    Ok(())
}

fn build_core(config: &Config) -> Result<DaemonCore, StartupError> {
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
    Ok(Core::new(account, state, SystemClock, settings, pairs))
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
