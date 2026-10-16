//! Tray-App `protonsync-tray` (Qt 6/QML über cxx-qt, ADR 0011).
//!
//! Die App ist nur eine Anzeige: Was sie zeigt, berechnet `tray-model`, und
//! alles, was sie tut, geschieht über die Daemon-API.

// `unsafe` ist nur in den zwei Brückenmodulen erlaubt, deren Code cxx-qt bzw.
// cxx erzeugen; alles andere in diesem Crate bleibt ohne `unsafe`.
#[allow(unsafe_code)]
mod application;
#[allow(unsafe_code)]
mod bridge;
mod runtime;

use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use cxx_qt_lib::{QQmlApplicationEngine, QUrl};

/// Haupt-QML-Datei im Qt-Ressourcensystem (aus dem QML-Modul in `build.rs`).
const MAIN_QML: &str = "qrc:/qt/qml/org/protonsync/tray/qml/Main.qml";
/// So lange darf `--check` höchstens brauchen, bevor es als gescheitert gilt.
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
/// Exit-Code, wenn die Prüfung nicht rechtzeitig fertig wird.
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

// `--version` und `--help` schreiben bewusst auf die Standardausgabe.
#[allow(clippy::print_stdout)]
fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None => {}
        Some("--check") => {
            runtime::enable_check_mode();
            // Lädt QML nicht (Fehler beim Laden), beendet sich QML nie selbst.
            thread::spawn(|| {
                thread::sleep(CHECK_TIMEOUT);
                eprintln!("protonsync-tray --check: QML wurde nicht fertig geladen");
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
