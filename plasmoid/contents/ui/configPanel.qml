// Settings > In a panel: knobs or an icon, and what the icon does.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

ConfigPage {
    id: page

    Kirigami.FormLayout {
        QQC2.Label {
            visible: page.placeText !== ""
            text: page.placeText + (page.place === "desktop" ? " These settings apply when it's in a panel." : "")
            opacity: 0.7
        }

        QQC2.RadioButton {
            Kirigami.FormData.label: "In a panel, show:"
            text: "The knobs"
            checked: page.cfg_showInPanel
            onToggled: if (checked) page.cfg_showInPanel = true
        }
        QQC2.Label {
            Layout.maximumWidth: Kirigami.Units.gridUnit * 22
            wrapMode: Text.WordWrap
            opacity: 0.7
            text: "They fit the panel's height: a tall panel shows full cards, a thin one bare dials."
        }
        QQC2.RadioButton {
            text: "An icon that opens them in a popup"
            checked: !page.cfg_showInPanel
            onToggled: if (checked) page.cfg_showInPanel = false
        }

        QQC2.SpinBox {
            Kirigami.FormData.label: "Scrolling on the icon turns knob:"
            enabled: !page.cfg_showInPanel
            from: 1
            to: 16
            value: page.cfg_wheelKnob
            onValueModified: page.cfg_wheelKnob = value
        }

        Item { Kirigami.FormData.isSection: true }

        QQC2.Label {
            Layout.maximumWidth: Kirigami.Units.gridUnit * 22
            wrapMode: Text.WordWrap
            opacity: 0.7
            text: "To hide the knobs until you need them, give them their own panel and set its Visibility to Auto hide or Dodge windows. To open the popup with a key, set a shortcut on the Keyboard Shortcuts page."
        }
    }
}
