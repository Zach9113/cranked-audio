// Widget settings: look, size, what shows. Every change previews right away in the widget.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.kcmutils as KCM
import org.kde.plasma.workspace.dbus as DBus

KCM.SimpleKCM {
    id: page

    property string cfg_style
    property int cfg_scale
    property string cfg_theme
    property bool cfg_showName
    property bool cfg_showPercent
    property bool cfg_showSource
    property bool cfg_showButton
    property bool cfg_showActionLabels
    property bool cfg_showHeader
    property bool cfg_showBackground
    property bool cfg_showInPanel
    property bool cfg_buttonsInDial
    property string cfg_hiddenKnobs
    property int cfg_wheelKnob

    // knob names from the running service, for the "Show these knobs" list
    property var knobNames: []
    Component.onCompleted: DBus.SessionBus.asyncCall({
        service: "io.github.zach9113.CrankedAudio",
        path: "/io/github/zach9113/CrankedAudio",
        iface: "io.github.zach9113.CrankedAudio1",
        member: "GetState",
        arguments: [],
        signature: ""
    }, reply => {
        try {
            page.knobNames = JSON.parse(String(reply.value)).knobs.map(k => k.name);
        } catch (e) {
        }
    }, () => {})

    function hidden() {
        return cfg_hiddenKnobs.split(",").map(s => parseInt(s)).filter(n => !isNaN(n));
    }

    function setShown(number, shown) {
        let list = hidden().filter(n => n !== number);
        if (!shown) list.push(number);
        cfg_hiddenKnobs = list.sort((a, b) => a - b).join(",");
    }

    Kirigami.FormLayout {
        QQC2.ComboBox {
            Kirigami.FormData.label: "Style:"
            model: [
                { value: "knobs", text: "Knobs (like the app)" },
                { value: "sliders", text: "Sliders (compact list)" },
                { value: "faders", text: "Faders (like a mixing desk)" }
            ]
            textRole: "text"
            valueRole: "value"
            // found in the model directly (indexOfValue isn't ready when the page opens)
            currentIndex: model.findIndex(m => m.value === page.cfg_style)
            onActivated: page.cfg_style = currentValue
        }

        QQC2.ComboBox {
            Kirigami.FormData.label: "Theme:"
            model: [
                { value: "plasma", text: "Match Plasma" },
                { value: "cranked", text: "Cranked (dark, like the app)" },
                { value: "glass", text: "Glass" },
                { value: "minimal", text: "Minimal" }
            ]
            textRole: "text"
            valueRole: "value"
            currentIndex: model.findIndex(m => m.value === page.cfg_theme)
            onActivated: page.cfg_theme = currentValue
        }

        RowLayout {
            Kirigami.FormData.label: "Size:"
            QQC2.Slider {
                from: 30
                to: 160
                stepSize: 5
                value: page.cfg_scale
                onMoved: page.cfg_scale = value
                Layout.preferredWidth: Kirigami.Units.gridUnit * 12
            }
            QQC2.Label { text: page.cfg_scale + "%" }
        }

        Item { Kirigami.FormData.isSection: true }

        QQC2.CheckBox { Kirigami.FormData.label: "Show:"; text: "Knob names"; checked: page.cfg_showName; onToggled: page.cfg_showName = checked }
        QQC2.CheckBox { text: "Percentages"; checked: page.cfg_showPercent; onToggled: page.cfg_showPercent = checked }
        QQC2.CheckBox { text: "What's playing"; checked: page.cfg_showSource; onToggled: page.cfg_showSource = checked }
        QQC2.CheckBox { text: "Buttons"; checked: page.cfg_showButton; onToggled: page.cfg_showButton = checked }
        QQC2.CheckBox {
            text: "What the buttons do (press / hold)"
            enabled: page.cfg_showButton
            checked: page.cfg_showActionLabels
            onToggled: page.cfg_showActionLabels = checked
        }
        QQC2.CheckBox { text: "Title and profile switcher"; checked: page.cfg_showHeader; onToggled: page.cfg_showHeader = checked }
        QQC2.CheckBox {
            text: "In a panel: show the knobs right in the panel (not an icon)"
            checked: page.cfg_showInPanel
            onToggled: page.cfg_showInPanel = checked
        }
        QQC2.CheckBox {
            text: "Background behind the widget (on the desktop)"
            checked: page.cfg_showBackground
            onToggled: page.cfg_showBackground = checked
        }
        QQC2.CheckBox {
            text: "Buttons inside the dials (more compact)"
            enabled: page.cfg_showButton && page.cfg_style === "knobs"
            checked: page.cfg_buttonsInDial
            onToggled: page.cfg_buttonsInDial = checked
        }

        Item { Kirigami.FormData.isSection: true }

        // one column, so the form lays it out as a single row however many knobs there are
        ColumnLayout {
            Kirigami.FormData.label: "Knobs:"
            spacing: 0
            Repeater {
                model: page.knobNames
                delegate: QQC2.CheckBox {
                    required property string modelData
                    required property int index
                    text: modelData
                    checked: page.hidden().indexOf(index + 1) < 0
                    onToggled: page.setShown(index + 1, checked)
                }
            }
            QQC2.Label {
                visible: page.knobNames.length === 0
                text: "Start Cranked Audio to choose which knobs show."
                opacity: 0.7
            }
        }

        QQC2.SpinBox {
            Kirigami.FormData.label: "Scrolling on the icon turns knob:"
            from: 1
            to: Math.max(1, page.knobNames.length)
            value: page.cfg_wheelKnob
            onValueModified: page.cfg_wheelKnob = value
        }

        QQC2.Label {
            Layout.maximumWidth: Kirigami.Units.gridUnit * 22
            wrapMode: Text.WordWrap
            opacity: 0.7
            text: "To open the widget with a key, set a shortcut on the Keyboard Shortcuts page (for example Meta+K). Put it in its own panel or on the desktop to use it on its own."
        }
    }
}
