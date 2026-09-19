# Cómo funciona hoy

Lo que ya corre en `~/Proyectos/pleamar`. Rust, `wgpu` 30, `cosmic-text` para el texto, `resvg` e `image` para las imágenes; y solo en Linux, `smithay-client-toolkit` 0.21 (layer-shell).

## Tres hilos

| Hilo | Hace | No hace |
| --- | --- | --- |
| **Plataforma** (`plataforma/wayland.rs`) | Una superficie layer-shell por monitor (y por los que se enchufen), su escala, y el ratón | Nada más |
| **Lógica** (`logica.rs`) | Corre un `Guion`: recibe eventos con nombre, cuenta hechos y sucesos, pide gestos | No anima. No sabe de coordenadas. Se bloquea a propósito para el ensayo |
| **Render** (`render.rs`) | Dueño de los muelles y del reloj. Evalúa, pinta y avisa | No sabe qué está pintando |

El ratón va **al render**, que es quien sabe qué hay debajo; a la lógica le llega ya como `Entra("ver")` o `Pulsa("orbe")`.

## Una escena son datos (`escena.rs`)

- **Propiedades** con nombre. Cada una es un muelle (posición, velocidad, destino). Si la escena se sustituye, las del mismo nombre sobreviven.
- **Expresiones** puras (`Expr`): constantes, propiedades, velocidades, `+ − × ÷`, `min`, `max`, `abs`, `suave`. En Rust se escriben con operadores sobrecargados: `orbe_x + 62.0`.
- **Lista de dibujo**, en orden:

| Instrucción | Qué hace |
| --- | --- |
| `Grupo` | Empieza un cuerpo; opcionalmente con sombra |
| `Forma` | Añade una elipse o caja, fundida con lo anterior (`fusion` = radio del mínimo suave) |
| `Relleno` | Pinta el cuerpo acumulado: sombra, color, luz y filo |
| `Recorte` | Lo siguiente se recorta a esta forma |
| `Plano` | Una forma suelta de color plano |
| `Textura` | Un trozo del atlas, colocado en pantalla |

- **Comportamientos** que el render lleva solo: `Parpadeo`, `Onda`, `Avance` (una aguja que da vueltas), `Mirada`.
- **Zonas**: una forma con nombre y una condición `activa`.
- **Hechos y sucesos**: la frontera. La lógica manda `Hecho("grabando", sí)`, `Suceso("confirmado")` o `Gesto("asentir")`, y nada más. Los hechos se leen en las expresiones (`Expr::H`), con `y`, `o`, `no`, `mayor`.
- **Capas**: reclamaciones en orden de prioridad — `mientras <expr>`, `N ms tras <suceso>`, `desde … hasta`, o por defecto. Gana la primera que se cumple. Cada reclamación tiene una **presencia** (una propiedad que va a 1 cuando gana), para pintar según quién mande, y puede **fijar** propiedades con su muelle y su retraso: eso es una coreografía.
- **Gestos**: fotogramas con duración, curva (`OutBack`, `InQuad`…) y aguante, sobre las propiedades **de pose**. Lo que un fotograma no nombra vuelve a su base; al acabar, los muelles recogen la pose. **Clases**: `Estado > Pedido > Reflejo > Postura > Ambiente`; uno solo corta a otro de su clase o inferior. Una **postura** se repite sola mientras algo sea verdad.
- **Reglas**: `Entra`, `Sale`, `Pulsa`, `Encima{durante}`, `Fuera{durante}`, `Quieto{durante}`, `Cada{a..b}`, `Al(suceso)` → efectos (`Animar`, `Hecho`, `Alternar`, `Suceso`, `Impulso`, `Gesto`). **Todo lo ejecuta el render.**
- **Movimiento reducido** (`--movimiento-reducido`): los muelles se posan y cada gesto enseña, quieto, el fotograma que más se aparta de la base.

## El renderer: un quad por elemento (`render.rs`, `forma.wgsl`, `formas.rs`)

Cada frame, el render recorre la lista de dibujo y la **compone** en dos tablas que sube a la GPU:

- **formas**, con sus expresiones ya evaluadas (16 números cada una);
- **elementos** (48 números): un *cuerpo* —una o varias formas fundidas, con su pintura, borde, luz y sombra— o un trozo del atlas. Cada uno lleva su **caja envolvente**, calculada en CPU con la misma geometría, ensanchada por la sombra y el fundido y recortada por sus recortes.

Se pinta con **una sola llamada** instanciada: un quad por elemento, en orden, con mezcla premultiplicada. Un píxel solo ejecuta las formas del elemento que lo cubre. Lo invisible (`alfa` 0, cajas vacías) no genera ni quad.

