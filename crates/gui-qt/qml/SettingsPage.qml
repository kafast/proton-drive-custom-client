// Settings: the tray app's own and the daemon's (changed through the daemon, ADR 0021).
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.protonsync.tray

ScrollView {
    id: page

    required property Tray tray
    required property var view
    readonly property var languages: ["auto", "de", "en"]
    // The daemon's editable settings; null while the daemon is not reachable.
    readonly property var settings: view.settings !== undefined ? view.settings : null
    readonly property bool editable: settings !== null && settings.editable

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
                ComboBox {
                    enabled: page.editable
                    model: [page.t("settings.mode_dry_run"), page.t("settings.mode_active")]
                    currentIndex: page.view.dry_run ? 0 : 1
                    onActivated: (index) => page.tray.setDryRun(index === 0)
                }
                Label { text: page.t("settings.trash_days") }
                SpinBox {
                    enabled: page.editable
                    from: 1
                    to: 3650
                    editable: true
                    value: page.settings !== null ? page.settings.trash_retention_days : 30
                    onValueModified: page.tray.setTrashRetentionDays(value)
                }
                Label {
                    Layout.columnSpan: 2
                    Layout.fillWidth: true
                    visible: page.settings !== null && !page.settings.editable
                    text: page.t("settings.not_editable")
                    wrapMode: Text.Wrap
                }
            }
        }

        GroupBox {
            Layout.fillWidth: true
            visible: page.settings !== null && page.settings.pairs.length > 0
            title: page.t("settings.pairs")
            ColumnLayout {
                width: parent.width
                Repeater {
                    model: page.settings !== null ? page.settings.pairs : []
                    delegate: GridLayout {
                        required property var modelData
                        Layout.fillWidth: true
                        columns: 2
                        Label {
                            Layout.columnSpan: 2
                            text: modelData.id
                            font.bold: true
                        }
                        Label { text: page.t("settings.direction") }
                        ComboBox {
                            enabled: page.editable
                            model: page.settings.directions
                            textRole: "label"
                            currentIndex: modelData.direction_index
                            onActivated: (index) => page.tray.setPairDirection(
                                modelData.id, page.settings.directions[index].key)
                        }
                        CheckBox {
                            Layout.columnSpan: 2
                            enabled: page.editable
                            text: page.t("settings.protect_remote")
                            checked: modelData.protect_remote
                            onToggled: page.tray.setPairOption(modelData.id, "protect_remote", checked)
                        }
                        CheckBox {
                            Layout.columnSpan: 2
                            enabled: page.editable
                            text: page.t("settings.archive_versions")
                            checked: modelData.archive_versions
                            onToggled: page.tray.setPairOption(modelData.id, "archive_versions", checked)
                        }
                    }
                }
            }
        }

        GroupBox {
            Layout.fillWidth: true
            title: page.t("settings.config_file")
            GridLayout {
                width: parent.width
                columns: 1
                Label {
                    Layout.fillWidth: true
                    text: page.tray.configPath
                    elide: Text.ElideMiddle
                }
                Button {
                    text: page.t("settings.open_config")
                    onClicked: Qt.openUrlExternally("file://" + encodeURI(page.tray.configPath))
                }
                Label {
                    Layout.fillWidth: true
                    text: page.t("settings.restart_hint")
                    wrapMode: Text.Wrap
                }
            }
        }
    }
}
