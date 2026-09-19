# El lenguaje, v0 — lo que ya funciona

**Estado:** implementado (`src/lenguaje/`). Un fichero `.plm` entra y sale la misma `Escena` que antes se escribía en Rust. [[pleamar · 03 El lenguaje - borrador 0]] era el boceto en castellano; **esto es lo que hay de verdad, con las palabras clave en inglés**. Aún sin nombre propio.

```sh
pleamar --escena escenas/marea.plm      # se recarga sola al guardar el fichero
pleamar --comprobar escenas/marea.plm   # la lee, dice si está bien, y sale
```

Ejemplos completos: **`escenas/barra.plm` (una barra de verdad: escritorios, ventana, hora y volumen)**, `escenas/marea.plm` (la bolita y su tarjeta, 150 líneas), `escenas/cara.plm` (capas y gestos, sin lógica ninguna) y `escenas/bandeja.plm` (componentes, `repeat` y reparto: una lista de avisos que crece y encoge).

## La idea en una frase

**Todo lo que se escribe aquí lo ejecuta el render, solo.** No hay bucles ni variables que muten: todo termina y todo se comprueba al cargar. La lógica —fuera, en un `.luau` con el mismo nombre: [[pleamar · 10 La lógica en Luau]]— solo pone hechos, textos y sucesos.

## Forma general

```
scene Nombre {
    palabra cabecera … { bloque }     // un nodo
    nombre: valor, valor              // una propiedad
}
```

Un salto de línea o un `;` acaban una sentencia. `//` comenta hasta el final de la línea. Los nombres llevan puntos (`orb.x`, `note.0.title`).

**El orden es el que le convenga a quien lee.** El fichero se lee en cuatro vueltas —declaraciones; `let` y capas; dibujo; reglas—, así que una regla puede ir antes que la forma que nombra, y un `prop` al final. Solo un `let` tiene que ir antes de quien lo usa.

Números con unidad: `40`, `40px`, `34%` (= 0.34), `138deg` (a radianes), `320ms` y `14s` (duraciones). Colores `#151616` o `#fff`. Textos `"entre comillas"`.

## Declaraciones

| Sentencia | Qué es |
| --- | --- |
| `surface { size: 720, 224; anchor: top; margin: 40; level: top; reserve: 0; screens: "HDMI-A-1" }` | La ventana que pide. `size: full, 44` es todo el ancho del monitor; cuánto es se lee en `screen.width`. `anchor`: top, bottom, left, right, top_left…, center. `level`: background, bottom, top, overlay. `screens: all` o una lista. `keyboard: none | on_demand | exclusive` |
| `prop orb.x = 360 ~lively` | Una propiedad animada: un muelle. Sin `~`, `lively` |
| `pose eyes = 14` | Una propiedad de la pose: la que un gesto lleva de la mano |
| `fact open = false` | Algo que es verdad un rato. Lo ponen la lógica y las reglas |
| `event confirmed` · `event view_event ->` | Algo que ocurre. Con `->`, además sale hacia la lógica |
| `text notice.title = "Reunión"` | Un texto vivo: la lógica lo cambia |
| `image fox = icon "firefox", 48, 48` · `… = file "ruta.png", 48, 48` | Una imagen, y a qué tamaño se pinta como mucho |
| `measure label` | Crea `label.width` y `label.height`, que rellena el texto que lleve `measure: label` |
| `let panel.x = orb.x + 62` | Un nombre para una expresión |
| `let mint = #9ed6bd` · `let warm = mix(mint, #f84, 50%)` | Un nombre para un color |
| `spring bouncy = 170, 12` | Un muelle propio: rigidez, freno. De casa: lively, calm, quick, slow, eyes, pose. En línea: `~spring(170, 12)` |

## Expresiones

`+ - * /`, paréntesis, `< > <= >=`, `and or not`, `true false`. Verdad es más de 0.5.
Funciones: `min`, `max`, `abs`, `clamp(x, a, b)`, `smooth(a, b, x)`, `mix(a, b, t)`, `if(cond, a, b)` y **`vel(prop)`** —la velocidad de un muelle, que solo el render conoce—.
Valen como nombre: un `let`, un `prop`, un `fact`, una medida (`label.width`) y la presencia de una reclamación (`shape.rec`: 1 mientras gana).