| Primitiva | Notas |
| --- | --- |
| `Elipse`, `Caja` | como antes |
| `Arco` | como «∩»; `apertura` es medio ángulo en radianes |
| `Segmento` | línea de extremos redondos |
| `.trazo(grosor)` | solo el contorno: un círculo se vuelve un **aro** |
| `.girada(ángulo)` | sobre su centro; positivo, sentido del reloj |
| `Transformar` | gira, escala y mueve todo lo que venga después alrededor de un pivote. Es una **pila que se compone**: son matrices afines que se multiplican, así que un giro dentro de un grupo que escala dentro de otro que gira hace lo que se espera. Las zonas del ratón pueden vivir bajo las mismas transformaciones (`zona_bajo`) |
| `Opacidad` | todo lo de dentro se pinta **aparte, en una capa**, y se funde como una sola cosa. Sin capa, fundir pieza a pieza deja ver lo de detrás a través de lo de delante. Solo gasta capa mientras está a medio fundir (ni a 0 ni a 1); hasta cuatro a la vez |
| `Recorte` | ahora es una **pila** (hasta cuatro): un hijo se recorta a su padre y a su abuelo |
| `Pintura::Lineal`, `borde` | degradado entre dos puntos; borde interior del cuerpo |

`formas.rs` tiene la geometría una sola vez para tres usos: codificar para la GPU, saber qué hay bajo el ratón y calcular cajas.

**Medido** (RTX 2060, 720×300, `--escena enjambre --sin-vsync`, ms por frame):

| formas | intérprete por píxel (antes) | por elementos |
| --- | --- | --- |
| 12 | 0,27 | 0,25 |
| 60 | 0,53 | 0,28 |
| 200 | 1,37 | 0,30 |
| 600 | 3,66 | 0,42 |
| 2000 | no cabía (tope 1024) | 0,57 |

Ojo con lo que dice esta tabla: **el intérprete viejo no iba tan mal como se temía** a este tamaño de superficie; 600 formas seguían cabiendo de sobra en un frame. Donde se habría roto es a pantalla completa (9,6 veces más píxeles) o en una gráfica integrada. El nuevo, además, deja de tener tope.

## Texto e imágenes (`texto.rs`)

Todo lo que acaba siendo un trozo de atlas. Tres crates de Rust puro que existen en los tres sistemas: `cosmic-text`, `image` y `resvg`.

- **`Instr::Texto`**: contenido fijo o **vivo** (un texto con nombre que la lógica cambia con `c.texto("aviso.título", …)`), un punto y un `ancla` —qué parte del texto cae sobre él: (0.5, 0.5) lo centra—, un ancho opcional para partir en líneas, y un `Estilo` (familia, tamaño, peso, color, interlínea, alineado, máximo de líneas con puntos suspensivos).
- El **tipógrafo** da forma al texto —ligaduras, derecha a izquierda, fuentes de reserva, emoji en color— y guarda la maqueta: mientras no cambien texto, estilo ni ancho, no se repite. Cada glifo se pinta una vez, **a la escala de la lámina más fina**, en un atlas de 2048² que comparte con las imágenes. Una letra es una máscara que el shader tiñe; un emoji trae su color.
- Cada glifo es un elemento más: se recorta, se transforma y se funde como todo. El origen del texto se redondea a píxeles de verdad para que no salga blando.
- **`Instr::Imagen`**: una ruta o un icono por nombre (`Fuente::Icono("firefox")`; encontrarlo es cosa de la plataforma). Los SVG se pintan al tamaño exacto por la escala; con `tinte`, la forma se pinta de un color: lo que quiere un icono simbólico.
- **El taller.** Dar forma, pintar glifos y decodificar imágenes ocurre en su propio hilo, que además es quien lee las fuentes del sistema al arrancar. El render **pide y no espera**: mientras llega una maqueta enseña la última que hubo en ese sitio, y lo que ya está en la escena se encarga antes de que exista la ventana. Primer frame a los ~170 ms, y ninguno lento después.
- **Medir.** Un `Texto` con `mide` deja su ancho y su alto en dos propiedades de solo lectura. Con `Comportamiento::Sigue` —una propiedad que persigue a una expresión con su muelle— una caja crece con su rótulo.
- Hay un chivato permanente: cualquier frame que pase de 2,4 periodos sale por consola con su hora.

## La frontera de plataforma (`src/plataforma/`)

Lo único del programa que sabe qué es Wayland. Una plataforma pone las superficies que pide una escena y se las entrega al render como láminas; le cuenta el ratón, la escala y los monitores que van y vienen; le da una `Ventana` con un método (`region_de_entrada`); y sabe encontrar un icono por su nombre. Hoy solo hay `wayland.rs`. En cualquier otro sistema el núcleo compila y dice que aún no sabe poner ventanas. **`./portable.sh` lo comprueba contra Linux, Windows y macOS**, y es la guarda de que nada de un sistema se cuele fuera de aquí.

## Servicios (`plataforma/mod.rs`, `plataforma/hyprland.rs`)

