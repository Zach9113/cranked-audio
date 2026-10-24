// One knob in the "Knobs" style, drawn like the Cranked Audio window: a dial in the knob's colours with the level
// in the middle, what's playing, and the knob's button. Anything switched off in the settings simply isn't there,
// so the column shrinks to what's left.

import QtQuick
import QtQuick.Layouts
import QtQuick.Shapes
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
    required property bool showActionLabels
    required property bool buttonsInDial
    readonly property bool hasButton: knob.press !== "Nothing" || knob.hold !== "Nothing"
    // the button lives in the middle of the dial
    readonly property bool buttonInside: buttonsInDial && showButton && hasButton

    signal setLevel(int percent)
    signal step(int delta)
    signal pressButton()
    signal holdButton()

    // the level shown while dragging, so the dial follows the hand without waiting for the service
    property int dragLevel: -1
    readonly property int level: dragLevel >= 0 ? dragLevel : (knob.level === null ? -1 : knob.level)

    function mix(colour, under, amount) {
        const a = Qt.color(colour), b = Qt.color(under);
        return Qt.rgba(a.r * amount + b.r * (1 - amount), a.g * amount + b.g * (1 - amount), a.b * amount + b.b * (1 - amount), 1);
    }

    implicitWidth: unit * 6.4
    implicitHeight: content.implicitHeight + unit
    radius: unit * 0.8
    color: theme.card
    border.width: theme.border ? 1 : 0
    border.color: theme.border

    ColumnLayout {
        id: content
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.top: parent.top
        anchors.topMargin: column.unit * 0.5
        width: parent.width - column.unit * 0.8
        spacing: column.unit * 0.4

        PlasmaComponents.Label {
            visible: column.showName
            text: column.knob.name
            color: column.theme.text
            font.weight: Font.Bold
            font.pixelSize: column.unit * 0.85
            horizontalAlignment: Text.AlignHCenter
            elide: Text.ElideRight
            Layout.fillWidth: true
        }

        Item {
            Layout.alignment: Qt.AlignHCenter
            Layout.preferredWidth: column.unit * 5
            Layout.preferredHeight: column.unit * 5

            Shape {
                id: dialShape
                anchors.fill: parent
                preferredRendererType: Shape.CurveRenderer
                readonly property real radius: width / 2 - column.unit * 0.3
                readonly property real split: 270 * Math.max(0, column.level) / 100

                // the unfilled part of the ring, in the knob's background colour
                ShapePath {
                    strokeColor: column.knob.background.toLowerCase() === "#000000"
                        ? column.theme.track : column.mix(column.knob.background, column.theme.base, 0.55)
                    strokeWidth: column.unit * 0.45
                    fillColor: "transparent"
                    capStyle: ShapePath.FlatCap
                    PathAngleArc {
                        centerX: dialShape.width / 2
                        centerY: dialShape.height / 2
                        radiusX: dialShape.radius
                        radiusY: dialShape.radius
                        startAngle: 135 + dialShape.split
                        sweepAngle: 270 - dialShape.split
                    }
                }
                // the level, in the knob's colour
                ShapePath {
                    strokeColor: column.knob.muted ? column.mix(column.knob.muted_color, "#ffffff", 0.6)
                        : column.knob.lowered ? column.mix(column.knob.color, column.theme.base, 0.55)
                        : column.knob.color
                    strokeWidth: column.unit * 0.45
                    fillColor: "transparent"
                    capStyle: ShapePath.FlatCap
                    PathAngleArc {
                        centerX: dialShape.width / 2
                        centerY: dialShape.height / 2
                        radiusX: dialShape.radius
                        radiusY: dialShape.radius
                        startAngle: 135
                        sweepAngle: dialShape.split
                    }
                }
            }

            Rectangle {
                anchors.centerIn: parent
                width: parent.width * 0.66
                height: width
                radius: width / 2
                color: column.buttonInside
                    ? (column.knob.held || inside.pressed ? "#4a3818" : inside.containsMouse ? Qt.lighter(column.theme.centre, 1.15) : column.theme.centre)
                    : (dial.pressed ? Qt.lighter(column.theme.centre, 1.25) : dial.containsMouse ? Qt.lighter(column.theme.centre, 1.12) : column.theme.centre)
                border.width: column.buttonInside ? 1 : 0
                border.color: column.knob.held || inside.pressed ? "#e0a040" : column.theme.edge
            }

            ColumnLayout {
                anchors.centerIn: parent
                spacing: 0
                Kirigami.Icon {
                    visible: column.buttonInside
                    Layout.alignment: Qt.AlignHCenter
                    Layout.preferredWidth: column.unit * (column.showPercent ? 1.4 : 1.8)
                    Layout.preferredHeight: Layout.preferredWidth
                    source: column.icon(column.knob.press === "Nothing" ? column.knob.hold : column.knob.press)
                    color: column.knob.held ? "#ffd38a" : column.theme.text
                    isMask: true
                }
                PlasmaComponents.Label {
                    visible: column.showPercent
                    text: column.level < 0 ? "–" : column.level + "%"
                    color: column.knob.muted ? column.theme.subtle : column.theme.text
                    font.weight: Font.Bold
                    font.pixelSize: column.unit * (column.buttonInside ? 0.62 : 1.15)
                    Layout.alignment: Qt.AlignHCenter
                }
                PlasmaComponents.Label {
                    visible: column.knob.muted || column.knob.lowered
                    text: column.knob.muted ? "muted" : "lowered"
                    color: column.knob.muted ? "#e07a6a" : "#e0b050"
                    font.pixelSize: column.unit * 0.6
                    Layout.alignment: Qt.AlignHCenter
                }
            }

            // drag up/down or scroll to turn it (Shift = 5 at a time), like the window's knobs
            MouseArea {
                id: dial
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.SizeVerCursor
                property real startY
                property int startLevel
                onPressed: mouse => {
                    startY = mouse.y;
                    startLevel = Math.max(0, column.level);
                }
                onPositionChanged: mouse => {
                    if (!pressed) return;
                    const next = Math.max(0, Math.min(100, Math.round(startLevel + (startY - mouse.y) / 2)));
                    if (next !== column.dragLevel) {
                        column.dragLevel = next;
                        column.setLevel(next);
                    }
                }
                onReleased: dragEnd.restart()
                onWheel: wheel => column.step((wheel.angleDelta.y > 0 || wheel.angleDelta.x > 0 ? 1 : -1) * (wheel.modifiers & Qt.ShiftModifier ? 5 : 1))
            }
            // the middle of the dial, when it's the button: click to press, hold for the hold action
            MouseArea {
                id: inside
                enabled: column.buttonInside
                anchors.centerIn: parent
                width: parent.width * 0.66
                height: width
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                pressAndHoldInterval: 500
                property bool heldDown: false
                onPressed: heldDown = false
                onPressAndHold: {
                    heldDown = true;
                    column.holdButton();
                }
                onClicked: if (!heldDown) column.pressButton()
            }
            // keep the dragged value a moment so the dial doesn't jump back before the service catches up
            Timer {
                id: dragEnd
                interval: 600
                onTriggered: column.dragLevel = -1
            }
        }

        PlasmaComponents.Label {
            visible: column.showSource
            text: column.knob.source
            color: column.theme.subtle
            font.pixelSize: column.unit * 0.65
            horizontalAlignment: Text.AlignHCenter
            elide: Text.ElideRight
            Layout.fillWidth: true
        }

        Rectangle {
            visible: column.showButton && column.hasButton && !column.buttonInside
            Layout.alignment: Qt.AlignHCenter
            Layout.preferredWidth: column.unit * 3.4
            Layout.preferredHeight: column.unit * 3.4
            radius: column.unit * 0.7
            color: column.knob.held || button.pressed ? "#4a3818" : button.containsMouse ? Qt.lighter(column.theme.centre, 1.15) : column.theme.centre
            border.width: 1
            border.color: column.knob.held || button.pressed ? "#e0a040" : column.theme.edge

            Kirigami.Icon {
                anchors.centerIn: parent
                width: column.unit * 1.7
                height: width
                source: column.icon(column.knob.press === "Nothing" ? column.knob.hold : column.knob.press)
                color: column.knob.held ? "#ffd38a" : column.theme.text
                isMask: true
            }

            MouseArea {
                id: button
                anchors.fill: parent
                hoverEnabled: true
                pressAndHoldInterval: 500
                property bool heldDown: false
                onPressed: heldDown = false
                onPressAndHold: {
                    heldDown = true;
                    column.holdButton();
                }
                onClicked: if (!heldDown) column.pressButton()
            }
        }

        PlasmaComponents.Label {
            visible: column.showButton && column.showActionLabels
            text: "Press: " + column.knob.press + "\nHold: " + column.knob.hold
            color: column.theme.subtle
            font.pixelSize: column.unit * 0.6
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }
    }

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
