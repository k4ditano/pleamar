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
| `Shape` · `ShapePath` · `PathLine` | 40 · 78 · 56 | ✅ `path` con `move`, `line`, `curve` y `close`: relleno o con trazo, y se funde con lo demás |
| `Component` · `Loader` | 67 · 64 | 🟡 `component` sí; carga diferida no · §2 |
| `GradientStop` | 66 | 🟡 degradado lineal de dos colores; sin paradas ni radial |
| `Image` | 63 | ✅ `image`, por fichero, por icono o desde un dato |
| `TextInput` | 34 | ✅ `input` (una línea; sin IME) |
| `ListView` · `Flickable` | 31 · 22 | 🟡 `view:` en un reparto, con `content:` y `for … from` para listas de miles en unas pocas copias. Sin arrastrar |
| `Flow` | 28 | ⬜ sin salto de línea en los repartos |
| `QtObject` | 38 | 🟡 propiedades sueltas, sin agrupar |

| De Quickshell | | pleamar |
| --- | --- | --- |
| `Process` | 83 | ✅ `run`, `spawn`, `kill`, con permisos |
| `Singleton` | 47 | ✅ un plugin (biblioteca con lógica) es esto, y además con frontera y permisos |
| `Region` | 27 | ✅ la región de entrada se calcula sola, de las zonas |
| `FileView` | 16 | ✅ servicio `files`, con carpeta propia por escena y por plugin |
| `PanelWindow` | 10 | ✅ varias por escena (`surface panel { … }`), con su `open:` |
| `Variants` | 7 | ✅ `screens: each`: una superficie por monitor, cada una con su estado |
| `ShellRoot` · `Scope` | 7 · 4 | ✅ `scene` |
| `IpcHandler` | 4 | ✅ `pleamar --decir`, con `emit`, `fact`, `text`, `get` |
| `IconImage` | 3 | ✅ `image x = icon "…"` |
| `SystemClock` | 2 | ✅ servicio `clock`, y `service clock as now { … }` sin lógica ninguna |
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

### 🟡 Listas: lo que queda

Una lista de cinco mil ya cabe en dieciséis copias (`content:` + `for … from`, §3.1), así que lo que hay no depende de lo que haya. Falta **arrastrar** para moverlas, que en un panel táctil es lo natural, y que las copias **nazcan y mueran solas** en vez de que la escena declare la ventana y la lógica corte el trozo (G12).

### 🟡 Carga diferida: `Loader`, `LazyLoader`, `Component`

128 usos. En pleamar todo se despliega al cargar. Para un menú que casi nunca se abre, o una lista de 200, eso es trabajo y memoria por nada.

### 🟡 Lo que no se ha probado con manos de verdad

El teclado exclusivo, el clic fuera de una emergente, el arrastre real, la rueda, y las notificaciones y la bandeja en la sesión de verdad (con k4 parado). Están en la nota 08 como E1, E8, S11, B12 y B14.

### ⚪ Lo demás

Ventanas normales y bloqueo de sesión (S7); `ScreencopyView`; degradados con paradas y radial; `Flow`; IME (E7). De los ficheros queda el formato: se guarda texto, y el JSON se lo parsea quien escribe (F1).

## 3. Lo que pleamar tiene y Quickshell no

Para no perderlo de vista, porque es la razón de que esto exista:

- **El render anima solo.** Con la lógica bloqueada 600 ms, 38 frames a ~17 ms; QtQuick, en el mismo ensayo, un hueco de 600 ms. La lógica no puede hacer tartamudear la pantalla, por mal escrita que esté.
- **Todo se comprueba al cargar.** Un nombre mal escrito es un fallo con fichero, línea, flecha y «did you mean…?», no un `undefined` en marcha.
- **Un lenguaje que no puede colgarse:** sin bucles libres ni recursión. Lo declarado termina siempre.
- **Plugins con contrato:** frontera propia bajo su nombre, hilo propio, y permisos que **aprueba quien los usa** (`pleamar --aprobar`). En Quickshell, un trozo de configuración de otro es JavaScript con todos tus permisos.
- **Formas que se funden**, con sombra, filo y luz, sin capas ni trucos: es SDF.
- **Multiplataforma por diseño:** todo lo del sistema detrás de `src/plataforma/`, y `./portable.sh` comprueba que compila para Windows y macOS. Los escritorios y la ventana activa van por protocolos estándar, no por un compositor.

## 3.1. Lo que además salió de aquí

**El editor sabe el lenguaje**: `pleamar --lsp` da los fallos mientras se escribe, con el mismo compilador que lee la escena, y `--resaltado` escribe la sintaxis desde el vocabulario. Quickshell tiene el tooling de QML, que es mucho más viejo y más completo; esto es pequeño, pero no puede desfasarse.

**Una lista de cinco mil filas cuesta lo que dieciséis**, y se escribe en la escena: `content:` dice lo que mide de verdad, `for … from` numera las copias desde donde toca, y el desplazamiento es una propiedad que una regla puede llevar donde quiera. En Quickshell eso es `ListView`, que virtualiza él solo pero se lleva su propio hilo de instanciación por delante.

**Un camino es una forma más**, no una isla: `Shape` en QtQuick es un motor aparte (triangula y pinta con otro camino de render), así que no se funde con lo de alrededor ni tiene sombra de balde. Aquí es la misma distancia con signo que un círculo, y `blend` lo funde con lo que tenga al lado.

**Un servicio se pide desde la escena**, no desde la lógica: `service clock as now { time: text }` y los campos llegan solos a hechos y textos, con el compilador comprobando que ese servicio trae eso. En Quickshell lo equivalente es un `Singleton` con sus `property` y su JavaScript.

## 4. En qué orden

1. Que el servidor de lenguaje sepa de nombres: completar los hechos y los componentes de la escena, e ir a donde se declaran (G9).
2. Copias que nazcan y mueran solas (G12), y arrastrar una lista.
3. Ventanas normales, bloqueo de sesión, IME.
