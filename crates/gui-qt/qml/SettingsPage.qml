// Settings: the tray app's own (editable) and the daemon's (read-only).
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.protonsync.tray

ScrollView {
    id: page

    required property Tray tray
    required property var view
    readonly property var languages: ["auto", "de", "en"]

    function t(key) { tray.language; return tray.text(key) }

    contentWidth: availableWidth

    ColumnLayout {
        width: page.availableWidth
        spacing: 12

        GroupBox {
            Layout.fillWidth: true
            title: page.t("settings.tray")
            GridLayout {
                width: parent.width
                columns: 2
                Label { text: page.t("settings.language") }
                ComboBox {
                    model: [page.t("settings.language_auto"), "Deutsch", "English"]
                    currentIndex: Math.max(0, page.languages.indexOf(page.tray.language))
                    onActivated: (index) => page.tray.chooseLanguage(page.languages[index])
                }
                CheckBox {
                    Layout.columnSpan: 2
                    text: page.t("settings.notifications")
                    checked: page.tray.notifications
                    onToggled: page.tray.enableNotifications(checked)
                }
                CheckBox {
                    Layout.columnSpan: 2
                    text: page.t("settings.autostart")
                    checked: page.tray.autostart
                    onToggled: page.tray.enableAutostart(checked)
                }
            }
        }

        GroupBox {
            Layout.fillWidth: true
            title: page.t("settings.daemon")
            GridLayout {
                width: parent.width
                columns: 2
                Label { text: page.t("settings.daemon_version") }
                Label { text: page.view.daemon_version }
                Label { text: page.t("settings.mode") }
                Label {
                    text: page.view.dry_run ? page.t("settings.mode_dry_run") : page.t("settings.mode_active")
                }
                Label { text: page.t("settings.config_file") }
                Label {
                    Layout.fillWidth: true
                    text: page.tray.configPath
                    elide: Text.ElideMiddle
                }
                Button {
                    Layout.columnSpan: 2
                    text: page.t("settings.open_config")
                    onClicked: Qt.openUrlExternally("file://" + encodeURI(page.tray.configPath))
                }
                Label {
                    Layout.columnSpan: 2
                    Layout.fillWidth: true
                    text: page.t("settings.restart_hint")
                    wrapMode: Text.Wrap
                }
            }
        }
    }
}
