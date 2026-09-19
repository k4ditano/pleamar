// El mismo ensayo en QtQuick, sobre Quickshell: una tarjeta que se abre con
// muelle y, justo al empezar, 600 ms de JavaScript que no cede el hilo.
import QtQuick
import Quickshell

ShellRoot {
    PanelWindow {
        id: ventana
        screen: Quickshell.screens.find(s => s.name === "HDMI-A-1") ?? Quickshell.screens[0]
        anchors.top: true
        margins.top: 380
        implicitWidth: 720
        implicitHeight: 230
        color: "transparent"
        exclusiveZone: 0

        property bool abierta: false
        property real maxHueco: 0
        property int frames: 0

        Rectangle {
            id: orbe
            width: 56; height: 56; radius: 28; color: "#151616"
            y: 62
            x: ventana.abierta ? 112 : 332
            Behavior on x { SpringAnimation { spring: 3.2; damping: 0.32; mass: 1 } }
            Row {
                anchors.centerIn: parent; spacing: 10
                Repeater { model: 2; Rectangle { width: 7; height: 17; radius: 3.5; color: "#f5f7f5" } }
            }
        }
        Rectangle {
            x: orbe.x + 90; y: 20; radius: 24; color: "#151616"
            width: ventana.abierta ? 406 : 0
            height: ventana.abierta ? 190 : 0
            Behavior on width { SpringAnimation { spring: 2.8; damping: 0.36 } }
            Behavior on height { SpringAnimation { spring: 2.8; damping: 0.36 } }
        }

        FrameAnimation {
            running: true
            onTriggered: {
                ventana.frames++
                if (frameTime > ventana.maxHueco) ventana.maxHueco = frameTime
            }
        }
        Timer {
            interval: 2600; running: true; repeat: true; triggeredOnStart: false
            onTriggered: {
                if (ventana.frames > 0)
                    console.log("qml · ciclo · " + ventana.frames + " frames · hueco máximo "
                        + (ventana.maxHueco * 1000).toFixed(1) + " ms")
                ventana.frames = 0; ventana.maxHueco = 0
                ventana.abierta = !ventana.abierta
                const fin = Date.now() + 600
                while (Date.now() < fin) {}
            }
        }
        Timer { interval: 9000; running: true; onTriggered: Qt.quit() }
    }
}
