---
name: pleamar
description: Write or edit pleamar scenes (.plm files) and their Luau logic (.luau) — bars, launchers, panels, notifications, tray, popups, windows. Use when touching any .plm file, when building a desktop shell with pleamar, or when asked about the pleamar language, its services, springs, layers, gestures, components or permissions.
---

# Writing pleamar scenes

pleamar is a desktop runtime in Rust with its own declarative language. A `.plm`
file is a scene: what is drawn and how it reacts. Everything declared there is
run by the **renderer** — springs, rules, layers, gestures — so it keeps moving
even if the logic stalls. An optional `.luau` file next to it is the logic, and
it only reports facts; it never draws.

**It is not QML.** Do not carry over QtQuick names, properties or idioms. There
is no `Rectangle`, no `anchors.fill`, no `Behavior`, no JavaScript in the scene.

## The rule that matters

**Never guess a keyword or a property. Check it.**

```sh
pleamar --gramatica        # every word the compiler accepts, by category
pleamar --comprobar x.plm  # reads the scene, says whether it is fine, exits
```

`--gramatica` prints the real vocabulary: statements, the properties of each
element, functions, triggers, effects, springs, units, services and what each
service reports. If a word is not in that list, it does not exist. The compiler
suggests the closest one ("did you mean…?"), so a failed `--comprobar` usually
tells you the right word.

After writing or editing any scene, run `--comprobar` on it. Always. It is
instant and it catches everything: unknown names, wrong types, a property that
belongs to another element, a service asked for something it does not report.

## Where the answers are

| | |
| --- | --- |
| `docs/11-language-reference.md` | The reference. Grammar, every element, every property, types, rules, layers, gestures, services, files. Read this before writing anything non-trivial |
| `docs/guide.md` | From zero to a bar, six steps |
| `docs/recipes.md` | Whole scenes that work: long lists, grids, melting shapes, gradients, paths, saved settings, normal windows, popups |
| `docs/10-luau-logic.md` | What the `.luau` logic can and cannot do |
| `escenas/*.plm` | Real scenes that run: `barra.plm` (a bar), `lista-larga.plm` (5000 rows), `iconos.plm` (tray with menus), `lanzador.plm` (launcher), `caminos.plm` (paths and gradients) |
| `pruebas/*.plm` | One file per language feature, each with the expected result on its first line |

## The shape of a scene

```
language 0.1              // optional, first line: which language version it needs
import "comun/paleta.plm" // libraries, relative to this file

scene Name {
    surface { size: full, 44; anchor: top }   // the window it asks for
    permissions { services: "clock" }         // without this, the logic can do nothing

    service clock as now { time: text }       // system services, by name
    fact open = false                         // what the logic may report
    text title = "…"
    model rows max 14 { label: text }
    event chosen ->                           // `->` means the logic hears it too
    prop x = 360 ~calm                        // something that moves: a spring
    let mint = #9ed6bd                        // a name for an expression or colour

    box { from: 0, 0; size: 200, 44; color: mint }   // what is drawn
    text title { at: 10, 22; anchor: left center; size: 12; color: #f5f7f5 }

    zone box hit { from: 0, 0; size: 200, 44; cursor: pointer }
    on press hit { toggle open }              // rules, run by the renderer
    follow x = if(open, 420, 360)             // movement that carries itself
}
```

## Things that are easy to get wrong

- **Ranges are exclusive at the end.** `repeat i in 1..10` gives 1 to 9. A model
  with `max 16` has records 0 to 15, and its logic fills `1..16` in Lua terms.
- **`prop` is a spring, not a variable.** Do not set it every frame from the
  logic; declare where it goes (`follow`, a rule, `impulse`) and let it travel.
- **A property belongs to its element.** `corner` is a `box` thing, `radius` an
  `ellipse` thing, `width`/`lines` are `text` things. `--comprobar` lists the
  valid ones when you miss.
- **Inside a layout (`row`/`column`) a child must say how much room it takes.**
  Wrap loose shapes in `group { size: w, h; … }`.
- **`if()` returns numbers, not colours.** For colours use `mix(a, b, t)`, and a
  comparison works as `t`: `mix(ink, mint, open)`.
- **Text with holes** is `text "{a} · {b}"`, where `a` and `b` are live texts or
  facts. For a number with decimals, `text number(expr, 2, " %")`.
- **Zones are what catch the mouse.** A named shape only becomes a zone if a rule
  names it, if it carries `active`, or if it is declared with `zone`.
- **Permissions are per service and listening is not commanding**:
  `services: "audio"` lets the logic know the volume; changing it needs
  `"audio.volume"` or `"audio.*"`.

## The logic, if there is any

`escena.luau` next to `escena.plm`. Sandboxed Luau, own thread, cut off if a
handler runs longer than two seconds. It can only cross the boundary the scene
declares:

```lua
fact.open = true                 -- a declared fact
text.title = "hello"             -- a declared live text
model.rows = { { label = "a" } } -- the whole list at once, atomically
emit("chosen", 3)                -- a declared event
on("chosen", function(n) end)    -- listen: events, "fact:x", "text:x", "press:zone"
after(200, f)  every(1000, f)    -- timers
run("date", {"+%H:%M"}, f)       -- a command, if `permissions { run: … }` allows it
sys.watch("audio", f)            -- a service; sys.ask(…) asks, sys.call(…) commands
```

Everything else is a mistake: the logic does not draw, does not move properties
and does not know about coordinates.

## How to try a scene without bothering anyone

```sh
pleamar --comprobar x.plm                    # no window: just says if it is fine
pleamar --escena x.plm --segundos 8          # closes itself after 8 seconds
pleamar --escena x.plm --pantalla HDMI-A-1   # on a chosen monitor
pleamar --escena x.plm --raton "160,20@800 pulsa@1400 sube@1500"  # a pretend mouse
pleamar --decir x "fact open true"           # talk to a running scene
pleamar --escena x.plm --bloqueo 2000        # stall the logic on purpose
```

Never drive the real mouse or keyboard to test a scene: `--raton` exists for
that, and `--decir` reaches anything the logic can hear.

## Measuring it against Quickshell

If the job is about cost — CPU, memory, frames, how long it takes to appear —
read `measuring.md` next to this file. It has the commands, what each number
means, how to restart someone's bar without losing it, and the traps that make a
comparison worthless.

## Before saying it is done

```sh
./probar.sh      # every test scene, plus the examples inside the documentation
./portable.sh    # still builds for Windows and macOS
```

If the language changed, `probar.sh` also checks that the reference's vocabulary
still matches `--gramatica` and that the highlighters in `editor/` are the
current ones. Regenerate them with `pleamar --resaltado vim|vscode`.
