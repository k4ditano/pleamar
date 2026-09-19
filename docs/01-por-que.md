# Por qué existe

## El problema

Quickshell es QML sobre Qt, y QtQuick anima en el mismo hilo en que corre la lógica. Un `Behavior` o un `SpringAnimation` se paran si un handler de JavaScript tarda. Y tarda justo cuando más se nota: **al abrir un panel**, que es cuando se instancia, se lee y se parsea.

No es un fallo de Quickshell ni se arregla cambiando C++ por Rust: es una decisión de diseño de QtQuick, que está hecho para aplicaciones y no para escritorios.

## La idea

Copiada de Core Animation (iOS), que es por lo que la Dynamic Island no se atasca nunca: **la vista declara transiciones y otro las ejecuta.**

- La **lógica** dice «el ancho va a 406 con este muelle, dentro de 70 ms» y se olvida.
- El **render**, en su hilo, recorre esa intención a la cadencia de la pantalla pase lo que pase.

De ahí sale todo lo demás: si el render tiene que poder trabajar solo, lo que se le da tienen que ser **datos y expresiones puras**, no código. Y eso es un lenguaje.

## Lo que se midió

RTX 2060, pantalla a 60 Hz. Abrir la tarjeta de Marea con la lógica bloqueada 600 ms justo al empezar:

| | frames durante el bloqueo | frame más largo |
| --- | --- | --- |
| pleamar | 38 | 17–19 ms |
| pleamar `--ingenuo` (lógica en el hilo que pinta) | 0 | 617 ms |
| QtQuick sobre Quickshell (`comparar/shell.qml`) | 0 | 600 ms |

Memoria en reposo: 119 MB frente a 214 MB del banco de Quickshell. **La mitad, no la cuarta parte**: casi todo lo de pleamar es el driver de Vulkan de NVIDIA. Pintar lo estático por CPU sería la forma de bajar de ahí.

## Las cinco ideas del diseño completo

Solo la 2 y media 3 están hechas.

1. **El estado vive fuera de la vista.** Recargar es cambiar la función, no perder el estado.
2. **Las animaciones no las ejecuta tu código.** ← lo que prueba el prototipo
3. **Un renderer para este dominio**: formas SDF. Transformar una forma en otra es interpolar dos fórmulas.
4. **El sistema como grafo de datos perezoso**: `audio.salida.volumen`. Si nadie lo mira, ni se conecta.
5. **Los plugins son actores con permisos**: VM aislada, presupuesto de CPU, nunca en el hilo que pinta.

## Lo que esto no es

No sustituye a k4 ni a Marea. Son 2000 líneas de prototipo contra quince años de QtQuick: texto, IME, accesibilidad, multimonitor, bandeja… Un prototipo que impresione son meses; algo para usar a diario, años. **El camino realista para k4 y Marea hoy sigue siendo un daemon en Rust + Lua al lado de Quickshell.** pleamar es la pregunta de qué habría que construir si no existiera nada.
