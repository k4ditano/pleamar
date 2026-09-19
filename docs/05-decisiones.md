# Decisiones y preguntas abiertas

## Decidido

| Fecha | Decisión | Por qué |
| --- | --- | --- |
| 2026-09-19 | **El render es dueño de las animaciones**; la lógica declara intenciones | Es la idea que se quería probar, y se midió: 38 frames frente a 0 con la lógica bloqueada |
| 2026-09-19 | **Una escena son datos**, no código | Si el render ha de trabajar solo, no puede depender de ejecutar nada ajeno |
| 2026-09-19 | **Formas SDF** en un shader intérprete | Fundir, sombrear y hacer hit-test salen de la misma fórmula |
| 2026-09-19 | **Lenguaje propio, A+B**: árbol de elementos + todo lo declarativo que el render pueda hacer solo | C (Luau puro) es más potente pero no puede garantizar qué corre en el render. Esa frontera es el proyecto |
| 2026-09-19 | **Luau solo para la lógica** | Sandbox, tipos graduales, LSP hecho, y las IA lo escriben bien |
| 2026-09-19 | **Sintaxis aburrida, semántica nueva** | Que quien venga de QML lo lea a la primera; lo único que aprender es lo que lo hace distinto |
| 2026-09-19 | **`capa` es el concepto central, no `estado`** | La prueba con Marea: `forma` es un hueco que muchos reclaman, y hoy se defiende con ocho guardas copiadas |
| 2026-09-19 | **Dos formas de animar**: muelles y gestos por fotogramas | Marea tiene 25 gestos que ya son datos de fotogramas; un muelle no cuenta una historia |
| 2026-09-19 | **Primero la semántica en el runtime, después el parser** | Un parser congela la sintaxis; aún no sabemos del todo qué conceptos hacen falta |
| 2026-09-19 | **Una forma de capa solo se ve con más de media presencia** | Fundir dos formas de ojos da un borrón; como las presencias suman uno, así una se va y entra la otra |
| 2026-09-19 | **Un `estado` es una reclamación que fija propiedades**; no hay construcción aparte | Abrir/cerrar la tarjeta de Marea salió como `capa tarjeta { abierta mientras abierta?; reposo }`, sin nada nuevo |
| 2026-09-19 | **Las palabras clave del lenguaje, en inglés** (`layer`, `gesture`, `while`, `after`…) | Decisión de Abel. k4 ya recibe PRs de fuera y cambiarlo después es caro. El código Rust del runtime sigue en castellano por ahora; el parser hará de frontera |

## Lo que enseñó implementarlo

- **La lógica de Marea se quedó en diez líneas.** Abrir, cerrar, dormirse, realzar el botón y el salto al pulsar son reglas. A la lógica le llega `Capa("tarjeta", "abierta")` para hacer *su* trabajo y `Suceso("ver-evento")`.
- **La lógica de la isla no decide nada**: dos capas y tres reglas.
- Hizo falta `Alternar(hecho)` —pulsar para abrir y cerrar— y `Fuera{durante}` tiene que «armarse»: solo cuenta si antes se estuvo encima, o cerraría nada más arrancar.
- Lo que emite un fotograma se atiende en el frame siguiente. Para un obturador de cámara son 16 ms; habrá que ver si importa.
- **Pendiente:** dos capas que fijan la misma propiedad (hoy gana la última que cambió), transiciones distintas según *de dónde* se viene, y gestos con parámetros (`señalar(lado)`).

## Abierto

| Pregunta | Opciones | Nota |
| --- | --- | --- |
| ¿Cómo se llama el lenguaje, y su extensión? | — | pleamar es el runtime; el nombre lo puso Claude y se puede cambiar |
| El orden de prioridad de `capa forma` | Ver [[pleamar · 04 Prueba - cabe Marea]] §3.2 | Lo dedujo Claude de los comentarios. Hay que revisarlo línea a línea |
| ¿`confirmado()` mientras graba borra el disco rojo? | — | Visto leyendo `ExpressionController.qml`, sin ejecutar. Comprobar en Marea |
| ¿Tipos escritos o inferidos? | `hecho cuenta: entero` · `hecho cuenta = 0` | |
| ¿Qué pasa si dos capas fijan la misma propiedad? | Error al cargar · gana la capa declarada antes | Probablemente error |
| Componentes, repetición, layout, texto dinámico | — | Sin diseñar. Sin esto no hay lista de notificaciones |
| ¿Render por CPU para lo estático? | — | Bajaría los ~100 MB que pone el driver de Vulkan |
