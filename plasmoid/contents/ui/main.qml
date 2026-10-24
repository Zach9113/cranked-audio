// Cranked Audio panel widget: the knobs and buttons of the running Cranked Audio service, over its D-Bus API.
// It keeps no state of its own; everything goes through the service, with the same rules as the mixer.

import QtQuick
import QtQuick.Layouts
import org.kde.plasma.plasmoid
import org.kde.plasma.core as PlasmaCore
import org.kde.plasma.components as PlasmaComponents
import org.kde.plasma.extras as PlasmaExtras
import org.kde.kirigami as Kirigami
import org.kde.plasma.workspace.dbus as DBus

PlasmoidItem {
    id: root

    readonly property var api: ({
        service: "io.github.zach9113.CrankedAudio",
        path: "/io/github/zach9113/CrankedAudio",
        iface: "io.github.zach9113.CrankedAudio1"
    })
    // the service's snapshot (see cranked-ipc): mixer, profile, profiles, knobs[...]
    property var snapshot: null
    property bool running: false

    function call(member, args, signature, done) {
        const message = Object.assign({ member: member, arguments: args || [], signature: signature || "" }, api);
        DBus.SessionBus.asyncCall(message, reply => {
            root.running = true;
            if (done) done(reply.value);
        }, error => {
            root.running = false;
        });
    }

    function refresh() {
        call("GetState", [], "", value => {
            try {
                root.snapshot = JSON.parse(String(value));
            } catch (e) {
                root.snapshot = null;
            }
        });
    }

    function setKnob(index, percent) {
        call("SetKnob", [new DBus.uint32(index), new DBus.uint32(Math.round(percent))], "(uu)", refresh);
    }

    function stepKnob(index, delta) {
        call("StepKnob", [new DBus.uint32(index), new DBus.int32(delta)], "(ui)", refresh);
    }

    function press(index) {
        call("Press", [new DBus.uint32(index)], "(u)", refresh);
    }

    function hold(index) {
        call("Hold", [new DBus.uint32(index)], "(u)", refresh);
    }

    // the same icons the window uses for each press action
    function actionIcon(label) {
        if (label === "Play/Pause") return "media-playback-start";
        if (label === "Next") return "media-skip-forward";
        if (label === "Previous") return "media-skip-backward";
        if (label === "Mute") return "audio-volume-muted";
        if (label === "Switch output") return "audio-card";
        if (label === "Next profile") return "view-list-details";
        if (label === "Nothing") return "";
        return "system-shutdown";
    }

    // quick while the popup is open, slow otherwise (just enough for the tooltip)
    Timer {
        interval: root.expanded ? 250 : 5000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: root.refresh()
    }

    Plasmoid.icon: "io.github.zach9113.CrankedAudio"
    Plasmoid.title: "Cranked Audio"
    toolTipMainText: "Cranked Audio"
    toolTipSubText: {
        if (!running || !snapshot) return "Not running";
        const levels = snapshot.knobs.map(k => k.name + " " + (k.level === null ? "–" : k.level + "%") + (k.muted ? " (muted)" : ""));
        return levels.join("\n") + "\nProfile: " + snapshot.profile;
    }

    compactRepresentation: MouseArea {
        acceptedButtons: Qt.LeftButton | Qt.MiddleButton
        onClicked: mouse => {
            if (mouse.button === Qt.MiddleButton) {
                root.press(0);
            } else {
                root.expanded = !root.expanded;
            }
        }
        // scrolling over the panel icon turns the first knob, like the volume applet
        onWheel: wheel => root.stepKnob(0, wheel.angleDelta.y > 0 ? 2 : -2)
        Kirigami.Icon {
            anchors.fill: parent
            source: Plasmoid.icon
            active: parent.containsMouse
        }
        hoverEnabled: true
    }

    fullRepresentation: PlasmaExtras.Representation {
        Layout.preferredWidth: Kirigami.Units.gridUnit * 24
        Layout.preferredHeight: column.implicitHeight + Kirigami.Units.largeSpacing * 2
        Layout.minimumWidth: Kirigami.Units.gridUnit * 18
        collapseMarginsHint: true

        header: PlasmaExtras.PlasmoidHeading {
            RowLayout {
                anchors.fill: parent
                PlasmaExtras.Heading {
                    level: 3
                    text: "Cranked Audio"
                    Layout.fillWidth: true
                }
                PlasmaComponents.ComboBox {
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 8
                    visible: !!root.snapshot && root.snapshot.profiles.length > 1
                    model: root.snapshot ? root.snapshot.profiles : []
                    currentIndex: root.snapshot ? root.snapshot.profiles.indexOf(root.snapshot.profile) : -1
                    onActivated: index => root.call("SetProfile", [new DBus.string(root.snapshot.profiles[index])], "(s)", root.refresh)
                }
                PlasmaComponents.ToolButton {
                    icon.name: "window-new"
                    text: "Open"
                    display: PlasmaComponents.AbstractButton.IconOnly
                    PlasmaComponents.ToolTip.text: "Open the Cranked Audio window"
                    PlasmaComponents.ToolTip.visible: hovered
                    onClicked: root.call("OpenWindow", [], "", null)
                }
            }
        }

        ColumnLayout {
            id: column
            anchors.fill: parent
            anchors.margins: Kirigami.Units.largeSpacing
            spacing: Kirigami.Units.largeSpacing

            PlasmaExtras.PlaceholderMessage {
                visible: !root.running || !root.snapshot
                Layout.fillWidth: true
                iconName: "io.github.zach9113.CrankedAudio"
                text: "Cranked Audio isn't running"
                explanation: "Start it from the app menu."
            }

            Repeater {
                model: root.running && root.snapshot ? root.snapshot.knobs : []
                delegate: RowLayout {
                    id: knobRow
                    required property var modelData
                    required property int index
                    Layout.fillWidth: true
                    spacing: Kirigami.Units.smallSpacing * 2

                    // the knob's colour, like its LEDs
                    Rectangle {
                        Layout.preferredWidth: Kirigami.Units.smallSpacing
                        Layout.fillHeight: true
                        radius: width / 2
                        color: knobRow.modelData.muted ? knobRow.modelData.muted_color : knobRow.modelData.color
                    }

                    ColumnLayout {
                        Layout.preferredWidth: Kirigami.Units.gridUnit * 7
                        spacing: 0
                        PlasmaComponents.Label {
                            text: knobRow.modelData.name
                            font.weight: Font.Bold
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }
                        PlasmaComponents.Label {
                            text: knobRow.modelData.muted ? "muted"
                                : knobRow.modelData.lowered ? knobRow.modelData.source + " · lowered"
                                : knobRow.modelData.source
                            opacity: 0.7
                            font: Kirigami.Theme.smallFont
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }
                    }

                    PlasmaComponents.Slider {
                        id: slider
                        Layout.fillWidth: true
                        from: 0
                        to: 100
                        stepSize: 1
                        opacity: knobRow.modelData.muted ? 0.5 : 1
                        // follow the service, except while the user is dragging
                        Binding on value {
                            when: !slider.pressed
                            value: knobRow.modelData.level === null ? 0 : knobRow.modelData.level
                        }
                        onMoved: root.setKnob(knobRow.index, value)
                    }

                    PlasmaComponents.Label {
                        text: knobRow.modelData.level === null ? "–" : Math.round(slider.value) + "%"
                        Layout.preferredWidth: Kirigami.Units.gridUnit * 2.2
                        horizontalAlignment: Text.AlignRight
                    }

                    PlasmaComponents.ToolButton {
                        icon.name: root.actionIcon(knobRow.modelData.press)
                        visible: knobRow.modelData.press !== "Nothing" || knobRow.modelData.hold !== "Nothing"
                        checked: knobRow.modelData.held
                        PlasmaComponents.ToolTip.text: "Press: " + knobRow.modelData.press + "\nHold: " + knobRow.modelData.hold
                        PlasmaComponents.ToolTip.visible: hovered
                        onClicked: root.press(knobRow.index)
                        onPressAndHold: root.hold(knobRow.index)
                    }
                }
            }
        }
    }
}
