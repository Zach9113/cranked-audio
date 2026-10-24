// Cranked Audio panel widget: the knobs and buttons of the running Cranked Audio service, over its D-Bus API.
// It keeps no state of its own; everything goes through the service, with the same rules as the mixer.
// Settings (Appearance): style, theme, size, which text and which knobs show.

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

    readonly property var cfg: Plasmoid.configuration
    readonly property real unit: Kirigami.Units.gridUnit * cfg.scale / 100
    readonly property var hiddenKnobs: String(cfg.hiddenKnobs).split(",").map(s => parseInt(s)).filter(n => !isNaN(n))
    // the knobs to show, each with its index in the service
    readonly property var shownKnobs: running && snapshot
        ? snapshot.knobs.map((knob, index) => ({ knob: knob, index: index })).filter(k => hiddenKnobs.indexOf(k.index + 1) < 0)
        : []

    // colours per theme; "plasma" follows the desktop's colour scheme
    readonly property var theme: {
        const text = Kirigami.Theme.textColor;
        const faint = (a) => Qt.rgba(text.r, text.g, text.b, a);
        switch (cfg.theme) {
        case "cranked":
            return { base: "#1b1b21", card: "#1b1b21", centre: "#2a2a32", track: "#34343e", edge: "#3a3a44", text: "#f0f0f4", subtle: "#a8a8b4", border: "" };
        case "glass":
            return { base: "#202024", card: Qt.rgba(1, 1, 1, 0.05), centre: Qt.rgba(1, 1, 1, 0.08), track: Qt.rgba(1, 1, 1, 0.14),
                     edge: Qt.rgba(1, 1, 1, 0.18), text: "#ffffff", subtle: Qt.rgba(1, 1, 1, 0.65), border: Qt.rgba(1, 1, 1, 0.16) };
        case "minimal":
            return { base: Kirigami.Theme.backgroundColor, card: "transparent", centre: faint(0.06), track: faint(0.14), edge: faint(0.15), text: text, subtle: faint(0.6), border: "" };
        default:
            return { base: Kirigami.Theme.backgroundColor, card: faint(0.05), centre: faint(0.09), track: faint(0.16), edge: faint(0.15), text: text, subtle: faint(0.65), border: "" };
        }
    }

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

    // quick while the popup is open, slow otherwise (just enough for the tooltip)
    Timer {
        interval: root.expanded || Plasmoid.formFactor === PlasmaCore.Types.Planar ? 250 : 5000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: root.refresh()
    }

    Plasmoid.icon: "io.github.zach9113.CrankedAudio"
    Plasmoid.title: "Cranked Audio"
    // the frame Plasma draws behind a widget on the desktop; off lets the knobs float on the wallpaper
    Plasmoid.backgroundHints: cfg.showBackground ? PlasmaCore.Types.DefaultBackground : PlasmaCore.Types.NoBackground
    toolTipMainText: "Cranked Audio"
    toolTipSubText: {
        if (!running || !snapshot) return "Not running";
        const levels = snapshot.knobs.map(k => k.name + " " + (k.level === null ? "–" : k.level + "%") + (k.muted ? " (muted)" : ""));
        return levels.join("\n") + "\nProfile: " + snapshot.profile;
    }

    compactRepresentation: MouseArea {
        hoverEnabled: true
        acceptedButtons: Qt.LeftButton | Qt.MiddleButton
        onClicked: mouse => {
            if (mouse.button === Qt.MiddleButton) {
                root.press(root.cfg.wheelKnob - 1);
            } else {
                root.expanded = !root.expanded;
            }
        }
        // scrolling over the panel icon turns the chosen knob, like the volume applet
        onWheel: wheel => root.stepKnob(root.cfg.wheelKnob - 1, wheel.angleDelta.y > 0 ? 2 : -2)
        Kirigami.Icon {
            anchors.fill: parent
            source: Plasmoid.icon
            active: parent.containsMouse
        }
    }

    fullRepresentation: PlasmaExtras.Representation {
        id: full
        readonly property real pad: root.unit * 0.6
        Layout.preferredWidth: content.implicitWidth + pad * 2
        Layout.preferredHeight: content.implicitHeight + pad * 2 + (header && header.visible ? header.implicitHeight : 0)
        Layout.minimumWidth: Math.min(Layout.preferredWidth, root.unit * 8)
        collapseMarginsHint: true

        header: PlasmaExtras.PlasmoidHeading {
            visible: root.cfg.showHeader
            RowLayout {
                anchors.fill: parent
                PlasmaExtras.Heading {
                    level: 4
                    text: "Cranked Audio"
                    Layout.fillWidth: true
                }
                PlasmaComponents.ToolButton {
                    visible: !!root.snapshot && root.snapshot.profiles.length > 1
                    text: root.snapshot ? root.snapshot.profile : ""
                    icon.name: "view-list-details"
                    PlasmaComponents.ToolTip.text: "Switch profile"
                    PlasmaComponents.ToolTip.visible: hovered
                    onClicked: profileMenu.popup()
                    PlasmaComponents.Menu {
                        id: profileMenu
                        Instantiator {
                            model: root.snapshot ? root.snapshot.profiles : []
                            delegate: PlasmaComponents.MenuItem {
                                required property string modelData
                                text: modelData
                                checkable: true
                                checked: root.snapshot && root.snapshot.profile === modelData
                                onTriggered: root.call("SetProfile", [new DBus.string(modelData)], "(s)", root.refresh)
                            }
                            onObjectAdded: (index, object) => profileMenu.insertItem(index, object)
                            onObjectRemoved: (index, object) => profileMenu.removeItem(object)
                        }
                    }
                }
                PlasmaComponents.ToolButton {
                    icon.name: "window-new"
                    display: PlasmaComponents.AbstractButton.IconOnly
                    text: "Open Cranked Audio"
                    PlasmaComponents.ToolTip.text: text
                    PlasmaComponents.ToolTip.visible: hovered
                    onClicked: root.call("OpenWindow", [], "", null)
                }
            }
        }

        Item {
            id: content
            anchors.fill: parent
            anchors.margins: full.pad
            // all knobs side by side when there's room; the Flow wraps them when there isn't
            readonly property real allColumns: root.shownKnobs.length * (root.unit * 6.4 + root.unit * 0.5) - root.unit * 0.5
            readonly property real allFaders: root.shownKnobs.length * (root.unit * 4.2 + root.unit * 0.4) - root.unit * 0.4
            implicitWidth: placeholder.visible ? root.unit * 16
                : root.cfg.style === "sliders" ? sliders.implicitWidth
                : root.cfg.style === "faders" ? allFaders : allColumns
            implicitHeight: placeholder.visible ? placeholder.implicitHeight
                : root.cfg.style === "sliders" ? sliders.implicitHeight
                : root.cfg.style === "faders" ? faders.implicitHeight : knobs.implicitHeight

            PlasmaExtras.PlaceholderMessage {
                id: placeholder
                anchors.centerIn: parent
                width: parent.width
                visible: !root.running || !root.snapshot
                iconName: "io.github.zach9113.CrankedAudio"
                text: "Cranked Audio isn't running"
                explanation: "Start it from the app menu."
            }

            // "Knobs": columns like the Cranked Audio window
            // columns wrap onto more rows when the popup is narrower than all of them
            Flow {
                id: knobs
                visible: !placeholder.visible && root.cfg.style === "knobs"
                width: parent.width
                spacing: root.unit * 0.5

                Repeater {
                    model: root.cfg.style === "knobs" ? root.shownKnobs : []
                    delegate: KnobColumn {
                        required property var modelData
                        knob: modelData.knob
                        index: modelData.index
                        unit: root.unit
                        theme: root.theme
                        showName: root.cfg.showName
                        showPercent: root.cfg.showPercent
                        showSource: root.cfg.showSource
                        showButton: root.cfg.showButton
                        showActionLabels: root.cfg.showActionLabels
                        buttonsInDial: root.cfg.buttonsInDial
                        onSetLevel: percent => root.setKnob(modelData.index, percent)
                        onStep: delta => root.stepKnob(modelData.index, delta)
                        onPressButton: root.press(modelData.index)
                        onHoldButton: root.hold(modelData.index)
                    }
                }
            }

            // "Faders": vertical faders like a mixing desk
            Flow {
                id: faders
                visible: !placeholder.visible && root.cfg.style === "faders"
                width: parent.width
                spacing: root.unit * 0.4
                Repeater {
                    model: root.cfg.style === "faders" ? root.shownKnobs : []
                    delegate: FaderColumn {
                        required property var modelData
                        knob: modelData.knob
                        index: modelData.index
                        unit: root.unit
                        theme: root.theme
                        showName: root.cfg.showName
                        showPercent: root.cfg.showPercent
                        showSource: root.cfg.showSource
                        showButton: root.cfg.showButton
                        onSetLevel: percent => root.setKnob(modelData.index, percent)
                        onPressButton: root.press(modelData.index)
                        onHoldButton: root.hold(modelData.index)
                    }
                }
            }

            // "Sliders": a compact list
            ColumnLayout {
                id: sliders
                visible: !placeholder.visible && root.cfg.style === "sliders"
                width: parent.width
                spacing: root.unit * 0.5
                Repeater {
                    model: root.cfg.style === "sliders" ? root.shownKnobs : []
                    delegate: RowLayout {
                        id: row
                        required property var modelData
                        readonly property var knob: modelData.knob
                        Layout.fillWidth: true
                        spacing: root.unit * 0.4

                        Rectangle {
                            Layout.preferredWidth: root.unit * 0.25
                            Layout.fillHeight: true
                            radius: width / 2
                            color: row.knob.muted ? row.knob.muted_color : row.knob.color
                        }
                        ColumnLayout {
                            visible: root.cfg.showName || root.cfg.showSource
                            Layout.preferredWidth: root.unit * 6
                            spacing: 0
                            PlasmaComponents.Label {
                                visible: root.cfg.showName
                                text: row.knob.name
                                color: root.theme.text
                                font.weight: Font.Bold
                                font.pixelSize: root.unit * 0.8
                                elide: Text.ElideRight
                                Layout.fillWidth: true
                            }
                            PlasmaComponents.Label {
                                visible: root.cfg.showSource
                                text: row.knob.muted ? "muted" : row.knob.lowered ? row.knob.source + " · lowered" : row.knob.source
                                color: root.theme.subtle
                                font.pixelSize: root.unit * 0.65
                                elide: Text.ElideRight
                                Layout.fillWidth: true
                            }
                        }
                        PlasmaComponents.Slider {
                            id: slider
                            Layout.fillWidth: true
                            Layout.minimumWidth: root.unit * 6
                            from: 0
                            to: 100
                            stepSize: 1
                            opacity: row.knob.muted ? 0.5 : 1
                            // follow the service, except while the user is dragging
                            Binding on value {
                                when: !slider.pressed
                                value: row.knob.level === null ? 0 : row.knob.level
                            }
                            onMoved: root.setKnob(row.modelData.index, value)
                        }
                        PlasmaComponents.Label {
                            visible: root.cfg.showPercent
                            text: row.knob.level === null ? "–" : Math.round(slider.value) + "%"
                            color: root.theme.text
                            font.pixelSize: root.unit * 0.8
                            Layout.preferredWidth: root.unit * 2.2
                            horizontalAlignment: Text.AlignRight
                        }
                        PlasmaComponents.ToolButton {
                            visible: root.cfg.showButton && (row.knob.press !== "Nothing" || row.knob.hold !== "Nothing")
                            icon.name: knobIcons.icon(row.knob.press === "Nothing" ? row.knob.hold : row.knob.press)
                            checked: row.knob.held
                            PlasmaComponents.ToolTip.text: "Press: " + row.knob.press + "\nHold: " + row.knob.hold
                            PlasmaComponents.ToolTip.visible: hovered
                            onClicked: root.press(row.modelData.index)
                            onPressAndHold: root.hold(row.modelData.index)
                        }
                    }
                }
            }
        }
    }

    // shared icon names for button actions
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
