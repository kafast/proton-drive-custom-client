// Login in steps: credentials, then the 2FA code and the mailbox password if needed.
// Inputs go straight to the daemon and are then cleared from the fields.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.protonsync.tray

ApplicationWindow {
    id: dialog

    required property Tray tray
    // credentials, code or data_password
    property string stage: "credentials"
    property bool busy: false

    function t(key) { tray.language; return tray.text(key) }

    function start() {
        stage = "credentials"
        busy = false
        errorLabel.text = ""
        show()
        raise()
        requestActivate()
        username.forceActiveFocus()
    }

    function submit() {
        busy = true
        errorLabel.text = ""
        if (stage === "credentials") {
            tray.loginStart(username.text, password.text)
            password.text = ""
        } else if (stage === "code") {
            tray.loginCode(code.text)
            code.text = ""
        } else {
            tray.loginDataPassword(dataPassword.text)
            dataPassword.text = ""
        }
    }

    title: t("login.title")
    width: 420
    height: 300
    minimumWidth: 360
    minimumHeight: 260
    onClosing: {
        password.text = ""
        code.text = ""
        dataPassword.text = ""
    }

    Connections {
        target: dialog.tray
        function onLoginStageChanged() {
            const next = dialog.tray.loginStage
            if (next.length === 0)
                return
            dialog.busy = false
            errorLabel.text = dialog.tray.loginError
            if (next === "done")
                dialog.close()
            else
                dialog.stage = next
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 16
        spacing: 8

        Label {
            text: dialog.t("login.username")
            visible: dialog.stage === "credentials"
        }
        TextField {
            id: username
            Layout.fillWidth: true
            visible: dialog.stage === "credentials"
            onAccepted: password.forceActiveFocus()
        }
        Label {
            text: dialog.t("login.password")
            visible: dialog.stage === "credentials"
        }
        TextField {
            id: password
            Layout.fillWidth: true
            visible: dialog.stage === "credentials"
            echoMode: TextInput.Password
            onAccepted: dialog.submit()
        }
        Label {
            text: dialog.t("login.code")
            visible: dialog.stage === "code"
        }
        TextField {
            id: code
            Layout.fillWidth: true
            visible: dialog.stage === "code"
            inputMethodHints: Qt.ImhDigitsOnly
            onAccepted: dialog.submit()
        }
        Label {
            text: dialog.t("login.data_password")
            visible: dialog.stage === "data_password"
        }
        TextField {
            id: dataPassword
            Layout.fillWidth: true
            visible: dialog.stage === "data_password"
            echoMode: TextInput.Password
            onAccepted: dialog.submit()
        }
        Label {
            id: errorLabel
            Layout.fillWidth: true
            visible: text.length > 0
            color: "#d23c3c"
            wrapMode: Text.Wrap
        }
        Label {
            Layout.fillWidth: true
            text: dialog.t("login.hint")
            wrapMode: Text.Wrap
            opacity: 0.7
        }
        Item { Layout.fillHeight: true }
        RowLayout {
            Layout.alignment: Qt.AlignRight
            BusyIndicator {
                running: dialog.busy
                visible: dialog.busy
                implicitHeight: 32
                implicitWidth: 32
            }
            Button {
                text: dialog.t("login.cancel")
                onClicked: dialog.close()
            }
            Button {
                text: dialog.t("login.submit")
                enabled: !dialog.busy
                highlighted: true
                onClicked: dialog.submit()
            }
        }
    }
}
