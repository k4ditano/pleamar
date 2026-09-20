# The logic, in Luau

**Status:** implemented (`src/logica_luau.rs`). If there is a `marea.luau` next to `marea.plm`, that is its logic. It reloads itself when saved, just like the scene.

Examples: `escenas/marea.luau` (notices coming in) and `escenas/bandeja.luau` (a list of data that grows and shrinks).

## What it can do, and what it cannot

The logic **does not animate, does not paint and knows nothing about coordinates**. It lives on its own thread; the renderer never waits for it. Only this crosses the boundary:

```lua
fact.open = true                          -- say what is true. It reads back as what it is: `true`, a number, or `"critical"`
text["notice.title"] = "Meeting in 5 min" -- say what a live text says
emit("confirmed")                         -- that something has happened
play("joy")                               -- ask for a gesture (the scene will grant it or not, by its class)

on("view_event", function(n) … end)       -- an event the scene lets out (`event x ->`), with its payload
on("press:view", …)  on("release:view", …)  on("enter:orb", …)  on("leave:orb", …)
on("scroll:sound", function(notches) … end)   on("key", function(name, text) … end)
on("text:query", function(value) … end)   -- someone typed in the `query` field; `text.query` already holds it
on("submit:query", function(value) … end) -- Enter in the field
on("focus", …)  on("blur", …)             -- the surface gains or loses the keyboard
on("drop:tray", function(data, mime) … end)  -- something was dropped from another application: text, or a list of `file://…`
focus("query")                            -- give the typing caret to a field; `focus()` takes it away
on("layer:card", function(claim) … end)   -- a layer changed hands
on("fact:open", function(v) … end)        -- a scene RULE changed a fact
on("demo", …)                             -- the tick of `--demo`

local t = every(1000, function() … end)   -- timers, in milliseconds
after(500, function() … end)
cancel(t)

run("date", { "+%H:%M" }, function(out, code) … end)   -- a system command; answers when it finishes
local id = spawn("pactl", { "subscribe" }, function(line) … end)   -- one that does NOT finish: one call per line
kill(id)

