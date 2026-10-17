//! Runtime behind the QML object `Tray`: two background threads and the
//! transfer of their results into the Qt thread.
//!
//! - The **command thread** executes commands from the UI one after another
//!   ([`Session::execute`]), so that QML never waits for the daemon.
//! - The **event thread** holds the subscription with the daemon, applies
//!   events and reconnects when the daemon restarts.
//!
//! Qt objects may only be changed in the Qt thread; that is why both threads
//! send their [`Update`]s there via `qt_thread().queue(...)`.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::QString;
use jiff::tz::TimeZone;
use protonsync_api::wire::default_socket_path;
use protonsync_api::{Event, ResolutionChoice};
use protonsync_client::{ClientError, EventSubscription, Lang, SocketTransport, Transport};
use protonsync_tray_model::session::{apply_event, mark_offline};
use protonsync_tray_model::settings::{autostart_enabled, set_autostart};
use protonsync_tray_model::{
    Command, Connector, LanguageChoice, Session, TrayModel, TrayPaths, TraySettings, Update, texts,
};
use serde::Serialize;

use crate::bridge::qobject::Tray;

/// Wait time before reconnecting after losing the daemon.
const RECONNECT_INTERVAL: Duration = Duration::from_secs(5);
/// Program name for autostart if the app's own path is unknown.
const PROGRAM_NAME: &str = "protonsync-tray";

/// Check mode (`--check`): QML loads every window once and quits.
static CHECK_MODE: AtomicBool = AtomicBool::new(false);

/// Turns on check mode (call before loading QML).
pub fn enable_check_mode() {
    CHECK_MODE.store(true, Ordering::Relaxed);
}

/// Connects through the daemon's Unix socket.
struct SocketConnector {
    path: PathBuf,
}

impl Connector for SocketConnector {
    fn connect(&self) -> Result<Box<dyn Transport + Send>, ClientError> {
        Ok(Box::new(SocketTransport::connect(&self.path)?))
    }
}

/// State owned by the QML object `Tray`.
pub struct Runtime {
    commands: Sender<Command>,
    model: Arc<Mutex<TrayModel>>,
    notifications: Arc<AtomicBool>,
    paths: TrayPaths,
    settings: TraySettings,
    system_lang: Lang,
}

impl Runtime {
    /// Loads the settings and starts both threads.
    pub fn start(tray: &Tray) -> Self {
        let paths = TrayPaths::from_process();
        let settings = TraySettings::load(&paths.settings).unwrap_or_else(|error| {
            eprintln!("protonsync-tray: {error}");
            TraySettings::default()
        });
        let system_lang = Lang::from_env();
        let model = Arc::new(Mutex::new(TrayModel::new(
            settings.language.resolve(system_lang),
            TimeZone::system(),
        )));
        let notifications = Arc::new(AtomicBool::new(settings.notifications));
        let (commands, receiver) = mpsc::channel();
        let socket = default_socket_path();

        let session = Session::new(
            SocketConnector {
                path: socket.clone(),
            },
            Arc::clone(&model),
        );
        let qt = tray.qt_thread();
        thread::spawn(move || run_commands(session, &receiver, &qt));

        let events = EventWatcher {
            socket,
            model: Arc::clone(&model),
            notifications: Arc::clone(&notifications),
            commands: commands.clone(),
            qt: tray.qt_thread(),
        };
        thread::spawn(move || events.run());

        Self {
            commands,
            model,
            notifications,
            paths,
            settings,
            system_lang,
        }
    }

    /// Hands a command to the command thread.
    pub fn send(&self, command: Command) {
        // This fails only if the thread is no longer running; then the app ends anyway.
        let _ = self.commands.send(command);
    }

    /// Fixed text in the current language.
    pub fn text(&self, key: &str) -> String {
        texts::text_or_key(key, self.lang())
    }

    fn lang(&self) -> Lang {
        self.model
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .lang()
    }

    /// Switches the language and saves it. Returns an error message if
    /// saving fails.
    pub fn choose_language(&mut self, choice: &str) -> Option<String> {
        let choice = LanguageChoice::parse(choice)?;
        self.settings.language = choice;
        self.model
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_lang(choice.resolve(self.system_lang));
        self.save()
    }

    /// Toggles notifications and saves the setting.
    pub fn enable_notifications(&mut self, enabled: bool) -> Option<String> {
        self.settings.notifications = enabled;
        self.notifications.store(enabled, Ordering::Relaxed);
        self.save()
    }

    /// Creates or removes the autostart entry.
    pub fn enable_autostart(&self, enabled: bool) -> Option<String> {
        let program = std::env::current_exe().unwrap_or_else(|_| PathBuf::from(PROGRAM_NAME));
        set_autostart(&self.paths, enabled, &program)
            .err()
            .map(|e| e.to_string())
    }

    fn save(&self) -> Option<String> {
        self.settings
            .save(&self.paths.settings)
            .err()
            .map(|e| e.to_string())
    }

    fn view_json(&self) -> String {
        let view = self
            .model
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .view();
        serde_json::to_string(&view).unwrap_or_default()
    }

    fn config_path(&self) -> &Path {
        &self.paths.daemon_config
    }
}

