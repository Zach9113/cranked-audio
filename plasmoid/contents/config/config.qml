import QtQuick
import org.kde.plasma.configuration

ConfigModel {
    ConfigCategory {
        name: "Look"
        icon: "preferences-desktop-theme"
        source: "configLook.qml"
    }
    ConfigCategory {
        name: "What shows"
        icon: "view-visible"
        source: "configShow.qml"
    }
    ConfigCategory {
        name: "In a panel"
        icon: "configure-toolbars"
        source: "configPanel.qml"
    }
    ConfigCategory {
        name: "Desktop & popup"
        icon: "preferences-desktop"
        source: "configDesktop.qml"
    }
}
