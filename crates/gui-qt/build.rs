//! Compiles the Qt bridge and the QML files (ADR 0011).

use cxx_qt_build::{CxxQtBuilder, QmlModule};

const QML_FILES: [&str; 6] = [
    "qml/Main.qml",
    "qml/StatusWindow.qml",
    "qml/OverviewPage.qml",
    "qml/ConflictsPage.qml",
    "qml/SettingsPage.qml",
    "qml/LoginDialog.qml",
];

const ICONS: [&str; 5] = [
    "icons/idle.svg",
    "icons/syncing.svg",
    "icons/attention.svg",
    "icons/error.svg",
    "icons/offline.svg",
];

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("org.protonsync.tray").qml_files(QML_FILES))
        .qt_module("Widgets")
        .qrc_resources(ICONS)
        .files(["src/bridge.rs", "src/application.rs"])
        .cpp_file("cpp/application.cpp")
        .build();
}
