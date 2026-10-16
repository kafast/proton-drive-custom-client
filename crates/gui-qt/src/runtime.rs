//! Laufzeit hinter dem QML-Objekt `Tray`: zwei Hintergrund-Threads und das
//! Übertragen ihrer Ergebnisse in den Qt-Thread.
//!
//! - Der **Befehls-Thread** führt Befehle aus der Oberfläche nacheinander aus
//!   ([`Session::execute`]), damit QML nie auf den Daemon wartet.
//! - Der **Ereignis-Thread** hält das Abonnement beim Daemon, führt Ereignisse
//!   nach und verbindet neu, wenn der Daemon neu startet.
//!
//! Qt-Objekte dürfen nur im Qt-Thread verändert werden; deshalb schicken beide
//! Threads ihre [`Update`]s über `qt_thread().queue(...)` dorthin.

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

/// Wartezeit, bevor nach einem verlorenen Daemon neu verbunden wird.
const RECONNECT_INTERVAL: Duration = Duration::from_secs(5);
/// Programmname für den Autostart, falls der eigene Pfad unbekannt ist.
const PROGRAM_NAME: &str = "protonsync-tray";

/// Prüfmodus (`--check`): QML lädt alle Fenster einmal und beendet sich.
static CHECK_MODE: AtomicBool = AtomicBool::new(false);

/// Schaltet den Prüfmodus ein (vor dem Laden von QML aufrufen).
pub fn enable_check_mode() {
    CHECK_MODE.store(true, Ordering::Relaxed);
}

/// Verbindet über den Unix-Socket des Daemons.
struct SocketConnector {
    path: PathBuf,
}

impl Connector for SocketConnector {
    fn connect(&self) -> Result<Box<dyn Transport + Send>, ClientError> {
        Ok(Box::new(SocketTransport::connect(&self.path)?))
    }
}

/// Zustand, den das QML-Objekt `Tray` besitzt.
pub struct Runtime {
    commands: Sender<Command>,
    model: Arc<Mutex<TrayModel>>,
    notifications: Arc<AtomicBool>,
    paths: TrayPaths,
    settings: TraySettings,
    system_lang: Lang,
}

impl Runtime {
    /// Lädt die Einstellungen und startet beide Threads.
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

    /// Reicht einen Befehl an den Befehls-Thread weiter.
    pub fn send(&self, command: Command) {
        // Nur wenn der Thread nicht mehr läuft, scheitert das; dann endet die App ohnehin.
        let _ = self.commands.send(command);
    }

    /// Fester Text in der aktuellen Sprache.
    pub fn text(&self, key: &str) -> String {
        texts::text_or_key(key, self.lang())
    }

    fn lang(&self) -> Lang {
        self.model
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .lang()
    }

    /// Wechselt die Sprache und speichert sie. Liefert eine Fehlermeldung,
    /// wenn das Speichern scheitert.
    pub fn choose_language(&mut self, choice: &str) -> Option<String> {
        let choice = LanguageChoice::parse(choice)?;
        self.settings.language = choice;
        self.model
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_lang(choice.resolve(self.system_lang));
        self.save()
    }

    /// Schaltet Benachrichtigungen um und speichert das.
    pub fn enable_notifications(&mut self, enabled: bool) -> Option<String> {
        self.settings.notifications = enabled;
        self.notifications.store(enabled, Ordering::Relaxed);
        self.save()
    }

    /// Legt den Autostart-Eintrag an oder entfernt ihn.
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

/// Überträgt die Einstellungen und die Ansicht in die QML-Eigenschaften;
/// `error` erscheint als Meldung.
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

/// Befehl zum Entscheiden eines Konflikts; `None` bei unbekannter Wahl.
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

/// Hält das Ereignis-Abonnement und verbindet bei Bedarf neu.
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
                // Was während der Trennung passiert ist, holt die Abfrage nach.
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

    /// Leitet Ereignisse weiter, bis die Verbindung endet. `Err` heißt: Das
    /// Qt-Objekt gibt es nicht mehr, der Thread soll enden.
    fn forward(&self, mut subscription: EventSubscription) -> Result<(), ()> {
        while let Ok(event) = subscription.next_event() {
            let notify = self.notifications.load(Ordering::Relaxed);
            for update in apply_event(&self.model, &event, notify) {
                queue(&self.qt, update)?;
            }
            // Nach einem Lauf haben sich auch Zähler geändert (Papierkorb, Konflikte).
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

/// Schickt ein Update in den Qt-Thread. `Err`, wenn das Qt-Objekt weg ist.
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
            // Erst leeren, damit QML auch einen gleichen Schritt erneut bemerkt.
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
    // Auch dieselbe Meldung zweimal hintereinander soll erscheinen.
    tray.as_mut().set_message(QString::default());
    tray.set_message(QString::from(message));
}

/// Name eines Aufzählungswerts wie in JSON (`snake_case`).
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
        assert_eq!(resolve_command("docs", 4, "irgendwas"), None);
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
