// The same rehearsal in QtQuick, on Quickshell: a card that opens with a
// spring and, right as it starts, 600 ms of JavaScript that does not yield the thread.
import QtQuick
import Quickshell

ShellRoot {
    PanelWindow {
        id: panel
        screen: Quickshell.screens.find(s => s.name === "HDMI-A-1") ?? Quickshell.screens[0]
        anchors.top: true
        margins.top: 380
        implicitWidth: 720
        implicitHeight: 230
        color: "transparent"
        exclusiveZone: 0

        property bool open: false
        property real maxGap: 0
        property int frames: 0

        Rectangle {
            id: orb
            width: 56; height: 56; radius: 28; color: "#151616"
            y: 62
            x: panel.open ? 112 : 332
            Behavior on x { SpringAnimation { spring: 3.2; damping: 0.32; mass: 1 } }
            Row {
                anchors.centerIn: parent; spacing: 10
                Repeater { model: 2; Rectangle { width: 7; height: 17; radius: 3.5; color: "#f5f7f5" } }
            }
        }
        Rectangle {
            x: orb.x + 90; y: 20; radius: 24; color: "#151616"
            width: panel.open ? 406 : 0
            height: panel.open ? 190 : 0
            Behavior on width { SpringAnimation { spring: 2.8; damping: 0.36 } }
            Behavior on height { SpringAnimation { spring: 2.8; damping: 0.36 } }
        }

        FrameAnimation {
            running: true
            onTriggered: {
                panel.frames++
                if (frameTime > panel.maxGap) panel.maxGap = frameTime
            }
        }
        Timer {
            interval: 2600; running: true; repeat: true; triggeredOnStart: false
            onTriggered: {
                if (panel.frames > 0)
                    console.log("qml · cycle · " + panel.frames + " frames · max gap "
                        + (panel.maxGap * 1000).toFixed(1) + " ms")
                panel.frames = 0; panel.maxGap = 0
                panel.open = !panel.open
                const end = Date.now() + 600
                while (Date.now() < end) {}
            }
        }
        Timer { interval: 9000; running: true; onTriggered: Qt.quit() }
    }
}
