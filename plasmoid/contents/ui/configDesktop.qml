// Settings > Desktop & popup: the title row and the background.

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

ConfigPage {
    id: page

    Kirigami.FormLayout {
        QQC2.Label {
            visible: page.placeText !== ""
            text: page.placeText + (page.place === "panel" && page.cfg_showInPanel
                ? " These settings apply on the desktop and in the popup." : "")
            opacity: 0.7
        }

        QQC2.CheckBox {
            Kirigami.FormData.label: "Title row:"
            text: "Title and profile switcher"
            checked: page.cfg_showHeader
            onToggled: page.cfg_showHeader = checked
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
    }
}
