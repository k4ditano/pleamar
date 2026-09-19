# Limitaciones conocidas

Todo lo que se sabe que está a medias, **con su plan de arreglo**, para ir tachándolo. Se anota según sale, no al final.

**Gravedad**: 🔴 frena lo siguiente · 🟡 molestará pronto · ⚪ caso raro. Al arreglar algo se mueve a la tabla de abajo con su fecha, no se borra.

## Multiplataforma

La regla: **todo lo de sistema vive detrás de `src/plataforma/`**, y el núcleo solo usa crates que existen en Linux, Windows y macOS. La guarda es `./portable.sh`, que compila el núcleo contra los tres; hoy pasa.

| Pieza | Linux | Windows | macOS | Cómo se completa |
| --- | --- | --- | --- | --- |
| Núcleo (modelo, render, capas, gestos, texto, imágenes) | ✅ | compila | compila | — |
| Ventanas (`plataforma::atender`, `Ventana`) | ✅ Wayland | ⬜ dice que no sabe y sale | ⬜ ídem | **Windows**: ventana sin bordes por capas + DirectComposition para el alfa, `WM_NCHITTEST` para la región de entrada, AppBar para `reserva`, `WM_DPICHANGED` para la escala. **macOS**: `NSPanel` sin bordes, nivel flotante, `CAMetalLayer` no opaca; lo transparente ya deja pasar el clic; `reserva` no existe y se ignora |
| Iconos por nombre (`plataforma::icono`) | 🟡 búsqueda ingenua | ⬜ devuelve nada | ⬜ devuelve nada | Windows: `IShellItemImageFactory`. macOS: `NSWorkspace.icon(forFile:)`. Los dos dan un mapa de bits, no una ruta: `icono` tendrá que poder devolver píxeles |
| Fuentes del sistema | ✅ `fontdb` + fontconfig | sin probar | sin probar | `fontdb` lee las carpetas de fuentes de cada sistema; falta verlo correr |
| Puntero global | ⬜ Wayland lo prohíbe | fácil (`GetCursorPos`) | fácil (`NSEvent.mouseLocation`) | En Linux, por el IPC del compositor. Es un hecho del grafo de datos |
| **Solo se comprueba que compila**: nadie ha ejecutado pleamar fuera de Linux | | | | Una máquina virtual de Windows con GPU, o CI que al menos arranque con el adaptador por software de `wgpu` |

## Pendientes

### Servicios y la barra

| # | Limitación | Gravedad | Cómo se arregla |
| --- | --- | --- | --- |
| B1 | **Solo hay dos servicios, y solo de Hyprland**: `workspaces` y `window`. El volumen de `barra.luau` sale de `pactl` y `wpctl`, que ata ese script a Linux | 🔴 | `audio`, `media`, `battery`, `network`, `notifications` y `tray` como servicios en `plataforma/`, con el mismo nombre y la misma tabla en todos los sistemas: PipeWire / WASAPI / CoreAudio, MPRIS / SMTC / MediaRemote. Otros compositores de Linux: `ext-workspace` y `wlr-foreign-toplevel` |
| B2 | `ws.active` es el escritorio con foco en todo el sistema, no el del monitor donde está la barra | 🟡 | Una instancia de escena por monitor (S2), con un hecho `screen.name` que la lógica pueda leer |
| B3 | `size: full` usa el ancho del **primer** monitor para todas las superficies | 🟡 | Lo mismo: S2 |
| B4 | **Sin probar**: `reserve` distinto de 0 (que las ventanas le dejen sitio a la barra). Y al recargar en caliente no se aplica: hay que relanzarla (G7) | ⚪ | Que lo pruebe Abel subiendo `reserve` a 50; y reconfigurar la superficie al recargar |
| B5 | Si al programa lo matan con una señal, lo que dejó corriendo (`pactl subscribe`) queda huérfano. Al salir por las buenas sí se para | 🟡 | En `plataforma/`: `PR_SET_PDEATHSIG` en Linux, un Job Object en Windows; y atender `SIGTERM` para salir por las buenas |
| B6 | La barra tarda ~420 ms en su primer frame, frente a ~170 de Marea | ⚪ | Medir qué: probablemente los 27 textos y el icono, que el taller hace en serie |
### La lógica (Luau)

