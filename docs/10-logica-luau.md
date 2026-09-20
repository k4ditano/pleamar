# La lógica, en Luau

**Estado:** implementado (`src/logica_luau.rs`). Si al lado de `marea.plm` hay un `marea.luau`, esa es su lógica. Se recarga sola al guardarla, igual que la escena.

Ejemplos: `escenas/marea.luau` (avisos que llegan) y `escenas/bandeja.luau` (una lista de datos que crece y encoge).

## Qué puede hacer, y qué no

La lógica **no anima, no pinta y no sabe de coordenadas**. Vive en su propio hilo; el render no la espera nunca. Solo cruza la frontera:

```lua
fact.open = true                          -- decir qué es verdad. Se lee como lo que es: `true`, un número, o `"critical"`
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
local menu = sys.ask("tray.menu", key)        -- preguntarle algo y esperar la respuesta (la lógica puede esperar)
log("lo que sea", 42)
busy(600)                                 -- trabajo de mentira, para ver que al render le da igual
```

**Los hechos tienen tipo**, el que les dio la escena: un sí o no se lee y se escribe con `true` y `false`; un enumerado (`fact mode: low | normal | critical`), con el nombre de su valor; lo demás, números. `on("fact:mode", function(m) … end)` recibe lo mismo. Un valor que no existe es un error que dice cuáles hay: `'mode' cannot be 'critcal'. Did you mean 'critical'?`

Un nombre mal escrito es un error al momento, con sugerencia: `the scene has no fact called 'opne'. Did you mean 'open'?`

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
| `notifications` | `{ { id, app, title, body, icon, urgency, actions = { { key, label } } }, … }`, la más nueva primero. **Devuelve `false` si otro programa ya las recibe**: solo puede ser uno | Linux: pleamar es el servidor de `org.freedesktop.Notifications` |
| `sys.call("notifications.dismiss", id)` · `("notifications.invoke", id, "default")` · `("notifications.clear")` | descartar, pulsar uno de sus botones (la aplicación se entera), vaciar | |
| `tray` | `{ { key, id, title, status, icon, menu }, … }`; `icon` es un nombre o una ruta, tal cual para `image … = from` | Linux: `StatusNotifierItem`. Vigía si no hay otro; si lo hay, anfitrión del suyo |
| `sys.call("tray.activate", key)` · `("tray.secondary", key)` · `("tray.context", key)` · `("tray.scroll", key, 1)` | el clic, el del medio, que enseñe su menú (si sabe), la rueda | |
| `sys.ask("tray.menu", key)` → `{ { id, label, enabled, separator, checked, children }, … }` · `sys.call("tray.menu_click", key, id)` | el menú de un icono, como un árbol, y elegir algo de él | Linux: `com.canonical.dbusmenu` |
| `apps` | `{ { name, exec, icon }, … }`, por orden alfabético | Linux: los `.desktop` de `XDG_DATA_DIRS` (sin los ocultos ni los de terminal) |
| `sys.call("apps.launch", exec)` | lanzar una, suelta del programa | Linux: `setsid -f sh -c` |

Lo que aún no es un servicio se puede sacar con `spawn` y `run`, pero eso ata el script a un sistema: es un apaño hasta que exista el servicio. `barra.luau` leía así el volumen; ya no llama a nada de Linux.

Al salir, el programa para todo lo que la lógica dejó corriendo; al recargar la lógica, también. Y si lo matan a la fuerza, se va con él igualmente (en Linux se lo pedimos al núcleo).

## Plugins: una lógica por biblioteca

Una biblioteca con un `.luau` al lado es un plugin (ver [[pleamar · 11 Referencia del lenguaje 0.1]], §5). Su lógica es un script como cualquier otro, con tres diferencias:

- **Solo ve lo suyo.** `text.now` es el `Clock.now` de la escena; `fact.secret`, si `secret` es de la escena, no existe: `plugin 'Nosy' has no fact called 'secret'. A plugin only sees what its library declares`. Lo mismo con `model`, `emit` y lo que escucha: `on("tapped", …)` es `Clock.tapped`, y `on("key", …)` no oye nada.
- **Sus permisos los aprueba quien lo usa.** `pleamar --aprobar escena.plm` enseña lo que pide cada plugin y pregunta; sin aprobar, corre sin ninguno (`plugin 'Clock' wants to run 'date', but nobody has approved its permissions`), y si su código o lo que pide cambia, vuelve a estar sin aprobar.
- **Sus permisos son los de su `.plm`**, no los de la escena que lo usa: `plugin 'Nosy' has no permission to run 'sh'. If it should be able to, declare it in its own .plm (the scene's do not count)`.
- **No pide gestos ni mueve el cursor de escribir**: eso es de la escena. Si quiere que pase algo, emite un suceso suyo, y la escena decide (`on Clock.tapped { play nod }`).

