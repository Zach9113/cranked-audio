// One knob in the "Faders" style: a vertical fader like a mixing desk, with the knob's colour, level and button.

import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.plasma.components as PlasmaComponents

Rectangle {
    id: column

    required property var knob
    required property int index
    required property real unit
    required property var theme
    required property bool showName
    required property bool showPercent
    required property bool showSource
    required property bool showButton
    property bool showCardBackground: true
    readonly property bool hasButton: knob.press !== "Nothing" || knob.hold !== "Nothing"

    signal setLevel(int percent)
    signal pressButton()
    signal holdButton()

    implicitWidth: unit * 4.2
    implicitHeight: content.implicitHeight + unit
    radius: unit * 0.7
    color: showCardBackground ? theme.card : "transparent"
    border.width: showCardBackground && theme.border ? 1 : 0
    border.color: theme.border

    ColumnLayout {
        id: content
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.top: parent.top
        anchors.topMargin: column.unit * 0.5
        width: parent.width - column.unit * 0.6
        spacing: column.unit * 0.35

        PlasmaComponents.Label {
            visible: column.showName
            text: column.knob.name
            color: column.theme.text
            font.weight: Font.Bold
            font.pixelSize: column.unit * 0.7
            horizontalAlignment: Text.AlignHCenter
            elide: Text.ElideRight
            Layout.fillWidth: true
        }

        RowLayout {
            Layout.alignment: Qt.AlignHCenter
            spacing: column.unit * 0.25
            // the knob's colour, like its LEDs
            Rectangle {
                Layout.preferredWidth: column.unit * 0.2
                Layout.fillHeight: true
                radius: width / 2
                color: column.knob.muted ? column.knob.muted_color : column.knob.color
            }
            PlasmaComponents.Slider {
                id: fader
                orientation: Qt.Vertical
                Layout.preferredHeight: column.unit * 8
                from: 0
                to: 100
                stepSize: 1
                opacity: column.knob.muted ? 0.5 : 1
                Binding on value {
                    when: !fader.pressed
                    value: column.knob.level === null ? 0 : column.knob.level
                }
                onMoved: column.setLevel(Math.round(value))
            }
        }

        PlasmaComponents.Label {
            visible: column.showPercent
            text: column.knob.level === null ? "–" : Math.round(fader.value) + "%"
            color: column.theme.text
            font.weight: Font.Bold
            font.pixelSize: column.unit * 0.75
            Layout.alignment: Qt.AlignHCenter
        }

        PlasmaComponents.Label {
            visible: column.showSource
            text: column.knob.muted ? "muted" : column.knob.lowered ? "lowered" : column.knob.source
            color: column.knob.muted ? "#e07a6a" : column.knob.lowered ? "#e0b050" : column.theme.subtle
            font.pixelSize: column.unit * 0.55
            horizontalAlignment: Text.AlignHCenter
            elide: Text.ElideRight
            Layout.fillWidth: true
        }

        PlasmaComponents.ToolButton {
            visible: column.showButton && column.hasButton
            Layout.alignment: Qt.AlignHCenter
            icon.name: knobIcons.icon(column.knob.press === "Nothing" ? column.knob.hold : column.knob.press)
            icon.width: column.unit * 1.2
            icon.height: column.unit * 1.2
            checked: column.knob.held
            PlasmaComponents.ToolTip.text: "Press: " + column.knob.press + "\nHold: " + column.knob.hold
            PlasmaComponents.ToolTip.visible: hovered
            onClicked: column.pressButton()
            onPressAndHold: column.holdButton()
        }
    }

    QtObject {
        id: knobIcons
        function icon(label) {
            if (label === "Play/Pause") return "media-playback-start";
            if (label === "Next") return "media-skip-forward";
            if (label === "Previous") return "media-skip-backward";
            if (label === "Mute") return "audio-volume-muted";
            if (label === "Switch output") return "audio-speakers";
            if (label === "Next profile") return "view-list-details";
            if (label === "Nothing") return "";
            return "system-shutdown";
        }
    }
}