| # | Limitación | Gravedad | Cómo se arregla |
| --- | --- | --- | --- |
| U1 | **`run` lanza cualquier orden.** La caja de arena cierra `io` y `os`, pero esa puerta está abierta de par en par y sin permisos | 🔴 | Un manifiesto por escena o plugin: qué órdenes, qué rutas, qué servicios. Sin declarar, nada. Es la idea 5 del diseño: los plugins son actores con permisos |
| U3 | Una lógica por escena: no hay plugins, ni varios scripts, ni `require` | 🟡 | Un estado de Luau por plugin, cada uno con su presupuesto, y un espacio de nombres para sus hechos |
| U4 | Los hechos se leen como números (`fact.open == 1`), no como sí/no | ⚪ | Que la escena declare el tipo (`fact open: bool`) y la lógica lo respete |
| U5 | `./portable.sh` comprueba Windows y macOS **sin** Luau: aquí no hay compilador cruzado de C++ | ⚪ | Instalar `mingw-w64-gcc`, o CI en los tres sistemas |
| U6 | El error de un nombre mal escrito no trae la línea del script: la traza de Luau se queda en «[C]» | ⚪ | Pedir la traza al nivel de quien llama (`debug.traceback` al nivel 2) |

### El lenguaje

| # | Limitación | Gravedad | Cómo se arregla |
| --- | --- | --- | --- |
| G12 | **Una lista de longitud variable es una de capacidad fija** con `show:`. Con la lógica poniendo textos y un `count` se comporta como una de verdad (`bandeja.luau`), pero el tope lo pone el fichero y no hay copias que nazcan en marcha | 🟡 | `repeat item in notes { … }` sobre un modelo que ponga la lógica, con clave por elemento para que al reordenar cada uno viaje a su sitio nuevo. Pide copias que nazcan y mueran en marcha: hoy todo se despliega al cargar |
| G14 | Los parámetros de un componente van por posición, sin tipo, sin valor por defecto, y no hay hueco para hijos (`children`) | 🟡 | `component Card(title, tone = mint) { … slot … }` |
| G15 | El reparto sabe de fila y columna con hueco, relleno, alineado y ancla (`anchor: right`). Sin salto de línea, sin «ocupa lo que quede», sin mínimos ni máximos. Un `group` sin `size:` ocupa lo que su último hijo, por casualidad más que por diseño | 🟡 | `grow:` en un hijo (reparte el sobrante del `size:` del reparto), `wrap`, y que un grupo sin tamaño ocupe la unión de sus hijos |
| G16 | El sitio de cada hijo es la suma de los anteriores, escrita entera: con *n* hijos las expresiones crecen como *n²*. Veinte no se notan; doscientos sí | ⚪ | Subexpresiones compartidas: que `Expr` sea un grafo y cada suma parcial se evalúe una vez por frame |
| G17 | Lo que mide un texto llega un frame tarde, así que un reparto con textos se asienta en uno o dos frames al cargar. Con muelle no se ve; sin él, un parpadeo | ⚪ | Ver T12 |
| G5 | Los mensajes de error están en castellano aunque las palabras clave sean inglesas. Y un fallo del tokenizador (una comilla sin cerrar) sigue parando la lectura en seco | 🟡 | Un catálogo de mensajes con las dos lenguas; que el tokenizador apunte el fallo y siga en la línea siguiente |
| G7 | Al recargar no se aplican los cambios de `surface` (S1), y el gesto que estuviera sonando y los retrasos pendientes se pierden | ⚪ | Reconfigurar la superficie; conservar el gesto si sigue existiendo con ese nombre |
| G9 | **Sin ayuda en el editor**: ni colores, ni autocompletado, ni errores mientras se escribe | 🟡 | Una gramática de tree-sitter para el resaltado, y un LSP pequeño que reutilice este mismo parser: los fallos ya vienen con su sitio |
| G11 | La recarga mira la fecha del fichero cuatro veces por segundo | ⚪ | Vale en los tres sistemas, que es por lo que se hizo así. Si molesta: el crate `notify` |