## Dibujo

Se pinta en el orden en que se escribe.

```
body {                                  // formas fundidas en una silueta
    color: #151616                      // o gradient: x0, y0, x1, y1, #c0, #c1
    rim: 5%;  light: 3%, y0, alto;  shadow: dx, dy, difusa, alfa;  border: grosor, #color;  opacity: e
    ellipse orb { at: x, y; radius: r; scale: sx, sy }
    box { from: x, y; size: w, h; corner: r;  blend: neck }     // blend: cuánto se funde con lo anterior
}

ellipse { at: x, y; radius: r; color: #…; opacity: e }          // una forma suelta
box     { at: cx, cy | from: x, y;  size: w, h;  corner: r }
arc     { at: x, y; radius: r; span: 138deg; width: w }         // como «∩»
line    { from: x, y; to: x, y; width: w }
// a cualquiera: rotate: ángulo · stroke: grosor (solo el contorno: un círculo → un aro)

text notice.title { at: x, y; anchor: left center; width: 354; lines: 1; size: 20; weight: 500;
                    color: #…; opacity: e; align: left; line_height: 1.3; family: "Inter"; measure: label }
text "Descartar"  { at: x, y; anchor: center }
text number(volume * 100, 0, " %") { … }                     // un número que sale de una expresión: decimales y lo de detrás

image fox { at: x, y; size: w, h; opacity: e; tint: #9ed6bd }   // tint: para iconos simbólicos

group {                                 // el árbol: transforma, funde y recorta a sus hijos
    pivot: x, y;  rotate: e;  scale: s | sx, sy;  move: dx, dy
    opacity: e                          // se funden como UNA cosa
    clip inset 3 ellipse { … }          // vale hasta el final del grupo
    …hijos…
}
```

**Una forma con nombre es una zona si alguna regla la nombra** (o si lleva `active`, o se declaró con `zone`): se puede pulsar, y el ratón entra por ella. Un nombre puesto solo para leerse mejor no para el clic. Hereda las transformaciones de los grupos donde esté. `active: expr` la enciende y la apaga. `zone box whole { … }` es una zona que no se pinta. **La que se declara después queda encima.**

## Componentes, repeticiones y repartos

```
component Note(i) {                     // parámetros: números, colores, "textos", o el nombre de un texto vivo
    size: 360, 58                       // cuánto ocupa, para quien lo reparta
    prop lit = 0 ~quick                 // cada copia tiene el suyo
    body { color: mix(#1b1c1c, #2b2d2d, lit); box hit { from: 0, 0; size: 360, 58; corner: 14 } }
    text note.$i.title { at: 16, 38; anchor: left center }
    on enter hit { lit: 1 ~quick }      // …y su propia zona y sus propias reglas
    on press hit { emit opened.$i }
}

Note(3) { move: 20, 40 }                // una copia es un grupo: move, rotate, scale, opacity

repeat i in 0..5 { event opened.$i -> } // se despliega al cargar; `$i` entra en los nombres

column list ~calm {                     // o `row`. Con muelle, cada hijo VA a su hueco
    at: 180, 66;  gap: 8;  padding: 0;  align: start | center | end
    anchor: right                       // qué parte cae sobre `at`: left, center, right · top, middle, bottom
    fill: #222;  corner: 12             // un fondo del tamaño de lo que contenga
    repeat i in 0..5 { Note(i) { show: count > i } }
    space 6
}
box { from: 180, 66 + list.height + 10; size: head.width, 2 }   // con nombre, se puede medir
```

- Lo que una copia declara por dentro (`prop`, formas con nombre, medidas) **es suyo**: dos copias no se pisan, y las reglas de dentro hablan de las suyas.
- **No hay motor de layout.** El sitio de cada hijo es una expresión —lo que ocupan los anteriores—: si uno crece, los demás se corren; con `~muelle`, se corren animados. `show:` decide si un hijo está: ocupa y se ve, o ni lo uno ni lo otro, y con muelle también eso es un viaje.
- Dentro de un reparto un hijo no dice dónde va. Saben cuánto ocupan `box`, `ellipse`, `image`, `text` (se mide solo), otro `row`/`column`, y un `group` o un componente con `size:`.
- Una lista de longitud variable es hoy una de capacidad fija con `show:`. Ver `escenas/bandeja.plm`.

