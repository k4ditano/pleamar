# The logic, in Luau

**Status:** implemented (`src/logic_luau.rs`). If there is a `marea.luau` next to `marea.plm`, that is its logic. It reloads itself when saved, just like the scene.

Examples: `examples/marea.luau` (notices coming in) and `examples/tray.luau` (a list of data that grows and shrinks).

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
json.decode(text)  json.encode(t)  -- what a program prints (`run`'s output) as tables, and back
focus("query")                            -- give the typing caret to a field; `focus()` takes it away
on("layer:card", function(claim) … end)   -- a layer changed hands
on("fact:open", function(v) … end)        -- a scene RULE changed a fact
on("demo", …)                             -- the tick of `--demo`

local t = every(1000, function() … end)   -- timers, in milliseconds
after(500, function() … end)
cancel(t)

run("date", { "+%H:%M" }, function(out, code) … end)   -- a system command; answers when it finishes
run("wl-copy", { "--type", "image/png" }, nil, { stdin = path, output = false })   -- how: a file on its input; and not waiting for what it writes
run("sudo", { "-S", "-p", "", "-v" }, function(_, code) … end, { input = pw .. "\n" })   -- a text on its input, which never touches a file, its arguments or its environment
local id = spawn("wf-recorder", { "-f", file }, function(line) … end, function(_, code) … end)   -- one that does not end: a call per line, and one when it HAS ended
spawn("pacman", args, on_line, on_exit, { input = "…", errors = true, env = { LC_ALL = "C" } })   -- how, too: a text on its input, its error output among the lines, and variables of its own (`env` and `errors` also for `run`: its error output after the rest)
local id = spawn("node", { "worker.mjs" }, on_line, on_exit, { stdin = "open" })   -- its input left open:
write(id, '{"type":"prompt"}\n')                       -- a line to it while it runs (true if it got there); write(id) closes it
kill(id, "int")                                       -- asks it to stop (or "term"); kill(id) alone kills it and forgets it
local id = spawn("pactl", { "subscribe" }, function(line) … end)   -- one that does NOT finish: one call per line
kill(id)

sys.watch("workspaces", function(w) … end)   -- a system service; returns whether this system has it
sys.call("workspaces.focus", 3)               -- ask a service for something
local menu = sys.ask("tray.menu", key)        -- ask it something and wait for the answer (the logic may wait)
log("whatever", 42)
tr("Good morning")                   -- the scene's `translations`, in the language `locale` says
busy(600)                                 -- fake work, to see that the renderer does not care
```

**Facts have a type**, the one the scene gave them: a yes-or-no is read and written with `true` and `false`; an enum (`fact mode: low | normal | critical`), with the name of its value; everything else, numbers. `on("fact:mode", function(m) … end)` receives the same. A value that does not exist is an error that says which ones there are: `'mode' cannot be 'critcal'. Did you mean 'critical'?`

A misspelled name is an error there and then, with a suggestion: `the scene has no fact called 'opne'. Did you mean 'open'?`

## The system services

`sys.watch(name, fn)` listens to something that happens in the system. The function receives the state right now and then every change, as a table. **The names are the same on every system**; who answers is `src/platform/`'s business. If this system does not have that service, `sys.watch` returns `false` and the scene decides what to do without it.

| Service | What it reports | Who provides it today |
| --- | --- | --- |
| `workspaces` | `{ active = 3, list = { { id, name, windows, monitor, active }, … } }`; each one's `active` says whether it is the active one **on its monitor**, which is what a bar per screen needs | Hyprland, over its sockets (without running `hyprctl`) |
| `window` | `{ title, class, monitor }` of the one with the focus; elsewhere than Hyprland also `list = { { id, title, class, monitor, active, minimized }, … }`, every window in the order it came | Hyprland; any compositor with wlr-foreign-toplevel (pleamar-wm, sway, niri…) |
| `sys.call("workspaces.focus", n)` | go to a workspace | Hyprland |
| `sys.call("window.restore", id)` · `("window.minimize", id)` · `("window.activate", id)` · `("window.close", id)` | a window, by the `id` `window`'s `list` gives it: brought back (with the keyboard), put away, given the keyboard, asked to close | wlr-foreign-toplevel (pleamar-wm, sway, niri…) |
| `audio` | `{ volume = 0.54, muted = false, input, input_muted, outputs, inputs, apps }`: the default output and input (0..1, as the mixer shows it), the devices for each, `{ { id, name, default }, … }`, and what is playing, `apps = { { id, name, icon, binary, title, volume, muted, playing }, … }`: one per stream (a browser has one for each tab that sounds), `icon` its icon name (or its program's, in lower case, if it does not say), `title` what it plays, `playing` false while it is paused. Event sounds (a click, a notice's chime) are left out | Linux: PulseAudio's protocol spoken directly (PipeWire speaks it too), told of changes; nothing is spawned. Without a sound server to speak to, `wpctl` |
| `sys.call("audio.volume", 0.5)` · `("audio.step", -0.05)` · `("audio.mute")` · `("audio.input", 0.8)` · `("audio.input_mute")` · `("audio.default", id)` · `("audio.app_volume", id, 0.5)` · `("audio.app_mute", id[, true])` | set it, move it one step, silence it (or `("audio.mute", true)`), the same for the input, where it plays or listens (an `id` from the lists), and one application's volume or silence (an `id` from `apps`) | `apps`, `app_volume` and `app_mute` need a sound server (PulseAudio or PipeWire); with `wpctl` alone there are no `apps` |
| `battery` | `{ present, percent, charging }`; a desktop answers `{ present = false }` | Linux: `/sys/class/power_supply` |
| `brightness` | `{ present, level }`; with no backlight, `{ present = false }` | Linux: `/sys/class/backlight` |
| `sys.call("brightness.level", 0.6)` | set it | Linux: `brightnessctl`, which is who has the permission |
| `network` | `{ online, kind = "wired" \| "wifi" \| "none", name, strength, wifi, networks = { { ssid, strength, secure, known, active }, … } }`: the connection in use, whether the wifi radio is on, and the networks in range (one per name, strongest first; `known`: used before) | Linux: NetworkManager over D-Bus, told of changes; without it, the kernel (`/proc/net`, only `online`, `kind`, `name`, `strength`) |
| `sys.call("network.wifi", true)` · `("network.scan")` · `("network.connect", ssid[, password])` · `("network.disconnect")` · `("network.forget", ssid)` | the radio on or off, look for networks, join one (a known one as it was saved; a new one with its password), leave, forget a saved one | `services: "network.*"`; NetworkManager |
| `bluetooth` | `{ present, powered, discovering, devices = { { name, address, paired, connected, battery, icon }, … } }`: the first adapter and the devices it knows or sees, connected first; `battery` 0..1, or -1 if the device does not say. With no adapter, `{ present = false }` | Linux: BlueZ over D-Bus, told of changes |
| `sys.call("bluetooth.power", true)` · `("bluetooth.scan", true)` · `("bluetooth.connect", address)` · `("bluetooth.disconnect", address)` · `("bluetooth.forget", address)` | the radio, looking for devices, connecting (it pairs first if it has to, and trusts it; it takes seconds and the change arrives through the service), leaving, forgetting | `services: "bluetooth.*"`; BlueZ |
| `thumbnails` | `{ capturing, list = { { id, title, app, picture, stale }, … } }`: every window of the desktop, the scene's or not. `picture` is a small PNG of it (400 px at most), a path with a version that `image … = from` paints and reloads: empty until it is asked for. A window asked for live (`thumbnails.live`) skips the file: `picture` names its last frame, kept in memory (`thumbnails:3?12`), which `image … = from` paints at the size it is drawn (a very large one a little softer, as any live image); once it is no longer asked for live, `picture` is the PNG of that last frame. `stale`, the compositor said it stopped copying it and the picture is the last one; a window it does not copy without saying so (Hyprland, for one off every monitor) keeps its last picture all the same, not stale. A copy with the same pixels as the last one changes nothing. `capturing` false, the compositor lists its windows but cannot copy them | Linux: `ext-foreign-toplevel-list` and `ext-image-copy-capture` (pleamar-wm, Hyprland, sway, niri…); a window is copied only when it changes, a few times a second at most |
| `sys.call("thumbnails.want", ids)` · `("thumbnails.want", "all")` · `("thumbnails.want")` | which windows to copy: the ones on screen, all of them, or none (the default). Copying costs: ask for the ones shown | |
| `sys.call("thumbnails.live", ids)` · `("thumbnails.live", "all")` · `("thumbnails.live")` | which windows to copy live, for an overview that shows them moving: up to 30 times a second, and only when they change, straight to the image that draws them, at its size. The same arguments as `thumbnails.want`, and the two do not mix: a window in either is copied, live if it is in this one. A window moved between the two keeps its picture until it next changes, except that one no longer live becomes a file straight away. Live costs more: ask for the ones shown | |
| `media` | `{ playing, title, artist, album, length, position, rate, art, player }`; `length` and `position` in seconds (0 if the player does not say), `position` as of the report —carry it on at `rate` while `playing`, see the recipe *How far into the song*—, `art` the `mpris:artUrl` as given; with no players, `player = ""`; `players = { { id, name, playing, chosen }, … }`, all of them, `name` being the player's `Identity` | Linux: MPRIS over D-Bus (`zbus`), without asking every so often |
| `sys.call("media.toggle")` · `("media.next")` · `("media.previous")` · `("media.choose", id)` | to the player being reported; `media.choose` pins which one that is (an `id` from `players`), `""` lets it choose again | |
| `notifications` | `{ { id, app, title, body, icon, image, urgency, time, actions = { { key, label } } }, … }`, the newest first. `icon` is the application's; `image` the picture it brings —the sender's photo—, a file even when it arrives as pixels (`image-data`); `time` when it arrived, in seconds since 1970. There can only be one receiver: the newest shell takes them from an older one (which gets them back when it leaves); behind a daemon that will not give them up, it waits in the queue. **Returns `false` only without a session bus** | Linux: pleamar is the `org.freedesktop.Notifications` server |
| `sys.call("notifications.dismiss", id)` · `("notifications.invoke", id, "default")` · `("notifications.clear")` | dismiss, press one of its buttons (the application finds out), empty | |
| `sys.ask("auth.check", password)` | is that the password of whoever owns the session? `true` or `false`. It is what a lock screen needs to open. A wrong one takes a couple of seconds, on purpose, and the system counts it like any other failed login | `services: "auth"` |
| `sys.call("notifications.keep", true)` | they stop expiring on their own. Six seconds and gone is what a bar of bubbles wants; a notification **centre** —a tray, a history— keeps them until somebody decides, and the standard leaves that to the server. It is not only how long they show: once one is closed its application stops listening, so an expired one can no longer be opened from here | |
| `notification_history` | the same shape: the ones that went away **on their own**, unseen, newest first, the last 50 (not the ones someone closed or opened: those were seen). It lives while the process does. With `notifications.keep(true)` nothing expires, so it stays empty: then the list itself is the centre | Linux |
| `sys.call("notifications.forget", id)` · `("notifications.clear_history")` | one out of the history, or all of it | |
| `tray` | `{ { key, id, title, status, icon, menu }, … }`; `icon` is a name or a path, ready as it comes for `image … = from` | Linux: `StatusNotifierItem`. Watcher if there is no other; if there is, host to it |
| `sys.call("tray.activate", key)` · `("tray.secondary", key)` · `("tray.context", key)` · `("tray.scroll", key, 1)` | the click, the middle one, asking it to show its menu (if it knows how), the wheel | |
| `sys.ask("tray.menu", key)` → `{ { id, label, enabled, separator, checked, children }, … }` · `sys.call("tray.menu_click", key, id)` | an icon's menu, as a tree, and choosing something from it | Linux: `com.canonical.dbusmenu` |
| `apps` | `{ { name, exec, icon, id, wmclass, local_name, categories }, … }`, in alphabetical order; `id` is the `.desktop` file's name (`org.gnome.Calculator`, which is the app id its windows say) and `wmclass` its `StartupWMClass`, to tell whose a window is; `local_name` its name in the user's language (`Name[es]`: «Calculadora»; the name itself if it has none), `categories` its `Categories=` as written (`Utility;Calculator;`) | Linux: the `.desktop` files in `XDG_DATA_DIRS` (without the hidden ones or the terminal ones) |
| `sys.call("apps.launch", exec)` | launch one, loose from the program | Linux: `setsid -f sh -c` |

Whatever is not a service yet can be got out with `spawn` and `run`, but that ties the script to one system: it is a stopgap until the service exists. `bar.luau` used to read the volume that way; it no longer calls anything of Linux's.

On exit, the program stops everything the logic left running; on reloading the logic, too. And if it is killed outright, it all goes with it just the same (on Linux the kernel is asked to see to it).

## Plugins: one logic per library

A library with a `.luau` next to it is a plugin (see [Language reference — version 0.1](11-language-reference.md), §5). Its logic is a script like any other, with four differences:

- **It only sees its own.** `text.now` is the scene's `Clock.now`; `fact.secret`, if `secret` belongs to the scene, does not exist: `plugin 'Nosy' has no fact called 'secret'. A plugin only sees what its library declares`. The same with `model`, `emit` and what it listens to: `on("tapped", …)` is `Clock.tapped`, and `on("key", …)` hears nothing.
- **Its permissions are approved by whoever uses it.** `pleamar --approve scene.plm` shows what each plugin asks for and asks about it; unapproved, it runs with none (`plugin 'Clock' wants to run 'date', but nobody has approved its permissions`), and if its code or what it asks for changes, it goes back to unapproved.
- **Its permissions are the ones in its own `.plm`**, not those of the scene using it: `plugin 'Nosy' has no permission to run 'sh'. If it should be able to, declare it in its own .plm (the scene's do not count)`.
- **It does not ask for gestures or move the typing caret**: that belongs to the scene. If it wants something to happen, it emits an event of its own, and the scene decides (`on Clock.tapped { play nod }`).

Each plugin has its own Luau state —its memory, its timers, its processes— **and its own thread**: one that gets stuck does not hold back the others or the scene's logic. The scene may have no logic at all.

`require("lib/format")` loads `lib/format.luau` from that logic's folder (the scene's, or the plugin's), once; whatever it returns is the module. No `..`, no whole paths: it does not leave its folder. Saving a module reloads the logic, like saving the main file. A module shares the logic's globals (`on`, `fact`, `sys`…), not its `local`s: what it needs from the main file goes to it as an argument, `require("pages/wifi")(shared)`.

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
