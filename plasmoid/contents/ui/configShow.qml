// Settings > What shows: which text, buttons and knobs.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.plasma.workspace.dbus as DBus

ConfigPage {
    id: page

    // knob names from the running service, for the knobs list
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
        QQC2.CheckBox { Kirigami.FormData.label: "Text:"; text: "Knob names"; checked: page.cfg_showName; onToggled: page.cfg_showName = checked }
        QQC2.CheckBox { text: "Percentages"; checked: page.cfg_showPercent; onToggled: page.cfg_showPercent = checked }
        QQC2.CheckBox { text: "What's playing"; checked: page.cfg_showSource; onToggled: page.cfg_showSource = checked }

        Item { Kirigami.FormData.isSection: true }

        QQC2.CheckBox { Kirigami.FormData.label: "Buttons:"; text: "Show the buttons"; checked: page.cfg_showButton; onToggled: page.cfg_showButton = checked }
        QQC2.CheckBox {
            text: "Inside the dials (more compact)"
            enabled: page.cfg_showButton && page.cfg_style === "knobs"
            checked: page.cfg_buttonsInDial
            onToggled: page.cfg_buttonsInDial = checked
        }
        QQC2.CheckBox {
            text: "What they do (press / hold)"
            enabled: page.cfg_showButton && !page.cfg_buttonsInDial
            checked: page.cfg_showActionLabels
            onToggled: page.cfg_showActionLabels = checked
        }

        Item { Kirigami.FormData.isSection: true }

        // one column, so the form lays it out as a single row however many knobs there are
        ColumnLayout {
            Kirigami.FormData.label: "Knobs shown:"
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

        QQC2.Label {
            visible: page.place === "panel"
            Layout.maximumWidth: Kirigami.Units.gridUnit * 22
            wrapMode: Text.WordWrap
            opacity: 0.7
            text: "In a thin panel the knobs are bare dials: names and what's playing are left out and the buttons go inside the dials."
        }
    }
}