Lo que pasa en el sistema llega a la lógica por `plataforma::servicio(nombre, avisar)`: un hilo que escucha y avisa con un `Valor` —un JSON en pequeño: nulo, sí/no, número, texto, lista, mapa— que Luau recibe como tabla. `plataforma::orden(nombre, args)` es el camino de vuelta. Hoy contesta Hyprland, hablado por sus dos sockets con `std` y nada más: uno para preguntar y mandar (`j/workspaces`, `dispatch …`), otro por el que cuenta lo que pasa. Un sistema sin ese servicio dice que no lo tiene.

## Superficies (`plataforma/wayland.rs`, `gpu.rs`)

La escena declara la superficie que quiere: tamaño en píxeles **lógicos**, ancla, margen, nivel (fondo, debajo, encima, sobre todo), cuánto sitio reserva, y en qué pantallas (`Todas` o una lista). El hilo de Wayland pone una en cada monitor que toque, las quita cuando el monitor se va y las pone cuando uno llega. Cada una pasa al render como una **lámina** cuando el compositor la configura.

- **Escala.** Con `wp_fractional_scale` y `wp_viewporter`: el tamaño lógico no cambia, y la lámina se pinta con `tamaño × escala` píxeles de verdad. El shader divide la posición por la escala y suaviza los bordes en píxeles de verdad, así que a escala 2 nada sale blando —salvo el texto, que es un mapa de bits—. Sin esos protocolos, cae a la escala entera de toda la vida.
- **Ritmo.** Solo una lámina —la del monitor más rápido— espera a la pantalla; las demás presentan sin bloquear. Si esperasen todas, un monitor a 60 Hz frenaría a otro a 165.
- **Región de entrada.** Cada frame en que cambian, las cajas de las zonas activas se le dan al compositor como región de entrada, justo antes de presentar: lo demás de la superficie, aunque sea suya, deja pasar el clic.
- La GPU (dispositivo, tubería, almacenes) se crea con la primera lámina que llega.

## Escribir (`render.rs`, `texto.rs`)

Un `input` lo edita **el render**, no la lógica: la tecla entra, `Edicion` cambia el texto y el cursor, y se ve en ese mismo frame aunque la lógica lleve un segundo atascada. El taller devuelve con cada maqueta dónde cae cada letra (`cursores`), y con eso se coloca el cursor, se pinta la selección y se sabe qué letra hay bajo el ratón. La lógica recibe el texto ya cambiado (`Evento::Texto`) y el Intro (`Evento::Envia`).

La repetición de tecla es nuestra (400 ms, luego una cada 33): así es igual en todos los sistemas. El portapapeles es `arboard`, que existe en los tres.

## Órdenes desde fuera (`plataforma/mod.rs`)

Un hilo escucha en `$XDG_RUNTIME_DIR/pleamar-ESCENA.sock`; `pleamar --decir ESCENA "emit toggle"` escribe una línea y se va (2 ms). La orden entra al render como `ARender::SucesoDeFuera`, por la misma puerta que los sucesos de la lógica. Es `cfg(unix)`: en Windows será una tubería con nombre.

## Soltar desde otra aplicación (`plataforma/wayland.rs`)

`wl_data_device`: al entrar un arrastre se mira si trae `text/uri-list` o `text/plain`, y al soltarlo se lee en un hilo aparte —quien lo ofrece puede tardar— y llega como `ARender::Soltado`. El render mira qué zona hay debajo y dispara `on drop zona`.

## Reposo

Si ninguna propiedad se mueve y ningún comportamiento está vivo, el render no pinta: espera un mensaje, un retraso que venza o el próximo parpadeo. Cero frames.

## Cómo se prueba

```sh
./target/release/pleamar                 # Marea; botón derecho la cierra
./target/release/pleamar --escena isla   # la isla
./target/release/pleamar --escena cara   # la cara de Marea: capas y gestos, con su guion
./target/release/pleamar --escena muestrario   # degradado, borde, transformaciones anidadas, textura girada, opacidad de grupo
./target/release/pleamar --demo --segundos 9
./target/release/pleamar --ingenuo       # la lógica bloquea al render, como QtQuick
./target/release/pleamar --raton "360,90@500 500,172@1100 pulsa@3200 fuera@4500"
```

Sale en `HDMI-A-1`. La gráfica de abajo es una barra por frame; la franja roja, el tiempo con la lógica bloqueada. **No medir mientras se captura con `grim`**: provoca frames caídos.

## Deudas conocidas

- Todas están en [[pleamar · 08 Limitaciones conocidas]]. Las que más pesan:
- Las escenas se escriben en Rust y se compilan con el programa.
- Sin layout.
- Un grupo con opacidad dentro de otro no tiene capa propia: multiplica. Y si hay más de cuatro fundiéndose a la vez, los que sobran también multiplican.
- Con escala distinta en cada eje, el suavizado del borde es aproximado.
- El degradado y la luz viven en el espacio del grupo, no en el de la forma: si la forma gira *por sí misma* (`.girada`), el degradado no gira con ella.
- Los primeros frames tras despertar no esperan al vsync: el reloj de animación debería ir con el tiempo de presentación.
- Un frame de cada ~400 se cae a 33 ms. Sin investigar.
