// Overview: account, dry-run hint, sync folders and mounts.
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
            Label {
                width: parent.width
                text: page.t("overview.dry_run_hint")
                wrapMode: Text.Wrap
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
    }
}
