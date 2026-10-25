// Settings > Look: style, theme and size, wherever the widget is.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

ConfigPage {
    id: page

    // in a panel showing knobs, the panel's height sets the size
    readonly property bool panelSized: place === "panel" && cfg_showInPanel

    Kirigami.FormLayout {
        QQC2.Label {
            visible: page.placeText !== ""
            text: page.placeText
            opacity: 0.7
        }

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
                { value: "minimal", text: "Minimal (no cards)" }
            ]
            textRole: "text"
            valueRole: "value"
            currentIndex: model.findIndex(m => m.value === page.cfg_theme)
            onActivated: page.cfg_theme = currentValue
        }

        QQC2.CheckBox {
            Kirigami.FormData.label: "Backgrounds:"
            text: "Widget background"
            checked: page.cfg_showBackground
            onToggled: page.cfg_showBackground = checked
        }
        QQC2.CheckBox {
            text: "Card backgrounds behind knobs"
            checked: page.cfg_showCardBackground
            onToggled: page.cfg_showCardBackground = checked
        }

        RowLayout {
            Kirigami.FormData.label: "Size:"
            enabled: !page.panelSized
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
        QQC2.Label {
            visible: page.panelSized
            Layout.maximumWidth: Kirigami.Units.gridUnit * 22
            wrapMode: Text.WordWrap
            opacity: 0.7
            text: "In a panel the knobs fit the panel's height: change the panel's height to resize them."
        }
    }
}
