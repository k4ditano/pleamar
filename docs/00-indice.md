# pleamar — Índice

**Qué es:** un runtime nuevo para escritorio —lo que sería «un Quickshell mejor»— y el lenguaje con el que se escriben sus escenas. Nació el 19 sep 2026 de una conversación sobre por qué QtQuick no puede animar mientras la lógica trabaja.

**Dónde está:** `~/Proyectos/pleamar` · [github.com/k4ditano/pleamar](https://github.com/k4ditano/pleamar) (privado). Estas notas tienen copia en `docs/` del repo; **si difieren, manda la de Edinot**.

## Las notas

| Nota | Para qué |
| --- | --- |
| [[pleamar · 01 Por qué existe]] | El problema, la idea y lo que se midió |
| [[pleamar · 02 Cómo funciona hoy]] | El runtime que ya corre: hilos, escena, shader |
| [[pleamar · 03 El lenguaje - borrador 0]] | El boceto en castellano de antes de implementarlo. Histórico; manda la 09 |
| [[pleamar · 04 Prueba - cabe Marea]] | El `ExpressionController` de Marea contra el lenguaje |
| [[pleamar · 05 Decisiones y preguntas abiertas]] | Lo decidido, con su porqué, y lo que falta por decidir |
| [[pleamar · 06 Bocetos A-B-C]] | Las tres sintaxis que se compararon. Histórico. |
| [[pleamar · 07 Qué falta para igualar a Quickshell]] | El inventario honesto, por tramos, y el orden de trabajo |
| [[pleamar · 09 El lenguaje v0]] | **La gramática que ya funciona**, con palabras clave en inglés. La referencia |
| [[pleamar · 08 Limitaciones conocidas]] | Todo lo que está a medias, con su gravedad, para ir tachándolo |

## Estado (19 sep 2026)

- ✅ **El render anima solo.** Con la lógica bloqueada 600 ms, 38 frames a ~17 ms. El mismo ensayo en QML: un hueco de 600 ms.
- ✅ **Una escena son datos.** Propiedades, expresiones, lista de dibujo, comportamientos y zonas. Dos escenas (Marea, isla) sobre el mismo render.
- ✅ **Se eligió la dirección del lenguaje:** árbol (A) + lo declarativo de (B), Luau solo para la lógica.
- ✅ **Se probó contra la Marea real.** Cabe, con tres piezas que el boceto no tenía: hechos y sucesos, capas, gestos.
- ✅ **Capas, gestos, hechos, sucesos y reglas en el runtime** (todavía escritos en Rust). Probado con la cara de Marea: el disco rojo aguanta buscar y confirmar mientras graba; al dejar de grabar, la lupa sale sola. Y Marea se abre, realza su botón y se cierra con la lógica bloqueada 5 s.
- ✅ **Frontera de plataforma** (`src/plataforma/`): todo lo de Wayland vive detrás, y `./portable.sh` comprueba que el núcleo compila para Linux, Windows y macOS. Pasa.
- ✅ **Texto de verdad e imágenes**: `cosmic-text` (ligaduras, árabe, japonés, emoji, líneas, puntos suspensivos, alineado), glifos a la escala del monitor, **textos vivos** que cambia la lógica, SVG/PNG/JPEG, iconos por nombre y teñido. Marea y la isla ya no usan mapas de bits.
- ✅ **Medir texto desde una expresión** y **taller de texto** en su propio hilo: ni un frame lento después del primero.
- ✅ **El lenguaje, v0**: un fichero `.plm` → escena. Tokenizador, parser, comprobación de nombres con «¿querías decir…?», errores con línea y flecha, y **recarga en caliente** que no pierde nada. `escenas/marea.plm` y `escenas/cara.plm` corren sin una línea de Rust.
- ⬜ Parser.
- ✅ **Renderer por elementos**: un quad por elemento con su caja. 600 formas cuestan 0,42 ms por frame frente a 3,66 ms del intérprete por píxel; 2000, 0,57 ms. Con él llegaron **aro/trazo, arco, segmento, giro** (propio y heredado), **degradado lineal, borde**, recortes anidados (hasta cuatro) y fundido real entre elementos. Y después, cerrando sus limitaciones: **transformaciones afines que se componen** (giro, escala, traslación), **opacidad de grupo** con capa intermedia, caja exacta para texturas giradas y zonas de ratón bajo transformaciones. Todo a la vista en `--escena muestrario`.
- ✅ **Superficies de verdad**: una por monitor —y por los que se enchufen después—, escala fraccional (probada a 2 y a 1,5 en un monitor virtual), y una región de entrada que sigue a las zonas: lo transparente deja pasar el clic. La escena declara su superficie (tamaño, ancla, nivel, margen, reserva).
- ⬜ más primitivas, texto dinámico, componentes, layout, servicios: ver la nota 07.

## Siguiente paso

Lo que queda del punto 4: **componentes, `repeat` y layout** —sin eso no hay lista de notificaciones ni lanzador—, y que los destinos de una capa o de un fotograma puedan ser expresiones. Después, el punto 5: **Luau** para la lógica, que es lo que le falta a una escena de fichero para tener vida propia.