sys.watch("workspaces", function(w) … end)   -- a system service; returns whether this system has it
sys.call("workspaces.focus", 3)               -- ask a service for something
local menu = sys.ask("tray.menu", key)        -- ask it something and wait for the answer (the logic may wait)
log("whatever", 42)
busy(600)                                 -- fake work, to see that the renderer does not care
```

**Facts have a type**, the one the scene gave them: a yes-or-no is read and written with `true` and `false`; an enum (`fact mode: low | normal | critical`), with the name of its value; everything else, numbers. `on("fact:mode", function(m) … end)` receives the same. A value that does not exist is an error that says which ones there are: `'mode' cannot be 'critcal'. Did you mean 'critical'?`

A misspelled name is an error there and then, with a suggestion: `the scene has no fact called 'opne'. Did you mean 'open'?`

## The system services

`sys.watch(name, fn)` listens to something that happens in the system. The function receives the state right now and then every change, as a table. **The names are the same on every system**; who answers is `src/plataforma/`'s business. If this system does not have that service, `sys.watch` returns `false` and the scene decides what to do without it.

| Service | What it reports | Who provides it today |
| --- | --- | --- |
| `workspaces` | `{ active = 3, list = { { id, name, windows, monitor, active }, … } }`; each one's `active` says whether it is the active one **on its monitor**, which is what a bar per screen needs | Hyprland, over its sockets (without running `hyprctl`) |
| `window` | `{ title, class }` | Hyprland |
| `sys.call("workspaces.focus", n)` | go to a workspace | Hyprland |
| `audio` | `{ volume = 0.54, muted = false }` | Linux: PipeWire (`wpctl`, and `pactl subscribe` to find out) |
| `sys.call("audio.volume", 0.5)` · `("audio.step", -0.05)` · `("audio.mute")` | set it, move it one step, silence it (or `("audio.mute", true)`) | |
| `battery` | `{ present, percent, charging }`; a desktop answers `{ present = false }` | Linux: `/sys/class/power_supply` |
| `brightness` | `{ present, level }`; with no backlight, `{ present = false }` | Linux: `/sys/class/backlight` |
| `sys.call("brightness.level", 0.6)` | set it | Linux: `brightnessctl`, which is who has the permission |
| `network` | `{ online, kind = "wired" \| "wifi" \| "none", name, strength }` | Linux: the default route, `/proc/net/wireless` and `iw` |
| `media` | `{ playing, title, artist, album, player }`; with no players, `player = ""` | Linux: MPRIS over D-Bus (`zbus`), without asking every so often |
| `sys.call("media.toggle")` · `("media.next")` · `("media.previous")` | to the player being reported | |
| `notifications` | `{ { id, app, title, body, icon, urgency, actions = { { key, label } } }, … }`, the newest first. **Returns `false` if another program already receives them**: there can only be one | Linux: pleamar is the `org.freedesktop.Notifications` server |
| `sys.call("notifications.dismiss", id)` · `("notifications.invoke", id, "default")` · `("notifications.clear")` | dismiss, press one of its buttons (the application finds out), empty | |
| `tray` | `{ { key, id, title, status, icon, menu }, … }`; `icon` is a name or a path, ready as it comes for `image … = from` | Linux: `StatusNotifierItem`. Watcher if there is no other; if there is, host to it |
| `sys.call("tray.activate", key)` · `("tray.secondary", key)` · `("tray.context", key)` · `("tray.scroll", key, 1)` | the click, the middle one, asking it to show its menu (if it knows how), the wheel | |
| `sys.ask("tray.menu", key)` → `{ { id, label, enabled, separator, checked, children }, … }` · `sys.call("tray.menu_click", key, id)` | an icon's menu, as a tree, and choosing something from it | Linux: `com.canonical.dbusmenu` |
| `apps` | `{ { name, exec, icon }, … }`, in alphabetical order | Linux: the `.desktop` files in `XDG_DATA_DIRS` (without the hidden ones or the terminal ones) |
| `sys.call("apps.launch", exec)` | launch one, loose from the program | Linux: `setsid -f sh -c` |

Whatever is not a service yet can be got out with `spawn` and `run`, but that ties the script to one system: it is a stopgap until the service exists. `barra.luau` used to read the volume that way; it no longer calls anything of Linux's.

On exit, the program stops everything the logic left running; on reloading the logic, too. And if it is killed outright, it all goes with it just the same (on Linux the kernel is asked to see to it).

## Plugins: one logic per library

A library with a `.luau` next to it is a plugin (see [[pleamar · 11 Referencia del lenguaje 0.1]], §5). Its logic is a script like any other, with four differences:

- **It only sees its own.** `text.now` is the scene's `Clock.now`; `fact.secret`, if `secret` belongs to the scene, does not exist: `plugin 'Nosy' has no fact called 'secret'. A plugin only sees what its library declares`. The same with `model`, `emit` and what it listens to: `on("tapped", …)` is `Clock.tapped`, and `on("key", …)` hears nothing.
- **Its permissions are approved by whoever uses it.** `pleamar --aprobar scene.plm` shows what each plugin asks for and asks about it; unapproved, it runs with none (`plugin 'Clock' wants to run 'date', but nobody has approved its permissions`), and if its code or what it asks for changes, it goes back to unapproved.
- **Its permissions are the ones in its own `.plm`**, not those of the scene using it: `plugin 'Nosy' has no permission to run 'sh'. If it should be able to, declare it in its own .plm (the scene's do not count)`.
- **It does not ask for gestures or move the typing caret**: that belongs to the scene. If it wants something to happen, it emits an event of its own, and the scene decides (`on Clock.tapped { play nod }`).

Each plugin has its own Luau state —its memory, its timers, its processes— **and its own thread**: one that gets stuck does not hold back the others or the scene's logic. The scene may have no logic at all.

`require("lib/formato")` loads `lib/formato.luau` from that logic's folder (the scene's, or the plugin's), once; whatever it returns is the module. No `..`, no whole paths: it does not leave its folder.

## Permissions

The sandbox closes off `io` and `os`; what stays open to the system is `run`, `spawn` and the services, and **each scene declares which ones it uses**, in its `.plm`:

```
permissions { run: "date";  services: "workspaces", "workspaces.focus", "window", "audio", "audio.*" }
```

Undeclared is nothing. **Listening is not commanding**: `"audio"` allows `sys.watch` and `sys.ask`; for `sys.call("audio.volume", …)` it takes `"audio.volume"`, or `"audio.*"` for everything in that service. A command or a service that is not there is an error there and then, which says what to write: `the scene gives no permission to run 'sh'. If it should be able to, declare it in the .plm: permissions { run: "sh" }`. It is in the scene and not in the script so that it can be read at a glance, before anything runs; it is printed at startup (`logic · permissions · commands: date · services: none`), and reloading the scene applies the new ones. `apps.launch` only launches applications the `apps` service has reported.

## The sandbox

- **No `io`, no `os.execute`**: it is Luau's `sandbox` mode. The only door to the system is `run`.
- **Memory ceiling**: 64 MB.
- **Counted seconds**: a handler that has gone more than 2 s without finishing is cut off, and the logic stays alive. Tested with a `while true do end`.
- **An error takes nothing down**: it is reported on the console and the rest of the handlers carry on.

> **`fact`, `text` and `sys` are always read there and then.** In its sandbox, Luau takes for granted that a global does not change and keeps what it read on load: `text.query` was worth `""` forever inside a handler. The compiler is told those three are mutable. It took an afternoon to find.

## Events with a payload

In the scene, `emit opened(i)`; in the logic, `on("opened", function(i) … end)`. Before, it took one event per index.

## Lists that come from data

```lua
model.rows = { { label = "Open", enabled = true }, { label = "Quit" } }     -- the whole list, at once
local r = model.rows[i + 1]                                                 -- and it reads back just as it was put in
```

The scene declares the shape (`model rows max 14 { label: text; enabled: bool = true }`) and walks it (`for r in rows`). From each record the declared fields are taken and the rest is ignored, so **a service's list is handed over as it comes**: `sys.watch("tray", function(list) model.icons = list end)`. Whatever is missing is worth its default value; a `bool` is written with `true` and `false`, an enum with the name of its value (or its number), and an inner list (`list items`) with another table, which is handed out the same way; and where a number is expected, a list counts as how many it holds (`children: number` with a submenu inside). Only what has changed since the previous time travels to the renderer. **The assignment is atomic**: if one record is wrong, it is an error and the list that was there stays as it was.

Reading it back gives the original table, with everything it carried (`model.icons[1].key`), not just the scene's fields: it is where the logic keeps what the scene does not need to see.

## Cross-platform

Luau is compiled from its C++ sources, and `mlua` supports it on all three systems. There is no cross-compiler here, so `./portable.sh` checks Windows and macOS **without** the `luau` feature and says so. The glue is Rust with nothing of the system in it.