### Texto e imágenes

| # | Limitación | Gravedad | Cómo se arregla |
| --- | --- | --- | --- |
| T12 | Un texto que aparece **por primera vez** no se ve hasta que el taller lo entrega (uno o dos frames); uno que *cambia* enseña el anterior mientras tanto. Y lo que mide un texto llega a las expresiones un frame después | ⚪ | Lo que ya está en la escena al cargarla se encarga antes de que exista la ventana. Para lo demás: componer dos veces el frame en que cambia una medida |
| T3 | Un solo atlas de 2048² en RGBA: 16 MB de VRAM aunque esté casi vacío. Si se llena, los glifos nuevos no se pintan (avisa por consola) y no se recicla nada | 🟡 | Empezar en 512² y crecer; desalojar por estantes lo que lleve más sin usarse; las máscaras de glifo en un atlas de un canal, que ocupa la cuarta parte |
| T4 | Los glifos se pintan a la escala de la lámina **más fina**; las demás los ven reducidos. Con un monitor a 2 y otro a 1, en el de 1 el texto sale una pizca blando | ⚪ | Un atlas por escala, y que cada lámina tenga sus propios `uv`: pide una tabla de elementos por lámina en vez de una común |
| T5 | Texto bajo una transformación que escala se remuestrea (ampliado, sale blando), y girado no cae en píxeles enteros | ⚪ | Pintar a `escala × factor` de la transformación cuando lleve quieta unos frames |
| T6 | Sin texto rico (negrita o color a mitad de párrafo), ni espaciado entre letras, ni subrayado, ni selección ni edición | 🟡 | `cosmic-text` ya tiene tramos con atributos: exponer `Contenido::Rico`. La edición es otro proyecto (IME incluido) |
| T7 | Al partir líneas queda un espacio al principio de la línea nueva | ⚪ | Mirar cómo trata `cosmic-text` el espacio final con `Wrap::WordOrGlyph`; si no tiene ajuste, recortarlo al colocar los glifos |
| T8 | `text number(expr, decimales, "detrás")` formatea un número, pero no hay plantillas (`"{a} de {b}"`), ni horas, ni separadores de miles | ⚪ | Cadenas con huecos: `text "{done} de {total}"` |
| T9 | Las imágenes tienen un tamaño máximo que declara la escena. Sin URL, sin GIF, sin «nueve parches». Al cambiar la escala se repintan todas | 🟡 | Caché en disco de los SVG ya pintados; tamaño según el destino en vez de declarado |
| T10 | La búsqueda de iconos no lee los `index.theme` ni sabe cuál es tu tema: prueba en los de siempre y se queda con el primero | 🟡 | La búsqueda de freedesktop de verdad (tema actual, herencia, tallas). Vive en `plataforma/` |
| T11 | Solo degradado lineal; sin radial, sin desenfoque, sin máscaras | 🟡 | El radial son diez líneas de shader. El desenfoque pide otra pasada por capa: la infraestructura de las capas de opacidad ya vale |

### Pintado

