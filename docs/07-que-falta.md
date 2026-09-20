# Paridad con Quickshell — qué falta de verdad

**Para qué es esta nota.** pleamar es **una alternativa a Quickshell**, no el motor de Marea. Marea fue la prueba de esfuerzo del principio, y algún día se reescribirá con este lenguaje; pero lo que decide si esto sirve es si alguien que hoy escribe su escritorio en Quickshell puede escribirlo aquí. Esta nota es esa vara de medir, y manda sobre [[pleamar · 04 Prueba - cabe Marea]].

**De dónde salen los números.** No de la documentación de Quickshell: de **dos configuraciones de verdad** que hay en esta máquina —`~/.config/quickshell` (k4, 278 ficheros QML) y `proyecto-marea` (370)—, contando qué tipos instancian. Un tipo que nadie usa no es una carencia; uno que sale 800 veces, sí.

## 1. Lo que usan de verdad

Tipos instanciados, con las veces que aparecen entre las dos configuraciones:

| De QtQuick | | pleamar |
| --- | --- | --- |
| `Rectangle` | 819 | ✅ `box`, y además fundido de siluetas, sombra, filo y luz |
| `Text` | 655 | ✅ `text`, con huecos (`"{a} · {b}"`), medida y recorte |
| `NumberAnimation` · `SequentialAnimation` · `ParallelAnimation` · `PauseAnimation` · `ScriptAction` | 469 · 78 · 38 · 42 · 37 | ✅ muelles (`~calm`), capas con retraso, y `gesture` (que es una línea de tiempo: lo mismo que `SequentialAnimation`) |
| `RowLayout` · `ColumnLayout` · `Row` · `Column` | 460 · 279 · 106 · 74 | ✅ `row` / `column` con hueco, relleno, alineado, ancla, `between` y muelle |
| `Repeater` | 364 | ✅ `repeat` (fijo) y `for` (sobre un modelo) |
| `MouseArea` · `HoverHandler` | 351 · 33 | ✅ zonas: `on press/enter/leave/hover/drag/scroll/hold` |
| `Item` | 351 | ✅ `group` |
| `Timer` | 195 | ✅ `every` en la escena, `after`/`every` en la lógica |
| `Connections` | 145 | ✅ reglas `on suceso`, `on("fact:x")` |
| `Shape` · `ShapePath` · `PathLine` | 40 · 78 · 56 | ⬜ **no hay** · §2 |
| `Component` · `Loader` | 67 · 64 | 🟡 `component` sí; carga diferida no · §2 |
| `GradientStop` | 66 | 🟡 degradado lineal de dos colores; sin paradas ni radial |
| `Image` | 63 | ✅ `image`, por fichero, por icono o desde un dato |
| `TextInput` | 34 | ✅ `input` (una línea; sin IME) |
| `ListView` · `Flickable` | 31 · 22 | ⬜ **no hay** · §2 |
| `Flow` | 28 | ⬜ sin salto de línea en los repartos |
| `QtObject` | 38 | 🟡 propiedades sueltas, sin agrupar |

| De Quickshell | | pleamar |
| --- | --- | --- |
| `Process` | 83 | ✅ `run`, `spawn`, `kill`, con permisos |
| `Singleton` | 47 | ✅ un plugin (biblioteca con lógica) es esto, y además con frontera y permisos |
| `Region` | 27 | ✅ la región de entrada se calcula sola, de las zonas |
| `FileView` | 16 | ⬜ **no hay**: ni leer ni escribir ficheros · §2 |
| `PanelWindow` | 10 | 🟡 una por escena · §2 |
| `Variants` | 7 | ⬜ **no hay**: es lo que hace una barra por pantalla · §2 |
| `ShellRoot` · `Scope` | 7 · 4 | ✅ `scene` |
| `IpcHandler` | 4 | ✅ `pleamar --decir`, con `emit`, `fact`, `text`, `get` |
| `IconImage` | 3 | ✅ `image x = icon "…"` |
| `SystemClock` | 2 | ⬜ la hora la pone la lógica llamando a `date` cada segundo |
| `ScreencopyView` | 2 | ⬜ ver lo que hay en una pantalla o en una ventana |
| `NotificationServer` | 2 | ✅ servicio `notifications` |
| `FloatingWindow` | 2 | ⬜ ventanas normales |
| `WlSessionLock` | 1 | ⬜ bloqueo de sesión |
| `LazyLoader` | 1 | ⬜ · §2 |
| `GlobalShortcut` | 1 | 🟡 un bind del compositor que llama a `--decir` |
| `PwObjectTracker` | 1 | ✅ servicio `audio` (aunque por `wpctl`, no nativo) |
| `ClippingRectangle` | 1 | ✅ `clip` |

