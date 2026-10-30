//! The QML object `Tray`: properties that QML displays and methods that QML
//! calls. There is no logic here; every method hands over to
//! [`crate::runtime`] or `tray-model` (ADR 0011).
//!
//! For Java developers: `#[cxx_qt::bridge]` generates a C++ class
//! `Tray : QObject` from this module. `self: Pin<&mut Tray>` means "mutable
//! access to an object that must not move in memory", because Qt holds
//! pointers to it.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        /// Qt string.
        type QString = cxx_qt_lib::QString;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        /// The tray app's state for QML.
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
        /// Starts the connection to the daemon (once, after QML has loaded).
        #[qinvokable]
        fn start(self: Pin<&mut Tray>);

        /// Fixed UI text in the configured language.
        #[qinvokable]
        fn text(self: &Tray, key: &QString) -> QString;

        /// Query the current state again.
        #[qinvokable]
        fn refresh(self: &Tray);

        /// Sync now (only plan when the daemon is in dry-run mode).
        #[qinvokable]
        fn sync_now(self: &Tray);

        /// Decide a conflict; `choice` is `keep_local`, `keep_remote` or `keep_both`.
        #[qinvokable]
        fn resolve(self: &Tray, pair: &QString, id: i64, choice: &QString);

        /// Start the login.
        #[qinvokable]
        fn login_start(self: &Tray, username: &QString, password: &QString);

        /// Submit the 2FA code.
        #[qinvokable]
        fn login_code(self: &Tray, code: &QString);

        /// Submit the mailbox password.
        #[qinvokable]
        fn login_data_password(self: &Tray, password: &QString);

        /// Log out.
        #[qinvokable]
        fn logout(self: &Tray);

        /// Choose the language: `auto`, `de` or `en`.
        #[qinvokable]
        fn choose_language(self: Pin<&mut Tray>, choice: &QString);

        /// Turn notifications on or off.
        #[qinvokable]
        fn enable_notifications(self: Pin<&mut Tray>, enabled: bool);

        /// Turn autostart on or off.
        #[qinvokable]
        fn enable_autostart(self: Pin<&mut Tray>, enabled: bool);

        /// Daemon setting: dry run (`true`) or active (`false`).
        #[qinvokable]
        fn set_dry_run(self: &Tray, dry_run: bool);

        /// Daemon setting: days after which trash entries expire.
        #[qinvokable]
        fn set_trash_retention_days(self: &Tray, days: i32);

        /// Daemon setting: direction of a sync pair (`two_way`, `download_only`, …).
        #[qinvokable]
        fn set_pair_direction(self: &Tray, pair: &QString, direction: &QString);

        /// Daemon setting: `protect_remote` or `archive_versions` of a sync pair.
        #[qinvokable]
        fn set_pair_option(self: &Tray, pair: &QString, option: &QString, enabled: bool);

        /// Show a desktop notification; `action` says what a click opens.
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
use protonsync_tray_model::daemon_settings;

use crate::runtime::Runtime;

/// The Rust side of the QML object `Tray`.
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

    fn set_dry_run(&self, dry_run: bool) {
        self.change_settings(Some(daemon_settings::dry_run_change(dry_run)));
    }

    fn set_trash_retention_days(&self, days: i32) {
        self.change_settings(daemon_settings::retention_change(i64::from(days)));
    }

    fn set_pair_direction(&self, pair: &QString, direction: &QString) {
        self.change_settings(daemon_settings::direction_change(
            &pair.to_string(),
            &direction.to_string(),
        ));
    }

    fn set_pair_option(&self, pair: &QString, option: &QString, enabled: bool) {
        self.change_settings(daemon_settings::option_change(
            &pair.to_string(),
            &option.to_string(),
            enabled,
        ));
    }

    /// Sends a change; `None` (an invalid value from QML) does nothing.
    fn change_settings(&self, change: Option<protonsync_api::SettingsChange>) {
        if let Some(change) = change {
            self.send(protonsync_tray_model::Command::ChangeSettings(change));
        }
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
    /// The runtime, once [`qobject::Tray::start`] has run.
    pub(crate) fn runtime(&self) -> Option<&Runtime> {
        self.runtime.as_ref()
    }
}