| # | Limitación | Gravedad | Cómo se arregla |
| --- | --- | --- | --- |
| P4 | Un grupo con opacidad dentro de otro no tiene capa propia: multiplica. Con más de cuatro fundiéndose a la vez, los que sobran también | ⚪ | Una pila de capas en vez de cuatro fijas, y pintar las de dentro antes que las de fuera |
| P5 | Con escala distinta en cada eje, el suavizado del borde es aproximado | ⚪ | Medir el gradiente de la distancia en el shader (`fwidth`) en vez de un factor fijo |
| P6 | El degradado y la luz viven en el espacio del grupo: si la forma gira *por sí misma* (`.girada`) no giran con ella | ⚪ | Evaluar la pintura en el espacio de la primera forma del cuerpo |
| P7 | Topes fijos: 4096 formas, 2048 elementos, 4 recortes anidados. Pasados, lo que sobra no se pinta, sin avisar. **Con texto se llega antes: cada glifo es un elemento** | 🟡 | Almacenes que crecen al doble cuando se llenan; avisar una vez. Y un elemento por *línea* de texto en vez de por glifo |
| P8 | La lista de dibujo se recompone entera en CPU cada frame, haya cambiado o no | ⚪ | Recomponer solo los tramos cuyas propiedades hayan cambiado; 2000 formas cuestan hoy 0,7 ms |
| P9 | Los primeros frames tras despertar no esperan a la pantalla: el reloj de animación va con el tiempo de la CPU, no con el de presentación | 🟡 | `wp_presentation` en Wayland (y sus equivalentes) para saber cuándo se enseñó cada frame, y avanzar los muelles con ese tiempo |
| P10 | Un frame de cada ~400 cae a 33 ms. Sin investigar | 🟡 | Medir con marcas de tiempo de GPU para saber si es nuestro o del compositor |

### Superficies y entrada

| # | Limitación | Gravedad | Cómo se arregla |
| --- | --- | --- | --- |
| E1 | **Rueda, botones, arrastre, mantener, teclas y cursor están probados con el guion de mentira**, que entra por el render. El lado de Wayland —que las muescas lleguen con su signo, que el cursor cambie, que el teclado llegue con `on_demand`— no lo ha visto nadie con un ratón de verdad | 🟡 | Que lo pruebe Abel en la barra: rueda sobre el volumen, arrastrar la barrita, el altavoz, y la manita sobre los escritorios |
| E2 | **No hay dónde escribir.** Las teclas llegan como sucesos, pero no existe un campo de texto: ni cursor de texto, ni selección, ni portapapeles, ni IME. Tampoco se exponen los modificadores (Ctrl, Mayús) | 🔴 | Un elemento `input` sobre `cosmic-text` (que ya sabe editar), `zwp_text_input_v3` para el IME en Wayland, y `on key Ctrl+K`. Sin esto no hay lanzador ni buscador |
| E3 | Sin arrastrar y soltar entre aplicaciones, sin atajos globales, y sin «cerrar al pulsar fuera» | 🟡 | `wl_data_device` para soltar; los atajos y el foco, por el compositor (`hyprland-global-shortcuts`, `hyprland-focus-grab`), detrás de `plataforma/` |
| E4 | La rueda suma lo vertical y lo horizontal, y un panel táctil se convierte en muescas a ojo (15 px por muesca) | ⚪ | `wheel.x` y `wheel.y` aparte, y desplazamiento continuo en píxeles para las listas |
| E5 | El botón derecho cierra el programa mientras la escena no lo use | ⚪ | Es la salida de emergencia del prototipo. Se irá cuando haya una orden para cerrar |
| E6 | Una zona que se arrastra dentro de un reparto anclado se mueve bajo el ratón si el reparto cambia de tamaño. Hoy se evita a mano, dando ancho fijo a lo que cambie | ⚪ | `min_width:` en los hijos, o congelar el reparto mientras dura un arrastre |
| S1 | El tamaño de la superficie lo declara la escena y es fijo | 🟡 | Hoy se esquiva declarándola grande: la región de entrada deja pasar el clic. De verdad: que la escena pueda cambiar su `Superficie` y la plataforma la reconfigure |
| S2 | Todas las superficies pintan **la misma escena con el mismo estado**. No hay una instancia por monitor | 🟡 | Es del lenguaje: `per screen { … }`, con propiedades y hechos propios por instancia |
| S3 | **Sin probar en dos monitores de verdad a distinto ritmo** (165 y 60 Hz) | 🟡 | `--pantalla todas` cuando a Abel le venga bien verlo en DP-3 |
| S4 | La región de entrada son las **cajas** de las zonas, no sus formas | ⚪ | Aproximar cada forma con unos cuantos rectángulos |
| S5 | **La mirada solo sigue al ratón dentro de las zonas.** Fuera, Wayland ya no nos cuenta dónde está | 🟡 | Un hecho `pointer.global` que en Linux venga del IPC del compositor. En Windows y macOS es una llamada |
| S7 | Sin ventanas normales, menús emergentes ni bloqueo de sesión | 🟡 | Más variantes de `Superficie`. En Wayland, `xdg_popup` y `ext-session-lock` |
| S8 | Si el compositor no da `Mailbox` ni `Immediate`, las láminas secundarias se frenan entre sí | ⚪ | Un hilo de presentación por lámina |
| S9 | Si la GPU se reinicia o se pierde el dispositivo, no se recupera | ⚪ | Atender `device lost` y rehacer `Gpu` y láminas; el estado de la escena no vive ahí, así que no se pierde nada |
| S10 | Al salir se llama a `exit`: no se destruye nada en orden | ⚪ | Que `atender` devuelva y soltar las láminas antes que las ventanas |

