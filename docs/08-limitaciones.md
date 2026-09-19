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

### El lenguaje

| # | Limitación | Gravedad | Cómo se arregla |
| --- | --- | --- | --- |
| G12 | **Una lista de longitud variable es una de capacidad fija** con `show:`. Los límites de `repeat` son números, y la lógica no puede darle a la escena una lista de cosas | 🔴 | `repeat item in notes { … }` sobre un modelo que ponga la lógica, con clave por elemento para que al reordenar cada uno viaje a su sitio nuevo. Pide copias que nazcan y mueran en marcha: hoy todo se despliega al cargar |
| G13 | Los sucesos no llevan datos: para saber qué aviso se pulsó hay un suceso por índice (`opened.$i`) | 🟡 | Sucesos con carga: `emit opened(i)`, y en la lógica `on("opened", function(i) … end)` |
| G14 | Los parámetros de un componente van por posición, sin tipo, sin valor por defecto, y no hay hueco para hijos (`children`) | 🟡 | `component Card(title, tone = mint) { … slot … }` |
| G15 | El reparto solo sabe de fila y columna con hueco, relleno y alineado. Sin salto de línea, sin «ocupa lo que quede», sin mínimos ni máximos. Un `group` sin `size:` ocupa lo que su último hijo, por casualidad más que por diseño | 🟡 | `grow:` en un hijo (reparte el sobrante del `size:` del reparto), `wrap`, y que un grupo sin tamaño ocupe la unión de sus hijos |
| G16 | El sitio de cada hijo es la suma de los anteriores, escrita entera: con *n* hijos las expresiones crecen como *n²*. Veinte no se notan; doscientos sí | ⚪ | Subexpresiones compartidas: que `Expr` sea un grafo y cada suma parcial se evalúe una vez por frame |
| G17 | Lo que mide un texto llega un frame tarde, así que un reparto con textos se asienta en uno o dos frames al cargar. Con muelle no se ve; sin él, un parpadeo | ⚪ | Ver T12 |
| G2 | **Una escena de fichero no tiene lógica propia**: la acompaña un guion mínimo en Rust que solo escucha | 🔴 | Punto 5: un bloque o un fichero Luau al lado, con `on("view_event", …)`, `fact.open = true`, `text["notice.title"] = …` |
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
| T8 | La lógica pone cadenas; la escena no sabe formatear un número (`"{volumen} %"`) | 🟡 | `Contenido::Formato` con expresiones dentro. Llegará con el lenguaje |
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
| S1 | El tamaño de la superficie lo declara la escena y es fijo | 🟡 | Hoy se esquiva declarándola grande: la región de entrada deja pasar el clic. De verdad: que la escena pueda cambiar su `Superficie` y la plataforma la reconfigure |
| S2 | Todas las superficies pintan **la misma escena con el mismo estado**. No hay una instancia por monitor | 🟡 | Es del lenguaje: `per screen { … }`, con propiedades y hechos propios por instancia |
| S3 | **Sin probar en dos monitores de verdad a distinto ritmo** (165 y 60 Hz) | 🟡 | `--pantalla todas` cuando a Abel le venga bien verlo en DP-3 |
| S4 | La región de entrada son las **cajas** de las zonas, no sus formas | ⚪ | Aproximar cada forma con unos cuantos rectángulos |
| S5 | **La mirada solo sigue al ratón dentro de las zonas.** Fuera, Wayland ya no nos cuenta dónde está | 🟡 | Un hecho `pointer.global` que en Linux venga del IPC del compositor. En Windows y macOS es una llamada |
| S6 | Sin teclado, rueda, arrastrar, mantener pulsado ni forma del cursor. El botón derecho cierra el programa | 🔴 | Ampliar `Disparador` (`Rueda`, `Arrastra`, `Tecla`) y la interfaz de plataforma. Es lo siguiente después del parser |
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
| L8 | La lógica es un `trait` de Rust, no Luau (ver G2) | 🔴 | Punto 5 |

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
