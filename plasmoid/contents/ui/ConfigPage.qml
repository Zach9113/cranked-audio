// Base for the widget's settings pages: every setting (Plasma hands each page all of them) and where the
// widget is, so a page can say which of its settings apply here.

import QtQuick
import org.kde.kcmutils as KCM
import org.kde.plasma.plasmoid
import org.kde.plasma.core as PlasmaCore

KCM.SimpleKCM {
    property string cfg_style
    property string cfg_styleDefault
    property int cfg_scale
    property int cfg_scaleDefault
    property string cfg_theme
    property string cfg_themeDefault
    property bool cfg_showName
    property bool cfg_showNameDefault
    property bool cfg_showPercent
    property bool cfg_showPercentDefault
    property bool cfg_showSource
    property bool cfg_showSourceDefault
    property bool cfg_showButton
    property bool cfg_showButtonDefault
    property bool cfg_showActionLabels
    property bool cfg_showActionLabelsDefault
    property bool cfg_showHeader
    property bool cfg_showHeaderDefault
    property bool cfg_showBackground
    property bool cfg_showBackgroundDefault
    property bool cfg_showInPanel
    property bool cfg_showInPanelDefault
    property bool cfg_buttonsInDial
    property bool cfg_buttonsInDialDefault
    property string cfg_hiddenKnobs
    property string cfg_hiddenKnobsDefault
    property int cfg_wheelKnob
    property int cfg_wheelKnobDefault

    // where this widget is: "panel", "desktop" or "" (unknown)
    readonly property string place: {
        const form = Plasmoid.formFactor;
        if (form === PlasmaCore.Types.Horizontal || form === PlasmaCore.Types.Vertical) return "panel";
        if (form === PlasmaCore.Types.Planar || form === PlasmaCore.Types.MediaCenter) return "desktop";
        return "";
    }
    readonly property string placeText: place === "panel" ? "This widget is in a panel."
        : place === "desktop" ? "This widget is on the desktop." : ""
}
