// Entry point of the tray app: tray icon with menu, status window and login dialog.
// All texts and states come from Rust (object `Tray`, ADR 0011).
import QtQuick
import Qt.labs.platform as Platform
import org.protonsync.tray

Item {
    id: root

    // View from Rust; an empty state before the daemon's first answer.
    readonly property var view: tray.viewJson ? JSON.parse(tray.viewJson) : ({
        state: "daemon_offline", icon: "offline", headline: "", tooltip: "protonsync",
        daemon_online: false, daemon_version: "", account: null, dry_run: false,
        pairs: [], mounts: [], conflicts: []
    })
    // Target of a click on the last notification.
    property string pendingAction: "overview"

    // Reads `tray.language` so that all texts refresh when the language changes.
    function t(key) { tray.language; return tray.text(key) }

    function showWindow(tab) { statusWindow.showTab(tab) }

    function handleAction(action) {
        if (action === "login")
            loginDialog.start()
        else
            showWindow(action)
    }

    function severityIcon(severity) {
        if (severity === "critical")
            return Platform.SystemTrayIcon.Critical
        if (severity === "warning")
            return Platform.SystemTrayIcon.Warning
        return Platform.SystemTrayIcon.Information
    }

    Tray {
        id: tray
        onNotify: (title, body, severity, action) => {
            root.pendingAction = action
            trayIcon.showMessage(title, body, root.severityIcon(severity), 8000)
        }
    }

    Platform.SystemTrayIcon {
        id: trayIcon
        // Only visible once the icon is set (see Component.onCompleted).
        visible: false
        icon.source: Qt.resolvedUrl("../icons/" + root.view.icon + ".svg")
        tooltip: root.view.tooltip
        onActivated: (reason) => {
            if (reason === Platform.SystemTrayIcon.Trigger)
                root.showWindow("overview")
        }
        onMessageClicked: root.handleAction(root.pendingAction)

        menu: Platform.Menu {
            Platform.MenuItem {
                text: root.view.headline
                enabled: false
            }
            Platform.MenuSeparator {}
            Platform.MenuItem {
                text: root.view.dry_run ? root.t("menu.plan_now") : root.t("menu.sync_now")
                enabled: root.view.account !== null
                onTriggered: tray.syncNow()
            }
            Platform.MenuItem {
                text: root.t("menu.status")
                onTriggered: root.showWindow("overview")
            }
            Platform.MenuItem {
                text: root.t("menu.conflicts") + " (" + root.view.conflicts.length + ")"
                visible: root.view.conflicts.length > 0
                onTriggered: root.showWindow("conflicts")
            }
            Platform.MenuSeparator {}
            Platform.MenuItem {
                text: root.t("menu.login")
                visible: root.view.daemon_online && root.view.account === null
                onTriggered: loginDialog.start()
            }
            Platform.MenuItem {
                text: root.t("menu.logout")
                visible: root.view.account !== null
                onTriggered: tray.logout()
            }
            Platform.MenuItem {
                text: root.t("menu.settings")
                onTriggered: root.showWindow("settings")
            }
            Platform.MenuSeparator {}
            Platform.MenuItem {
                text: root.t("menu.quit")
                onTriggered: Qt.quit()
            }
        }
    }

    StatusWindow {
        id: statusWindow
        tray: tray
        view: root.view
        onLoginRequested: loginDialog.start()
    }

    LoginDialog {
        id: loginDialog
        tray: tray
    }

    // Loads the same icon as an image; in check mode this shows whether the resources are right.
    Image {
        id: iconProbe
        visible: false
        source: trayIcon.icon.source
    }

    Component.onCompleted: {
        Qt.callLater(() => trayIcon.visible = true)
        tray.start()
        if (tray.checkMode) {
            if (iconProbe.status !== Image.Ready) {
                console.error("Icon not found: " + trayIcon.icon.source)
                Qt.exit(4)
                return
            }
            // Check mode: show every window once so that all bindings run.
            for (const tab of ["overview", "conflicts", "settings"])
                statusWindow.showTab(tab)
            loginDialog.start()
            Qt.callLater(Qt.quit)
        }
    }
}