### Semántica (capas, gestos, reglas)

| # | Limitación | Gravedad | Cómo se arregla |
| --- | --- | --- | --- |
| L1 | Dos capas que fijan la misma propiedad: gana la última que cambió | 🟡 | Error al cargar la escena |
| L2 | La coreografía de una reclamación no depende de *desde cuál* se llega | 🟡 | `transition a -> b { … }`, como en el borrador |
| L3 | Los gestos no tienen parámetros (`point(side)`) | 🟡 | Fotogramas con expresiones en vez de números, evaluadas al empezar el gesto |
| L4 | Lo que emite un fotograma se atiende en el frame siguiente, y lo del *primero* se ignora | ⚪ | Procesar los sucesos del gesto en el mismo bucle que los demás |
| L5 | Los hechos son números. Hay textos vivos, pero no símbolos (`dropping: page`) | 🟡 | Un tipo símbolo internado: un número por nombre |
| L7 | El movimiento reducido arranca y no se cae, pero no está mirado con capturas | 🟡 | Mirarlo |

## Arregladas

| Fecha | Qué |
| --- | --- |
| 2026-09-19 | El shader recorría toda la lista en cada píxel → un quad por elemento |
| 2026-09-19 | Las transformaciones no se componían y solo había giro → afines que se multiplican |
| 2026-09-19 | Sin opacidad de grupo → capa intermedia |
| 2026-09-19 | Una textura girada usaba la pantalla entera como caja → sus cuatro esquinas transformadas |
| 2026-09-19 | Las zonas del ratón no seguían a las transformaciones → `zona_bajo` |
| 2026-09-19 | Una sola superficie fija, a escala 1, que tapaba el ratón → una por monitor, escala fraccional, región de entrada |
| 2026-09-19 | El hover del botón pasaba por la lógica → reglas que ejecuta el render |
| 2026-09-19 | **P1** · El texto era un mapa de bits fijo a escala 1 → `cosmic-text` con atlas de glifos a la escala de la lámina; textos vivos que cambia la lógica |
| 2026-09-19 | **P2** · Sin imágenes ni iconos → SVG (`resvg`), PNG y JPEG (`image`), iconos por nombre y teñido para los simbólicos |
| 2026-09-19 | Wayland estaba por todo `main.rs` y dentro del render → `src/plataforma/`, con `./portable.sh` de guarda |
| 2026-09-19 | Las fuentes se buscaban con `fc-match`, que solo existe en Linux → `fontdb`, leídas en otro hilo desde el arranque |
| 2026-09-19 | **T1** · Dar forma al texto y decodificar imágenes costaba frames (245 ms en frío) → un hilo «taller»; el render pide, no espera, y enseña lo que tenía. Ni un frame lento después del primero |
| 2026-09-19 | **T2** · No se podía medir un texto desde una expresión → `mide`: dos propiedades que rellena el render; con `Sigue`, una caja persigue a su rótulo con un muelle |
| 2026-09-19 | **L8 (a medias)** · Las escenas se escribían en Rust → el lenguaje v0: parser, comprobación de nombres, errores con su sitio y recarga en caliente |
| 2026-09-19 | **L6** · Una zona bajo transformaciones había que declararla a mano → en el lenguaje, una forma con nombre hereda las de sus grupos |
| 2026-09-19 | **G1** · Sin componentes, `repeat` ni layout → `component` con parámetros y nombres propios por copia, `repeat` con `$i` en los nombres, y `row`/`column` donde cada hijo va a su hueco con un muelle. `escenas/bandeja.plm` |
| 2026-09-19 | **G3** (y **L3** a medias) · Los destinos y los fotogramas eran números → expresiones que se evalúan al dispararse; un gesto puede depender de un hecho |
| 2026-09-19 | **G4** · Había que declarar antes de usar → cuatro vueltas: el orden es el de quien lee |
| 2026-09-19 | **G6** (a medias) · Sin colores con nombre → `let mint = #9ed6bd`, `mix` entre nombres, y `#fff`. Sigue sin haber propiedades de color |
| 2026-09-19 | **G8** · Cada recarga dejaba bytes sin liberar → nombres internados |
| 2026-09-19 | **G10** · Toda forma con nombre era zona → solo si una regla la nombra, lleva `active` o se declaró con `zone` |
| 2026-09-19 | **G5** (a medias) · Solo se decía el primer error → hasta ocho de una vez |
| 2026-09-19 | **G2 / L8** · Una escena de fichero no tenía lógica propia → Luau, en caja de arena, con tope de memoria y corte a los 2 s; se recarga en caliente |
| 2026-09-19 | **G13** · Los sucesos no llevaban datos → `emit opened(i)` y `on("opened", function(i) … end)` |
| 2026-09-19 | La lógica no se enteraba de los hechos que cambiaba una regla → `Evento::Hecho`, y `on("fact:open", …)` |
| 2026-09-19 | **U2** · La lógica solo se enteraba del sistema con órdenes que acaban → `spawn`/`kill` (una llamada por línea) y servicios de verdad detrás de `plataforma/`: `sys.watch`, `sys.call`. Hyprland por sus sockets |
| 2026-09-19 | Un reparto no se podía centrar ni pegar a la derecha sin saber lo que mide → `anchor:` |
| 2026-09-19 | Una superficie tenía un ancho fijo → `size: full, 44` y `screen.width` |
| 2026-09-19 | Lo que la lógica dejaba corriendo sobrevivía al programa (un `pactl subscribe` huérfano) → se para al salir y al recargar |
| 2026-09-19 | **B4 (casi)** · Probado por Abel con su ratón y sus teclas: pulsar un escritorio lleva a él, el volumen y el silencio se reflejan al momento, el título sigue a la ventana, y el clic atraviesa lo transparente de la barra (que era también lo que quedaba por ver de la región de entrada) |
| 2026-09-19 | **S6** · Sin rueda, arrastrar, mantener, otros botones, teclas ni cursor → `on scroll`, `on drag`, `on release`, `on hold … for`, `on press right`, `on key`, `cursor:`, `keyboard:`; lo del ratón se lee en `local.x`, `drag.dx`, `wheel`; un hecho puede valer una expresión |
| 2026-09-19 | **B7** · No se podía cambiar el volumen desde la barra → rueda sobre la píldora, arrastrar la barrita, pulsar el altavoz. La escena mueve la barrita en el acto y la lógica manda `wpctl`, como mucho uno cada 50 ms |
