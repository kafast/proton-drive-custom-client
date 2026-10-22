// Overview: account, dry-run hint with commands, sync folders, mounts and help.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.protonsync.tray

ScrollView {
    id: page

    required property Tray tray
    required property var view

    signal loginRequested()

    function t(key) { tray.language; return tray.text(key) }

    function openFolder(path) {
        Qt.openUrlExternally("file://" + encodeURI(path))
    }

    // Terminal commands for a one-off sync, each below a comment in the UI language.
    // The per-folder example uses the first configured pair's ID.
    function syncCommands() {
        const pair = view.pairs.length > 0 ? view.pairs[0].id : "ID"
        return [
            "# " + t("overview.cmd_plan"), "protonsync sync", "",
            "# " + t("overview.cmd_apply"), "protonsync sync --apply", "",
            "# " + t("overview.cmd_apply_pair"), "protonsync sync --pair " + pair + " --apply"
        ].join("\n")
    }

    contentWidth: availableWidth

    ColumnLayout {
        width: page.availableWidth
        spacing: 10

        Label {
            Layout.fillWidth: true
            visible: !page.view.daemon_online
            text: page.t("overview.daemon_offline")
            wrapMode: Text.Wrap
        }

        RowLayout {
            Layout.fillWidth: true
            visible: page.view.daemon_online
            Label {
                Layout.fillWidth: true
                text: page.t("overview.account") + ": "
                      + (page.view.account !== null ? page.view.account : page.t("overview.not_logged_in"))
                elide: Text.ElideRight
            }
            Button {
                visible: page.view.account === null
                text: page.t("menu.login")
                onClicked: page.loginRequested()
            }
        }

        Frame {
            Layout.fillWidth: true
            visible: page.view.dry_run
            ColumnLayout {
                width: parent.width
                spacing: 6
                Label {
                    text: page.t("overview.dry_run_title")
                    font.bold: true
                }
                Label {
                    Layout.fillWidth: true
                    text: page.t("overview.dry_run_hint")
                    wrapMode: Text.Wrap
                }
                // Read-only but selectable, so the commands can be copied.
                TextArea {
                    Layout.fillWidth: true
                    readOnly: true
                    selectByMouse: true
                    wrapMode: TextEdit.Wrap
                    font.family: "monospace"
                    text: page.syncCommands()
                }
                Label {
                    Layout.fillWidth: true
                    text: page.t("overview.dry_run_off")
                    wrapMode: Text.Wrap
                }
                Button {
                    text: page.t("settings.open_config")
                    onClicked: Qt.openUrlExternally("file://" + encodeURI(page.tray.configPath))
                }
            }
        }

        Label {
            text: page.t("overview.pairs")
            font.bold: true
            visible: page.view.daemon_online
        }
        Label {
            visible: page.view.daemon_online && page.view.pairs.length === 0
            text: page.t("overview.no_pairs")
        }
        Repeater {
            model: page.view.pairs
            delegate: Frame {
                required property var modelData
                Layout.fillWidth: true
                RowLayout {
                    width: parent.width
                    ColumnLayout {
                        Layout.fillWidth: true
                        Label {
                            text: modelData.id + " (" + modelData.direction + ")"
                            font.bold: true
                        }
                        Label {
                            Layout.fillWidth: true
                            text: modelData.local + "  ⇄  " + modelData.remote
                            elide: Text.ElideMiddle
                        }
                        Label {
                            Layout.fillWidth: true
                            text: modelData.status
                            color: modelData.state === "error" ? "#d23c3c" : palette.windowText
                            wrapMode: Text.Wrap
                        }
                    }
                    Button {
                        text: page.t("overview.open_folder")
                        onClicked: page.openFolder(modelData.local)
                    }
                }
            }
        }

        Label {
            text: page.t("overview.mounts")
            font.bold: true
            visible: page.view.mounts.length > 0
        }
        Repeater {
            model: page.view.mounts
            delegate: Frame {
                required property var modelData
                Layout.fillWidth: true
                RowLayout {
                    width: parent.width
                    ColumnLayout {
                        Layout.fillWidth: true
                        Label {
                            text: modelData.id
                            font.bold: true
                        }
                        Label {
                            Layout.fillWidth: true
                            text: modelData.mountpoint
                            elide: Text.ElideMiddle
                        }
                        Label {
                            Layout.fillWidth: true
                            text: modelData.status
                            wrapMode: Text.Wrap
                        }
                    }
                    Button {
                        enabled: modelData.mounted
                        text: page.t("overview.open_folder")
                        onClicked: page.openFolder(modelData.mountpoint)
                    }
                }
            }
        }

        GroupBox {
            Layout.fillWidth: true
            Layout.topMargin: 6
            title: page.t("help.title")
            ColumnLayout {
                width: parent.width
                spacing: 6
                Label {
                    Layout.fillWidth: true
                    text: page.t("help.actions")
                    textFormat: Text.StyledText
                    wrapMode: Text.Wrap
                }
                Label {
                    Layout.fillWidth: true
                    text: page.t("help.commands")
                    wrapMode: Text.Wrap
                }
                TextArea {
                    Layout.fillWidth: true
                    readOnly: true
                    selectByMouse: true
                    font.family: "monospace"
                    text: "protonsync status\nprotonsync conflicts\nprotonsync trash\nprotonsync --help"
                }
                RowLayout {
                    Button {
                        text: page.t("help.open_manual")
                        onClicked: Qt.openUrlExternally(page.t("help.manual_url"))
                    }
                    Button {
                        text: page.t("help.open_commands")
                        onClicked: Qt.openUrlExternally(page.t("help.commands_url"))
                    }
                }
                Label {
                    Layout.fillWidth: true
                    text: page.t("help.local_manual")
                    wrapMode: Text.Wrap
                    opacity: 0.7
                }
            }
        }
    }
}
