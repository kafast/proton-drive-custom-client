//! Tray app `protonsync-tray` (Qt 6/QML via cxx-qt, ADR 0011).
//!
//! The app is only a display: what it shows is computed by `tray-model`, and
//! everything it does goes through the daemon API.

// `unsafe` is only allowed in the two bridge modules whose code cxx-qt and
// cxx generate; everything else in this crate stays free of `unsafe`.
#[allow(unsafe_code)]
mod application;
#[allow(unsafe_code)]
mod bridge;
mod runtime;

use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use cxx_qt_lib::{QQmlApplicationEngine, QUrl};

/// Main QML file in the Qt resource system (from the QML module in `build.rs`).
const MAIN_QML: &str = "qrc:/qt/qml/org/protonsync/tray/qml/Main.qml";
/// The longest `--check` may take before it counts as failed.
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
/// Exit code if the check does not finish in time.
const CHECK_FAILED: u8 = 1;

const HELP: &str = "\
protonsync-tray: Tray-App für protonsync / tray app for protonsync

Aufruf / usage: protonsync-tray [--check | --version | --help]

  --check     QML-Dateien laden und beenden (für Tests, z. B. mit
              QT_QPA_PLATFORM=offscreen) / load all QML files and exit
  --version   Version anzeigen / show the version
  --help      Diese Hilfe / this help

Der Daemon protonsyncd muss laufen; die App verbindet sich über denselben
Socket wie die CLI (PROTONSYNC_SOCKET). / The daemon protonsyncd must be
running; the app connects through the same socket as the CLI.
";

// `--version` and `--help` deliberately write to standard output.
#[allow(clippy::print_stdout)]
fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None => {}
        Some("--check") => {
            runtime::enable_check_mode();
            // If QML does not load (error while loading), QML never quits by itself.
            thread::spawn(|| {
                thread::sleep(CHECK_TIMEOUT);
                eprintln!("protonsync-tray --check: QML did not finish loading");
                std::process::exit(i32::from(CHECK_FAILED));
            });
        }
        Some("--version") => {
            println!("protonsync-tray {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some("--help" | "-h") => {
            print!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Some(other) => {
            eprintln!("protonsync-tray: unbekannte Option / unknown option: {other}\n\n{HELP}");
            return ExitCode::from(2);
        }
    }
    run()
}

fn run() -> ExitCode {
    let mut app = application::ffi::new_application();
    let mut engine = QQmlApplicationEngine::new();
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from(MAIN_QML));
    }
    let code = app
        .as_mut()
        .map_or(i32::from(CHECK_FAILED), application::ffi::run_application);
    ExitCode::from(u8::try_from(code).unwrap_or(CHECK_FAILED))
}
