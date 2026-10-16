//! Das QML-Objekt `Tray`: Eigenschaften, die QML anzeigt, und Methoden, die
//! QML aufruft. Hier steht keine Logik; jede Methode reicht an
//! [`crate::runtime`] bzw. `tray-model` weiter (ADR 0011).
//!
//! Für Java-Entwickler: `#[cxx_qt::bridge]` erzeugt aus diesem Modul eine
//! C++-Klasse `Tray : QObject`. `self: Pin<&mut Tray>` heißt „veränderbarer
//! Zugriff auf ein Objekt, das sich im Speicher nicht verschieben darf“, weil
//! Qt Zeiger darauf hält.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        /// Qt-Zeichenkette.
        type QString = cxx_qt_lib::QString;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        /// Der Zustand der Tray-App für QML.
        #[qobject]
        #[qml_element]
        #[qproperty(QString, view_json)]
        #[qproperty(QString, login_stage)]
        #[qproperty(QString, login_error)]
        #[qproperty(QString, message)]
        #[qproperty(QString, language)]
        #[qproperty(bool, notifications)]
        #[qproperty(bool, autostart)]
        #[qproperty(QString, config_path)]
        #[qproperty(bool, check_mode)]
        type Tray = super::TrayRust;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        /// Startet die Verbindung zum Daemon (einmal, nach dem Laden von QML).
        #[qinvokable]
        fn start(self: Pin<&mut Tray>);

        /// Fester Text der Oberfläche in der eingestellten Sprache.
        #[qinvokable]
        fn text(self: &Tray, key: &QString) -> QString;

        /// Stand neu abfragen.
        #[qinvokable]
        fn refresh(self: &Tray);

        /// Jetzt abgleichen (im Probelauf-Modus des Daemons nur planen).
        #[qinvokable]
        fn sync_now(self: &Tray);

        /// Konflikt entscheiden; `choice` ist `keep_local`, `keep_remote` oder `keep_both`.
        #[qinvokable]
        fn resolve(self: &Tray, pair: &QString, id: i64, choice: &QString);

        /// Anmeldung beginnen.
        #[qinvokable]
        fn login_start(self: &Tray, username: &QString, password: &QString);

        /// 2FA-Code nachreichen.
        #[qinvokable]
        fn login_code(self: &Tray, code: &QString);

        /// Mailbox-Passwort nachreichen.
        #[qinvokable]
        fn login_data_password(self: &Tray, password: &QString);

        /// Abmelden.
        #[qinvokable]
        fn logout(self: &Tray);

        /// Sprache wählen: `auto`, `de` oder `en`.
        #[qinvokable]
        fn choose_language(self: Pin<&mut Tray>, choice: &QString);

        /// Benachrichtigungen ein- oder ausschalten.
        #[qinvokable]
        fn enable_notifications(self: Pin<&mut Tray>, enabled: bool);

        /// Autostart ein- oder ausschalten.
        #[qinvokable]
        fn enable_autostart(self: Pin<&mut Tray>, enabled: bool);

        /// Eine Desktop-Benachrichtigung anzeigen; `action` sagt, was ein Klick öffnet.
        #[qsignal]
        fn notify(
            self: Pin<&mut Tray>,
            title: QString,
            body: QString,
            severity: QString,
            action: QString,
        );
    }

    impl cxx_qt::Threading for Tray {}
}

use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::QString;

use crate::runtime::Runtime;

/// Die Rust-Seite des QML-Objekts `Tray`.
#[derive(Default)]
pub struct TrayRust {
    view_json: QString,
    login_stage: QString,
    login_error: QString,
    message: QString,
    language: QString,
    notifications: bool,
    autostart: bool,
    config_path: QString,
    check_mode: bool,
    runtime: Option<Runtime>,
}

impl qobject::Tray {
    fn start(mut self: Pin<&mut Self>) {
        if self.rust().runtime.is_some() {
            return;
        }
        let runtime = Runtime::start(&self);
        self.as_mut().rust_mut().get_mut().runtime = Some(runtime);
        crate::runtime::show_settings(self.as_mut(), None);
        self.refresh();
    }

    fn text(&self, key: &QString) -> QString {
        let text = self
            .rust()
            .runtime
            .as_ref()
            .map_or_else(|| format!("[{key}]"), |r| r.text(&key.to_string()));
        QString::from(&text)
    }

    fn refresh(&self) {
        self.send(protonsync_tray_model::Command::Refresh);
    }

    fn sync_now(&self) {
        self.send(protonsync_tray_model::Command::SyncNow);
    }

    fn resolve(&self, pair: &QString, id: i64, choice: &QString) {
        if let Some(command) =
            crate::runtime::resolve_command(&pair.to_string(), id, &choice.to_string())
        {
            self.send(command);
        }
    }

    fn login_start(&self, username: &QString, password: &QString) {
        self.send(protonsync_tray_model::Command::LoginStart {
            username: username.to_string(),
            password: protonsync_api::Password::new(password.to_string()),
        });
    }

    fn login_code(&self, code: &QString) {
        self.send(protonsync_tray_model::Command::LoginCode(
            code.to_string().trim().to_owned(),
        ));
    }

    fn login_data_password(&self, password: &QString) {
        self.send(protonsync_tray_model::Command::LoginDataPassword(
            protonsync_api::Password::new(password.to_string()),
        ));
    }

    fn logout(&self) {
        self.send(protonsync_tray_model::Command::Logout);
    }

    fn choose_language(mut self: Pin<&mut Self>, choice: &QString) {
        let choice = choice.to_string();
        let error = self
            .as_mut()
            .rust_mut()
            .get_mut()
            .runtime
            .as_mut()
            .and_then(|runtime| runtime.choose_language(&choice));
        crate::runtime::show_settings(self, error);
    }

    fn enable_notifications(mut self: Pin<&mut Self>, enabled: bool) {
        let error = self
            .as_mut()
            .rust_mut()
            .get_mut()
            .runtime
            .as_mut()
            .and_then(|runtime| runtime.enable_notifications(enabled));
        crate::runtime::show_settings(self, error);
    }

    fn enable_autostart(mut self: Pin<&mut Self>, enabled: bool) {
        let error = self
            .as_mut()
            .rust_mut()
            .get_mut()
            .runtime
            .as_mut()
            .and_then(|runtime| runtime.enable_autostart(enabled));
        crate::runtime::show_settings(self, error);
    }

    fn send(&self, command: protonsync_tray_model::Command) {
        if let Some(runtime) = &self.rust().runtime {
            runtime.send(command);
        }
    }
}

impl TrayRust {
    /// Die Laufzeit, sobald [`qobject::Tray::start`] lief.
    pub(crate) fn runtime(&self) -> Option<&Runtime> {
        self.runtime.as_ref()
    }
}
