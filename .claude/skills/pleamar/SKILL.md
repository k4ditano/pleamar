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
pleamar --grammar        # every word the compiler accepts, by category
pleamar --check x.plm  # reads the scene, says whether it is fine, exits
```

`--grammar` prints the real vocabulary: statements, the properties of each
element, functions, triggers, effects, springs, units, services and what each
service reports. If a word is not in that list, it does not exist. The compiler
suggests the closest one ("did you mean…?"), so a failed `--check` usually
tells you the right word.

After writing or editing any scene, run `--check` on it. Always. It is
instant and it catches everything: unknown names, wrong types, a property that
belongs to another element, a service asked for something it does not report.

## Where the answers are

| | |
| --- | --- |
| `docs/11-language-reference.md` | The reference. Grammar, every element, every property, types, rules, layers, gestures, services, files. Read this before writing anything non-trivial |
| `docs/guide.md` | From zero to a bar, six steps |
| `docs/recipes.md` | Whole scenes that work: long lists, grids, melting shapes, gradients, paths, saved settings, normal windows, popups |
| `docs/10-luau-logic.md` | What the `.luau` logic can and cannot do |
| `examples/*.plm` | Real scenes that run: `bar.plm` (a bar), `long-list.plm` (5000 rows), `icons.plm` (tray with menus), `launcher.plm` (launcher), `paths.plm` (paths and gradients) |
| `tests/*.plm` | One file per language feature, each with the expected result on its first line |

## The shape of a scene

```
language 0.1              // optional, first line: which language version it needs
import "common/palette.plm" // libraries, relative to this file

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
- **A `let` is a name for an expression, not a variable**: a small one is
  written out wherever it is named. A big one is computed once a frame instead,
  so chaining them (`let b = a * (1 - t) + 60 * t`) is fine — but keep each link
  naming the one before it **once**, which is what `a * (1 - t) + k * t` does
  and `a + (k - a) * t` does not.
- **`prop` is a spring, not a variable.** Do not set it every frame from the
  logic; declare where it goes (`follow`, a rule, `impulse`) and let it travel.
  **The spring is the property's**: `prop lid = 0 ~150ms` is what `lid: 1` uses,
  and a rule only travels differently if it says so (`lid: 1 ~40ms`).
- **A property belongs to its element.** `corner` is a `box` thing, `radius` an
  `ellipse` thing, `width`/`lines` are `text` things. `--check` lists the
  valid ones when you miss.
- **Inside a layout (`row`/`column`) a child must say how much room it takes.**
  Wrap loose shapes in `group { size: w, h; … }`.
- **`if()` and `mix()` work with colours too**: `if(urgent, amber, mint)`,
  `mix(ink, mint, open)` —a comparison works as `t`—.
- **Text with holes** is `text "{a} · {b}"`, where `a` and `b` are live texts or
  facts. For a number with decimals, `text number(expr, 2, " %")`.
- **A surface does not grow with what it holds, and a shadow needs room.**
  `shadow: 0, 18, 44` reaches 62 px past its shape: leave it in `surface {
  size: … }` or the shadow is cut into a straight line. A panel that grows is
  the usual way to find this out: the renderer warns once, both for the shadow
  ("a shadow is cut: it needs 30 px below…") and for the drawing itself ("a
  drawing is cut: it needs 36 px below…"), once it has been cut for three
  seconds and never against an edge the surface is glued to. Only while it
  runs, though, never in `--check`: where a card ends is a sum that exists
  only while the scene is alive.
- **Zones are what catch the mouse.** A named shape only becomes a zone if a rule
  names it, if it carries `active`, or if it is declared with `zone`.
- **A press goes to the zone declared LAST**, not to the smallest one. So a
  grace zone —the big invisible rectangle that keeps a panel open while the
  pointer crosses a gap— goes **before** what it wraps, or it swallows every
  click inside it. This is the mistake that repeats: in marea-plm it happened
  five times, always the same way, and every time it looked like "the button
  does nothing". What is **hidden** does not catch it, though: a `show:` that is
  false, or an `opacity:` that has reached zero —on a group, on a layout—
  turns off the zones inside it. No need to repeat `active: open > 0.9` on
  every zone of a panel that fades in; keep `active:` for what is visible and
  still must not be pressed (a button mid-transition, a confirm that arms late).
- **Two rules in the same frame: the one declared LAST sets the value, and a
  `while` reads the frame BEFORE them.** A pointer that jumps from one row to
  another gives `leave` on the old one and `enter` on the new one in the same
  frame, so whatever clears has to be declared **before** whatever sets, or the
  row just entered is cleared by the row just left. Guarding it (`while thing ==
  what_it_set`) does not save it: the guard is reading the old value.
- **A loose `clip` reaches further than it looks**: it clips everything after it
  until the next named `surface`, not until the end of the block it seems to
  belong to. Something written further down comes out clipped to a shape that
  may be closed —that is, it does not come out at all—. Inside a `group`, it
  ends with the group.
- **A shadow is black unless you say otherwise** (`shadow: dx, dy, blur, alpha,
  colour`), and on a desktop of dark windows a black shadow has nothing to
  darken: it reads as a dirty ring around the thing it was meant to lift. What a
  small shape usually needs there is its own `rim`, which is light **inside** the
  silhouette and touches nothing behind it. Everything about a shadow is an
  expression, so it can show up only when there is something to cast one:
  `shadow: 0, 2 * open, 12 * open, 32% * open`.
- **An svg is `figure`, not `image`.** `image` rasterises it into an atlas —a
  sticker: it melts into nothing, it cannot be tinted by parts nor animated by
  layers—. `figure hat = file "hat.svg"` reads the same file as paths, and
  `figure hat.brim { … }` draws one layer (the `id` of its group in the file) in
  its place inside the piece, so two layers drawn apart still fit together.
- **Permissions are per service and listening is not commanding**:
  `services: "audio"` lets the logic know the volume; changing it needs
  `"audio.volume"` or `"audio.*"`.

## When the language falls short: your own shader

Before faking an effect with a hundred shapes, write it. `shader aurora = file
"aurora.wgsl"` declares one, `shader aurora { at: …; size: …; values: …; colors: … }`
paints a box with it. The file has ONE function, `fn shade(s: Shader) ->
vec4<f32>` —straight colour and coverage for each point—, and reads `s.pos`,
`s.size`, `s.uv`, `s.time`, `s.pointer`, `s.hovered`, `s.a`/`s.b` (the eight
`values:`), `s.color`/`s.color2`, and `behind(s, at)` / `behind_frosted(s, at)`
for what is behind the surface. Only what it reads costs: without `s.time` it
does not keep the scene painting. `--check` validates the WGSL with its line.
Contract and rules: §8.1 of the reference; working examples in
`examples/effects.plm` and `examples/shaders/`.

A `group` can also treat what it holds as one thing: `blur: 6`, `glow: 14, 90%,
mint` (or without a colour, a bloom), `saturation`, `brightness`, `contrast`,
`hue: 120deg`, `mask: x1, y1 to x2, y2` / `mask: radial x, y radius r1 to r2`,
`mode: add`. All animatable; §8.2. Two groups with effects cannot nest.

Particles are an element: `particles { at: x, y; count: 400; life: 0.8s .. 1.6s;
speed: 120 .. 220; direction: -90deg; spread: 40deg; gravity: 0, 260; size: 3, 1;
colors: a, b; shape: dot | square | spark; emit: cond }` (or `burst: event`).
Worked out on the card, thousands are cheap; §8.3.

A `text` can have `gradient:` (like a body's), `outline: w, color`, `shadow: dx,
dy, blur, alpha[, color]`, and per letter `letter_move: dx, dy`, `letter_opacity`,
`letter_scale`, where `letter` is its index and `letters` the count; §8.4.

For smaller things there is maths: `noise(x)`, `noise(x, y)`, `random(k)`,
`sqrt`, `pow`, `fract`, `mod`, `atan2`, `length`… and `time`, the seconds since
the scene started (naming it keeps the scene painting). Gesture frames take
`bezier(x1, y1, x2, y2)`, CSS's cubic-bezier.

## The logic, if there is any

`scene.luau` next to `scene.plm`. Sandboxed Luau, own thread, cut off if a
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
pleamar --check x.plm                    # no window: just says if it is fine
pleamar --scene x.plm --seconds 8          # closes itself after 8 seconds
pleamar --scene x.plm --screen HDMI-A-1   # on a chosen monitor
pleamar --scene x.plm --mouse "160,20@800 click@1400 up@1500"  # a pretend mouse
pleamar --say x "fact open true"           # talk to a running scene
pleamar --say x "emit arrives"             # …or fire one of its events
pleamar --scene x.plm --stall 2000        # stall the logic on purpose
pleamar --scene x.plm --record open,card # what those are worth on every frame
pleamar --scene x.plm --reduced-motion # springs settle, nothing loops
PLEAMAR_DEBUG_ZONES=1 pleamar --scene x.plm --mouse "…"  # which zones are under the pointer, as it moves
```

**Do not say an animation lasts what it was asked to last without measuring
it.** `--record name,other > log.tsv` prints `ms<tab>value…` once per frame,
and that is how a duration is checked against its contract. It says at the start
which names it could not find, with the ones that look like them: inside a
component's copy they carry their mark (`px#Hat2`). How to read those logs, in
`measuring.md`.

A press that does nothing is almost always a zone that is not the one on top,
or a rule that names another zone than the one under the pointer:
`PLEAMAR_DEBUG_ZONES=1` prints the zones under the pointer every time that
changes, with their real names (`drop.4#screen0` in a copy per monitor).

Never drive the real mouse or keyboard to test a scene: `--mouse` exists for
that, and `--say` reaches anything the logic can hear.

## Measuring it against Quickshell

If the job is about cost — CPU, memory, frames, how long it takes to appear —
read `measuring.md` next to this file. It has the commands, what each number
means, how to restart someone's bar without losing it, and the traps that make a
comparison worthless.

## Before saying it is done

```sh
./run-tests.sh      # every test scene, plus the examples inside the documentation
./portable.sh    # still builds for Windows and macOS
```

If the language changed, `run-tests.sh` also checks that the reference's vocabulary
still matches `--grammar` and that the highlighters in `editor/` are the
current ones. Regenerate them with `pleamar --highlight vim|vscode`.