## Capas — quién gana

```
layer shape ~quick {                    // de más a menos prioridad
    rec    while recording
    alert  for 700ms after urgent
    happy  for 620ms after confirmed, done
    broken from failed until forgotten
    eyes                                // por defecto
}
layer card ~calm {
    shown while open { orb.x: 140 ~lively;  panel.w: 406 ~calm after 70ms;  neck: 0 ~calm after 560ms }
    rest             { orb.x: 360 ~lively after 150ms;  panel.w: 0 ~calm after 90ms }
}
```

Gana la primera reclamación que se cumple; cuando deja de cumplirse se ve la siguiente, sola. El bloque de una reclamación es su coreografía: adónde va cada propiedad, con qué muelle y con qué retraso. **El destino es una expresión**, y se evalúa cuando le llega la hora: `r: base * 3 ~lively`. `shape.rec` se puede usar en cualquier expresión.

## Reglas — qué hace cambiar las cosas

```
on press view            { open = false; impulse orb.y -620; emit view_event }
on press right dot       { emit menu }               // o `middle`
on release dot           { r: 30 ~lively }           // se suelta lo que se pulsó ahí, esté donde esté ya el ratón
on hold dot for 500ms    { emit held }               // lleva ese rato pulsada
on enter view            { glow: 1 ~quick }          // el :hover de CSS
on leave view            { glow: 0 ~quick }
on hover orb for 320ms   { open = true }
on away whole for 420ms  { open = false }            // estuvo encima y lleva ese rato fuera
on scroll sound          { emit volume_step(wheel) } // la rueda, en cualquier zona que tenga debajo
on drag track            { volume = clamp(local.x / 64, 0, 1) }   // se mueve con el botón puesto
on key Escape            { open = false }            // la superficie tiene que pedir teclado: `keyboard: on_demand`
on key Ctrl+k            { toggle open }             // con modificadores: Ctrl+, Alt+, Super+ (Mayús va en la propia tecla)
on submit query          { emit launch(sel) }        // Intro dentro del campo `query`
on focus                 { glow: 1 ~quick }          // la superficie gana el teclado…
on blur                  { open = false }            // …o lo pierde: han pulsado en otro sitio
on drop tray             { emit dropped }            // sueltan algo de otra aplicación sobre la zona
on toggle                { toggle open; focus query } // un suceso, que puede venir de fuera (ver abajo)
on idle for 14s while not open { asleep = true }
on confirmed             { play joy }
every 2.5s..7s while awake { play yawn }
```

Efectos: `hecho = expresión` (se evalúa al dispararse), `toggle hecho`, `emit suceso` o con carga `emit opened(i)`, `impulse prop velocidad`, `play gesto`, `focus campo` (le da el cursor de escribir), `blur`, y `prop: valor ~muelle after 70ms`.

**Lo que una regla puede leer del ratón**, como si fueran hechos: `pointer.x`, `pointer.y` (en la superficie), `local.x`, `local.y` (**dentro de la zona**: en un reparto, (0, 0) es la esquina del hueco, esté donde esté en pantalla), `drag.dx`, `drag.dy` (desde que se pulsó) y `wheel` (muescas; positivo, hacia arriba). Un arrastre sigue aunque el ratón se salga de la zona, hasta soltar.

Una forma puede llevar `cursor: pointer | text | grab | grabbing`. **Un `row` o `column` con nombre es también una zona** —su caja entera, debajo de las de sus hijos—: así la rueda vale en toda una píldora.

Su tamaño (`list.width`, `list.height`) se puede leer **en cualquier parte del fichero, también antes** de donde se declara: el panel que envuelve a una lista puede perseguir su alto (`follow tall = 84 + list.height`).

> Ojo con lo que se arrastra dentro de un reparto anclado: si algo de dentro cambia de ancho mientras tanto (un «54 %» que pasa a «100 %»), el reparto se recoloca y la zona se mueve debajo del ratón. Dale ancho fijo a lo que cambie (`width: 42; align: right`).

