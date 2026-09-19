# La lógica, en Luau

**Estado:** implementado (`src/logica_luau.rs`). Si al lado de `marea.plm` hay un `marea.luau`, esa es su lógica. Se recarga sola al guardarla, igual que la escena.

Ejemplos: `escenas/marea.luau` (avisos que llegan) y `escenas/bandeja.luau` (una lista de datos que crece y encoge).

## Qué puede hacer, y qué no

La lógica **no anima, no pinta y no sabe de coordenadas**. Vive en su propio hilo; el render no la espera nunca. Solo cruza la frontera:

```lua
fact.open = true                          -- decir qué es verdad  (se lee como número: 1 o 0)
text["notice.title"] = "Reunión en 5 min" -- decir qué pone un texto vivo
emit("confirmed")                         -- que ha pasado algo
play("joy")                               -- pedir un gesto (la escena lo concederá o no, según su clase)

on("view_event", function(n) … end)       -- un suceso que la escena deja salir (`event x ->`), con su carga
on("press:view", …)  on("release:view", …)  on("enter:orb", …)  on("leave:orb", …)
on("scroll:sound", function(notches) … end)   on("key", function(name, text) … end)
on("text:query", function(value) … end)   -- alguien escribió en el campo `query`; `text.query` ya vale eso
on("submit:query", function(value) … end) -- Intro en el campo
on("focus", …)  on("blur", …)             -- la superficie gana o pierde el teclado
on("drop:tray", function(data, mime) … end)  -- soltaron algo de otra aplicación: texto, o una lista de `file://…`
focus("query")                            -- darle el cursor de escribir a un campo; `focus()` se lo quita
on("layer:card", function(claim) … end)   -- una capa cambió de manos
on("fact:open", function(v) … end)        -- una REGLA de la escena cambió un hecho
on("demo", …)                             -- el tic de `--demo`

local t = every(1000, function() … end)   -- temporizadores, en milisegundos
after(500, function() … end)
cancel(t)

run("date", { "+%H:%M" }, function(out, code) … end)   -- una orden del sistema; contesta al acabar
local id = spawn("pactl", { "subscribe" }, function(line) … end)   -- una que NO acaba: una llamada por línea
kill(id)

sys.watch("workspaces", function(w) … end)   -- un servicio del sistema; devuelve si este sistema lo tiene
sys.call("workspaces.focus", 3)               -- pedirle algo a un servicio
log("lo que sea", 42)
busy(600)                                 -- trabajo de mentira, para ver que al render le da igual
```

Un nombre mal escrito es un error al momento, con sugerencia: `la escena no tiene ningún hecho «opne». ¿Querías decir «open»?`

## Los servicios del sistema

`sys.watch(nombre, fn)` escucha algo que pasa en el sistema. La función recibe el estado de ahora y luego cada cambio, como una tabla. **Los nombres son los mismos en todos los sistemas**; quién contesta es cosa de `src/plataforma/`. Si este sistema no tiene ese servicio, `sys.watch` devuelve `false` y la escena decide qué hacer sin él.

| Servicio | Lo que cuenta | Quién lo da hoy |
| --- | --- | --- |
| `workspaces` | `{ active = 3, list = { { id, name, windows, monitor }, … } }` | Hyprland, por sus sockets (sin lanzar `hyprctl`) |
| `window` | `{ title, class }` | Hyprland |
| `sys.call("workspaces.focus", n)` | ir a un escritorio | Hyprland |
| `audio` | `{ volume = 0.54, muted = false }` | Linux: PipeWire (`wpctl`, y `pactl subscribe` para enterarse) |
| `sys.call("audio.volume", 0.5)` · `("audio.step", -0.05)` · `("audio.mute")` | ponerlo, moverlo un paso, callarlo (o `("audio.mute", true)`) | |
| `battery` | `{ present, percent, charging }`; un sobremesa contesta `{ present = false }` | Linux: `/sys/class/power_supply` |
| `network` | `{ online, kind = "wired" \| "wifi" \| "none", name, strength }` | Linux: la ruta por defecto, `/proc/net/wireless` e `iw` |
| `media` | `{ playing, title, artist, album, player }`; sin reproductores, `player = ""` | Linux: MPRIS por D-Bus (`zbus`), sin preguntar a cada rato |
| `sys.call("media.toggle")` · `("media.next")` · `("media.previous")` | al reproductor que se está contando | |
| `apps` | `{ { name, exec, icon }, … }`, por orden alfabético | Linux: los `.desktop` de `XDG_DATA_DIRS` (sin los ocultos ni los de terminal) |
| `sys.call("apps.launch", exec)` | lanzar una, suelta del programa | Linux: `setsid -f sh -c` |

Lo que aún no es un servicio se puede sacar con `spawn` y `run`, pero eso ata el script a un sistema: es un apaño hasta que exista el servicio. `barra.luau` leía así el volumen; ya no llama a nada de Linux.

Al salir, el programa para todo lo que la lógica dejó corriendo; al recargar la lógica, también. Y si lo matan a la fuerza, se va con él igualmente (en Linux se lo pedimos al núcleo).

## La caja de arena

- **Sin `io` ni `os.execute`**: es el modo `sandbox` de Luau. La única puerta al sistema es `run`.
- **Tope de memoria**: 64 MB.
- **Los segundos contados**: un manejador que lleve más de 2 s sin acabar se corta, y la lógica sigue viva. Probado con un `while true do end`.
- **Un error no tumba nada**: se dice por consola y el resto de manejadores siguen.

> **`fact`, `text` y `sys` se leen siempre en el momento.** En su caja de arena, Luau da por hecho que un global no cambia y se guarda lo que leyó al cargar: `text.query` valía `""` para siempre dentro de un manejador. Se le dice al compilador que esos tres son mutables. Costó una tarde encontrarlo.

## Sucesos con carga

En la escena, `emit opened(i)`; en la lógica, `on("opened", function(i) … end)`. Antes hacía falta un suceso por índice.

## Listas que vienen de datos

Todavía no hay copias que nazcan en marcha (G12), pero con la lógica poniendo textos y un hecho `count`, una lista de capacidad fija se comporta como una de verdad. `bandeja.luau` tiene una tabla de avisos: cada 1,6 s llega uno arriba, pulsar uno lo despacha, y la columna de la escena recoloca a los demás con su muelle.

## Multiplataforma

Luau se compila de sus fuentes en C++, y `mlua` lo soporta en los tres sistemas. Aquí no hay compilador cruzado, así que `./portable.sh` comprueba Windows y macOS **sin** la característica `luau` y lo dice. El pegamento es Rust sin nada de sistema.
