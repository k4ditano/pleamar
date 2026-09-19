# El lenguaje, v0 — lo que ya funciona

**Estado:** implementado (`src/lenguaje/`). Un fichero `.plm` entra y sale la misma `Escena` que antes se escribía en Rust. [[pleamar · 03 El lenguaje - borrador 0]] era el boceto en castellano; **esto es lo que hay de verdad, con las palabras clave en inglés**. Aún sin nombre propio.

```sh
pleamar --escena escenas/marea.plm      # se recarga sola al guardar el fichero
pleamar --comprobar escenas/marea.plm   # la lee, dice si está bien, y sale
```

Ejemplos completos: `escenas/marea.plm` (la bolita y su tarjeta, 150 líneas) y `escenas/cara.plm` (capas y gestos, sin lógica ninguna).

## La idea en una frase

**Todo lo que se escribe aquí lo ejecuta el render, solo.** No hay bucles ni variables que muten: todo termina y todo se comprueba al cargar. La lógica —fuera— solo pone hechos, textos y sucesos.

## Forma general

```
scene Nombre {
    palabra cabecera … { bloque }     // un nodo
    nombre: valor, valor              // una propiedad
}
```

Un salto de línea o un `;` acaban una sentencia. `//` comenta hasta el final de la línea. Los nombres llevan puntos (`orb.x`). **Lo que se usa tiene que estar declarado más arriba.**

Números con unidad: `40`, `40px`, `34%` (= 0.34), `138deg` (a radianes), `320ms` y `14s` (duraciones). Colores `#151616`. Textos `"entre comillas"`.

## Declaraciones

| Sentencia | Qué es |
| --- | --- |
| `surface { size: 720, 224; anchor: top; margin: 40; level: top; reserve: 0; screens: "HDMI-A-1" }` | La ventana que pide. `anchor`: top, bottom, left, right, top_left…, center. `level`: background, bottom, top, overlay. `screens: all` o una lista |
| `prop orb.x = 360 ~lively` | Una propiedad animada: un muelle. Sin `~`, `lively` |
| `pose eyes = 14` | Una propiedad de la pose: la que un gesto lleva de la mano |
| `fact open = false` | Algo que es verdad un rato. Lo ponen la lógica y las reglas |
| `event confirmed` · `event view_event ->` | Algo que ocurre. Con `->`, además sale hacia la lógica |
| `text notice.title = "Reunión"` | Un texto vivo: la lógica lo cambia |
| `image fox = icon "firefox", 48, 48` · `… = file "ruta.png", 48, 48` | Una imagen, y a qué tamaño se pinta como mucho |
| `measure label` | Crea `label.width` y `label.height`, que rellena el texto que lleve `measure: label` |
| `let panel.x = orb.x + 62` | Un nombre para una expresión |
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

image fox { at: x, y; size: w, h; opacity: e; tint: #9ed6bd }   // tint: para iconos simbólicos

group {                                 // el árbol: transforma, funde y recorta a sus hijos
    pivot: x, y;  rotate: e;  scale: s | sx, sy;  move: dx, dy
    opacity: e                          // se funden como UNA cosa
    clip inset 3 ellipse { … }          // vale hasta el final del grupo
    …hijos…
}
```

**Una forma con nombre es también una zona**: se puede pulsar, y el ratón entra por ella. Hereda las transformaciones de los grupos donde esté. `active: expr` la enciende y la apaga. `zone box whole { … }` es una zona que no se pinta. **La que se declara después queda encima.**

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

Gana la primera reclamación que se cumple; cuando deja de cumplirse se ve la siguiente, sola. El bloque de una reclamación es su coreografía: adónde va cada propiedad, con qué muelle y con qué retraso. `shape.rec` se puede usar en cualquier expresión.

## Reglas — qué hace cambiar las cosas

```
on press view            { open = false; impulse orb.y -620; emit view_event }
on enter view            { glow: 1 ~quick }          // el :hover de CSS
on leave view            { glow: 0 ~quick }
on hover orb for 320ms   { open = true }
on away whole for 420ms  { open = false }            // estuvo encima y lleva ese rato fuera
on idle for 14s while not open { asleep = true }
on confirmed             { play joy }
every 2.5s..7s while awake { play yawn }
```

Efectos: `hecho = true|false|n`, `toggle hecho`, `emit suceso`, `impulse prop velocidad`, `play gesto`, y `prop: valor ~muelle after 70ms`.

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

Con línea, columna, el trozo de fichero y, si se parece a algo, una sugerencia:

```
marea.plm:91:28: no hay nada que se llame «pannel.h». ¿Querías decir «panel.h»?
  91 |             size: panel.w, pannel.h
                                  ^
```

## Recarga en caliente

Al guardar el fichero se vuelve a leer (una escena como Marea, en menos de un milisegundo). Si está bien, sustituye a la vieja **sin perder nada**: las propiedades conservan valor y velocidad, los hechos y los textos siguen como estaban, y si el fichero cambió un destino, se va hacia él con su muelle. Si está mal, dice por qué y la vieja sigue.
