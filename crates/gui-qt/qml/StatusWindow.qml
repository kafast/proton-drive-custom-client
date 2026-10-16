// Statusfenster mit den Reitern Übersicht, Konflikte und Einstellungen.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.protonsync.tray

ApplicationWindow {
    id: window

    required property Tray tray
    required property var view
    readonly property var tabNames: ["overview", "conflicts", "settings"]

    signal loginRequested()

    function t(key) { tray.language; return tray.text(key) }

    function showTab(name) {
        tabs.currentIndex = Math.max(0, tabNames.indexOf(name))
        show()
        raise()
        requestActivate()
        tray.refresh()
    }

    title: t("app.name") + " – " + view.headline
    width: 640
    height: 540
    minimumWidth: 420
    minimumHeight: 360

    header: TabBar {
        id: tabs
        TabButton { text: window.t("tab.overview") }
        TabButton {
            text: window.t("tab.conflicts")
                  + (window.view.conflicts.length > 0 ? " (" + window.view.conflicts.length + ")" : "")
        }
        TabButton { text: window.t("tab.settings") }
    }

    StackLayout {
        anchors.fill: parent
        anchors.margins: 12
        currentIndex: tabs.currentIndex

        OverviewPage {
            tray: window.tray
            view: window.view
            onLoginRequested: window.loginRequested()
        }
        ConflictsPage {
            tray: window.tray
            view: window.view
        }
        SettingsPage {
            tray: window.tray
            view: window.view
        }
    }

    footer: Label {
        id: messageLabel
        visible: text.length > 0
        padding: 8
        wrapMode: Text.Wrap
    }

    // Meldungen aus Rust (z. B. Fehlerantworten) erscheinen für einige Sekunden.
    Connections {
        target: window.tray
        function onMessageChanged() {
            if (window.tray.message.length > 0) {
                messageLabel.text = window.tray.message
                hideMessage.restart()
            }
        }
    }

    Timer {
        id: hideMessage
        interval: 8000
        onTriggered: messageLabel.text = ""
    }
}