/// Transfers the settings and the view into the QML properties;
/// `error` appears as a message.
pub fn show_settings(mut tray: Pin<&mut Tray>, error: Option<String>) {
    let Some(runtime) = tray.rust().runtime() else {
        return;
    };
    let language = name(&runtime.settings.language);
    let notifications = runtime.settings.notifications;
    let autostart = autostart_enabled(&runtime.paths);
    let config_path = runtime.config_path().display().to_string();
    let view = runtime.view_json();
    tray.as_mut()
        .set_check_mode(CHECK_MODE.load(Ordering::Relaxed));
    tray.as_mut().set_language(QString::from(&language));
    tray.as_mut().set_notifications(notifications);
    tray.as_mut().set_autostart(autostart);
    tray.as_mut().set_config_path(QString::from(&config_path));
    tray.as_mut().set_view_json(QString::from(&view));
    if let Some(error) = error {
        show_message(tray, &error);
    }
}

/// Command to decide a conflict; `None` for an unknown choice.
pub fn resolve_command(pair: &str, id: i64, choice: &str) -> Option<Command> {
    let choice = match choice {
        "keep_local" => ResolutionChoice::KeepLocal,
        "keep_remote" => ResolutionChoice::KeepRemote,
        "keep_both" => ResolutionChoice::KeepBoth,
        _ => return None,
    };
    Some(Command::Resolve {
        pair: pair.to_owned(),
        id,
        choice,
    })
}

fn run_commands(
    mut session: Session<SocketConnector>,
    commands: &Receiver<Command>,
    qt: &CxxQtThread<Tray>,
) {
    for command in commands {
        for update in session.execute(command) {
            if queue(qt, update).is_err() {
                return;
            }
        }
    }
}

/// Holds the event subscription and reconnects when needed.
struct EventWatcher {
    socket: PathBuf,
    model: Arc<Mutex<TrayModel>>,
    notifications: Arc<AtomicBool>,
    commands: Sender<Command>,
    qt: CxxQtThread<Tray>,
}

impl EventWatcher {
    fn run(self) {
        loop {
            if let Ok(subscription) = EventSubscription::connect(&self.socket) {
                // The query catches up on whatever happened while disconnected.
                let _ = self.commands.send(Command::Refresh);
                if self.forward(subscription).is_err() {
                    return;
                }
                if queue(&self.qt, mark_offline(&self.model)).is_err() {
                    return;
                }
            }
            thread::sleep(RECONNECT_INTERVAL);
        }
    }

    /// Forwards events until the connection ends. `Err` means: the Qt object
    /// no longer exists, the thread should end.
    fn forward(&self, mut subscription: EventSubscription) -> Result<(), ()> {
        while let Ok(event) = subscription.next_event() {
            let notify = self.notifications.load(Ordering::Relaxed);
            for update in apply_event(&self.model, &event, notify) {
                queue(&self.qt, update)?;
            }
            // After a run, counters have changed too (trash, conflicts).
            if matches!(
                event,
                Event::SyncFinished(_) | Event::JobFinished(_) | Event::LoginRequired { .. }
            ) {
                let _ = self.commands.send(Command::Refresh);
            }
        }
        Ok(())
    }
}

/// Sends an update to the Qt thread. `Err` if the Qt object is gone.
fn queue(qt: &CxxQtThread<Tray>, update: Update) -> Result<(), ()> {
    qt.queue(move |tray| apply(tray, update)).map_err(|_| ())
}

fn apply(mut tray: Pin<&mut Tray>, update: Update) {
    match update {
        Update::View(view) => {
            let json = serde_json::to_string(&view).unwrap_or_default();
            tray.set_view_json(QString::from(&json));
        }
        Update::Login { stage, error } => {
            // Clear first, so that QML also notices the same step again.
            tray.as_mut().set_login_error(QString::default());
            tray.as_mut().set_login_stage(QString::default());
            tray.as_mut()
                .set_login_error(QString::from(&error.unwrap_or_default()));
            tray.set_login_stage(QString::from(&name(&stage)));
        }
        Update::Message(message) => show_message(tray, &message),
        Update::Notify(notification) => tray.notify(
            QString::from(&notification.title),
            QString::from(&notification.body),
            QString::from(&name(&notification.severity)),
            QString::from(&name(&notification.action)),
        ),
    }
}

fn show_message(mut tray: Pin<&mut Tray>, message: &str) {
    // The same message twice in a row should also appear.
    tray.as_mut().set_message(QString::default());
    tray.set_message(QString::from(message));
}

/// Name of an enum value as in JSON (`snake_case`).
fn name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_choices_become_commands() {
        assert_eq!(
            resolve_command("docs", 4, "keep_both"),
            Some(Command::Resolve {
                pair: "docs".into(),
                id: 4,
                choice: ResolutionChoice::KeepBoth
            })
        );
        assert_eq!(resolve_command("docs", 4, "whatever"), None);
    }

    #[test]
    fn names_match_qml() {
        assert_eq!(name(&LanguageChoice::Auto), "auto");
        assert_eq!(
            name(&protonsync_tray_model::LoginStage::DataPassword),
            "data_password"
        );
    }
}