## Escribir: `input`

```
text query = ""
input query { at: 48, 44; width: 504; size: 20; color: ink
              placeholder: "Busca una aplicación…"; selection: #2f5f52 }
```

Un campo de una línea que edita **el render**: cada tecla se ve en el frame siguiente, esté como esté la lógica. El campo se llama como el `text` que edita, y ese mismo nombre es su zona (pulsar le da el foco y coloca el cursor; arrastrar selecciona). Sabe lo de siempre: flechas, Inicio y Fin, Ctrl+flecha por palabras, Mayús para seleccionar, Ctrl+A, Ctrl+C / X / V contra el portapapeles del sistema, Retroceso y Supr, y repite la tecla que se deja pulsada. Si el texto no cabe, se desliza para que el cursor se vea.

La lógica se entera de cada cambio (`text:query`) y del Intro (`submit:query`); lo que no es escribir —Escape, las flechas de arriba y abajo— sigue llegando a `on key`.

**El teclado, solo cuando hace falta.** `keyboard: exclusive while open` en la `surface`: mientras `open` es falso la superficie no pide teclado, y el escritorio sigue siendo de quien era.

## Imágenes que elige la lógica

```
text  pic.$i  = ""                        // la lógica pone aquí «firefox», o una ruta que empiece por /
image icon.$i = from pic.$i, 24, 24       // y la imagen es la que ese texto diga
```

Cuando el texto cambia, el taller busca la imagen nueva en su hilo y el render sigue enseñando la anterior hasta que llega. Un texto vacío es ninguna imagen.

## Órdenes desde fuera

```
pleamar --decir lanzador "emit toggle"
```

Cada escena en marcha escucha en un socket con su nombre (el del fichero). `emit suceso` o `emit suceso 3` dispara sus reglas como si lo hubiera emitido la lógica. También `fact hecho valor`, `text nombre lo que ponga` (la lógica se entera, como si lo hubiera escrito alguien), `focus campo`, `quit`, y **`get nombre`, que contesta** con lo que valga ese hecho, texto o propiedad: `pleamar --decir lanzador "get open"` → `1`.

**Un atajo global es esto**: un bind del compositor que ejecuta esa orden. En Hyprland:

```lua
hl.bind("SUPER + space", hl.dsp.exec_cmd('pleamar --decir lanzador "emit toggle"'))
```

## Lo que lleva sola

```
blink eyelid every 2.4s..6s for 170ms
wave  breath = sleep * 1.3 at 1.7                    // amplitud · sin(1.7 t)
spin  angle by 0.9                                   // += 0.9 por segundo
follow chip.w = label.width + 32                     // persigue a la expresión con su muelle
look  gaze.x, gaze.y at orb.x, orb.y reach 5, 3.2 within 140 rest 3, 0.6
```

## Gestos

```
gesture nod reflex {                    // clases: ambient < posture < reflex < asked < state
    130ms out_quad         { look.y: 4; eyes: 10 }
    170ms out_back hold 60ms emit shutter { eyes: 15 }
    160ms                               // sin bloque: vuelta a la base
}
posture working while searching { … }   // se repite sola mientras sea verdad
```

Un gesto solo corta a otro de su clase o inferior. Lo que un fotograma no nombra vuelve a su base. Curvas: linear, in_quad, out_quad, in_cubic, out_cubic, in_out_sine, out_back.

## Errores

Con línea, columna, el trozo de fichero y, si se parece a algo, una sugerencia. **Se dicen todos los que se encuentren** (hasta ocho), no solo el primero:

```
marea.plm:91:28: no hay nada que se llame «pannel.h». ¿Querías decir «panel.h»?
  91 |             size: panel.w, pannel.h
                                  ^
```

## Recarga en caliente

Al guardar el fichero se vuelve a leer (una escena como Marea, en menos de un milisegundo). Si está bien, sustituye a la vieja **sin perder nada**: las propiedades conservan valor y velocidad, los hechos y los textos siguen como estaban, y si el fichero cambió un destino, se va hacia él con su muelle. Si está mal, dice por qué y la vieja sigue.