Y lo que usan del objeto `Quickshell`: `env` (82), `shellPath` (32), `screens` (31), `execDetached` (24), `iconPath` (18), `clipboardText` (6).

## 2. Lo que falta, por lo que duele

### 🔴 Muchas superficies, y una por pantalla

Una configuración de Quickshell **es un proceso con muchas ventanas**: la barra en cada monitor, el lanzador, el centro de notificaciones, el bloqueo. Comparten estado y se hablan entre ellas sin dar la vuelta por el sistema. Eso es `Variants` (7 usos) más `PanelWindow.screen` (9).

En pleamar, una escena es **una** superficie, y cada cosa es un proceso aparte que no comparte nada. Es el hueco más grande que queda, y se nota en todo: no se puede escribir una barra de verdad para dos monitores.

**Qué haría falta:** varias `surface` con nombre en una escena, cada una con su dibujo; y `per screen` para que una se repita por monitor, con sus propias propiedades y hechos. Anotado como S1, S2 y G22.

### 🔴 Listas con desplazamiento

`ListView` (31) y `Flickable` (22). Cualquier configuración real tiene una lista más larga que su hueco: notificaciones, ventanas, resultados. Hoy un modelo tiene tope (`max 14`) y se despliega entero al cargar: ni scroll, ni «solo se instancia lo que se ve».

### 🟡 Dibujo vectorial: `Shape`, `ShapePath`, `PathLine`

174 usos entre los tres. Un anillo de progreso, una curva, un gráfico, una flecha. pleamar dibuja con SDF —que es mejor para lo suyo: se funden, tienen sombra y filo de balde— pero **no tiene caminos**. Hace falta algo: `path { move; line; curve; close }`, rellenado y con trazo.

### 🟡 Ficheros: `FileView`

16 usos. Leer un `.json` de configuración, guardar lo que el usuario elige, ver si algo cambió. Hoy la lógica solo puede lanzar `cat`, y eso pide permiso de `run`. Debería ser un servicio con sus permisos: `files.read`, `files.write` sobre una carpeta declarada.

### 🟡 Carga diferida: `Loader`, `LazyLoader`, `Component`

128 usos. En pleamar todo se despliega al cargar. Para un menú que casi nunca se abre, o una lista de 200, eso es trabajo y memoria por nada.

### 🟡 Lo que no se ha probado con manos de verdad

El teclado exclusivo, el clic fuera de una emergente, el arrastre real, la rueda, y las notificaciones y la bandeja en la sesión de verdad (con k4 parado). Están en la nota 08 como E1, E8, S11, B12 y B14.

### ⚪ Lo demás

Ventanas normales y bloqueo de sesión (S7); `ScreencopyView`; un reloj como servicio; degradados con paradas y radial; `Flow`; otros compositores además de Hyprland (B8); IME (E7); mensajes de error en inglés (G5).

## 3. Lo que pleamar tiene y Quickshell no

Para no perderlo de vista, porque es la razón de que esto exista:

- **El render anima solo.** Con la lógica bloqueada 600 ms, 38 frames a ~17 ms; QtQuick, en el mismo ensayo, un hueco de 600 ms. La lógica no puede hacer tartamudear la pantalla, por mal escrita que esté.
- **Todo se comprueba al cargar.** Un nombre mal escrito es un fallo con fichero, línea, flecha y «¿querías decir…?», no un `undefined` en marcha.
- **Un lenguaje que no puede colgarse:** sin bucles libres ni recursión. Lo declarado termina siempre.
- **Plugins con contrato:** frontera propia bajo su nombre, hilo propio, y permisos que **aprueba quien los usa** (`pleamar --aprobar`). En Quickshell, un trozo de configuración de otro es JavaScript con todos tus permisos.
- **Formas que se funden**, con sombra, filo y luz, sin capas ni trucos: es SDF.
- **Multiplataforma por diseño:** todo lo del sistema detrás de `src/plataforma/`, y `./portable.sh` comprueba que compila para Windows y macOS.

## 4. En qué orden

1. **Muchas superficies y una por pantalla.** Sin esto no hay barra de verdad.
2. **Listas con desplazamiento**, y copias que nazcan en marcha (G12): las dos son la misma obra.
3. **Ficheros** como servicio, y un **reloj** como servicio.
4. **Caminos** (`path`).
5. Sacar del núcleo lo que solo servía para Marea, y los mensajes en inglés.
6. Ventanas normales, bloqueo de sesión, IME, otros compositores.