Cada plugin tiene su propio estado de Luau —su memoria, sus temporizadores, sus procesos— **y su propio hilo**: uno que se atasque no frena a los demás ni a la lógica de la escena. La escena puede no tener lógica ninguna.

`require("lib/formato")` carga `lib/formato.luau` de la carpeta de esa lógica (la de la escena, o la del plugin), una vez; lo que devuelva es el módulo. Sin `..`, sin rutas enteras: no sale de su carpeta.

## Permisos

La caja de arena cierra `io` y `os`; lo que queda abierto al sistema son `run`, `spawn` y los servicios, y **cada escena declara cuáles usa**, en su `.plm`:

```
permissions { run: "date";  services: "workspaces", "workspaces.focus", "window", "audio", "audio.*" }
```

Sin declarar, nada. **Escuchar no es mandar**: `"audio"` deja usar `sys.watch` y `sys.ask`; para `sys.call("audio.volume", …)` hace falta `"audio.volume"`, o `"audio.*"` para todo lo de ese servicio. Una orden o un servicio que no esté ahí es un error al momento, que dice qué escribir: `the scene gives no permission to run 'sh'. If it should be able to, declare it in the .plm: permissions { run: "sh" }`. Está en la escena y no en el script para que se lea de un vistazo, antes de ejecutar nada; al arrancar se imprime (`lógica · permisos · órdenes: date · servicios: ninguno`), y al recargar la escena se aplican los nuevos. `apps.launch` solo lanza aplicaciones que el servicio `apps` haya contado.

## La caja de arena

- **Sin `io` ni `os.execute`**: es el modo `sandbox` de Luau. La única puerta al sistema es `run`.
- **Tope de memoria**: 64 MB.
- **Los segundos contados**: un manejador que lleve más de 2 s sin acabar se corta, y la lógica sigue viva. Probado con un `while true do end`.
- **Un error no tumba nada**: se dice por consola y el resto de manejadores siguen.

> **`fact`, `text` y `sys` se leen siempre en el momento.** En su caja de arena, Luau da por hecho que un global no cambia y se guarda lo que leyó al cargar: `text.query` valía `""` para siempre dentro de un manejador. Se le dice al compilador que esos tres son mutables. Costó una tarde encontrarlo.

## Sucesos con carga

En la escena, `emit opened(i)`; en la lógica, `on("opened", function(i) … end)`. Antes hacía falta un suceso por índice.

## Listas que vienen de datos

```lua
model.rows = { { label = "Abrir", enabled = true }, { label = "Salir" } }   -- la lista entera, de una vez
local r = model.rows[i + 1]                                                 -- y se lee de vuelta tal como se puso
```

La escena declara la forma (`model rows max 14 { label: text; enabled: bool = true }`) y la recorre (`for r in rows`). De cada ficha se cogen los campos declarados y se ignora lo demás, así que **la lista de un servicio se entrega tal cual**: `sys.watch("tray", function(list) model.icons = list end)`. Lo que falte vale su valor por defecto; un `bool` se escribe con `true` y `false`, un enumerado con el nombre de su valor (o su número), y una lista de dentro (`list items`) con otra tabla, que se reparte igual; y donde se espera un número, una lista cuenta como cuántos tiene (`children: number` con un submenú dentro). Solo viaja al render lo que haya cambiado respecto a la vez anterior. **La asignación es atómica**: si una ficha está mal, es un error y la lista que había se queda como estaba.

Al leerla de vuelta sale la tabla original, con todo lo que traía (`model.icons[1].key`), no solo los campos de la escena: es donde la lógica guarda lo que la escena no necesita ver.

## Multiplataforma

Luau se compila de sus fuentes en C++, y `mlua` lo soporta en los tres sistemas. Aquí no hay compilador cruzado, así que `./portable.sh` comprueba Windows y macOS **sin** la característica `luau` y lo dice. El pegamento es Rust sin nada de sistema.
