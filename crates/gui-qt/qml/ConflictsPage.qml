// Konfliktliste: Für jeden Konflikt eine Entscheidung. Die verworfene Version
// landet im Papierkorb und lässt sich wiederherstellen.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.protonsync.tray

ScrollView {
    id: page

    required property Tray tray
    required property var view

    function t(key) { tray.language; return tray.text(key) }

    contentWidth: availableWidth

    ColumnLayout {
        width: page.availableWidth
        spacing: 10

        Label {
            visible: page.view.conflicts.length === 0
            text: page.t("conflicts.none")
        }
        Label {
            Layout.fillWidth: true
            visible: page.view.conflicts.length > 0
            text: page.t("conflicts.hint")
            wrapMode: Text.Wrap
        }

        Repeater {
            model: page.view.conflicts
            delegate: Frame {
                required property var modelData
                Layout.fillWidth: true
                ColumnLayout {
                    width: parent.width
                    Label {
                        Layout.fillWidth: true
                        text: modelData.path
                        font.bold: true
                        elide: Text.ElideMiddle
                    }
                    Label {
                        Layout.fillWidth: true
                        text: modelData.pair + " · " + modelData.kind + " · " + modelData.detected
                        wrapMode: Text.Wrap
                    }
                    Label {
                        Layout.fillWidth: true
                        visible: modelData.copy !== null
                        text: modelData.copy !== null ? modelData.copy : ""
                        elide: Text.ElideMiddle
                    }
                    RowLayout {
                        Button {
                            text: page.t("conflicts.keep_local")
                            onClicked: page.tray.resolve(modelData.pair, modelData.id, "keep_local")
                        }
                        Button {
                            text: page.t("conflicts.keep_remote")
                            onClicked: page.tray.resolve(modelData.pair, modelData.id, "keep_remote")
                        }
                        Button {
                            text: page.t("conflicts.keep_both")
                            onClicked: page.tray.resolve(modelData.pair, modelData.id, "keep_both")
                        }
                    }
                }
            }
        }
    }
}
