# pleamar

**An alternative to [Quickshell](https://quickshell.outfoxxed.me): write the
desktop — bars, launchers, notifications, tray, menus — in a declarative language
of its own, on top of a Rust runtime.**

The idea underneath: **animation should not depend on logic**. The scene declares
what moves and with which spring, and a render thread walks it at the screen's
pace, whatever happens in the thread that decides things. With the logic blocked
for 600 ms, 38 frames come out at 17 ms each; in the same test on QtQuick, a
600 ms hole.

## The quick comparison

The same bar, written twice: in Quickshell (`proyecto-marea`, 370 QML files) and
in pleamar (`marea-plm`, one `.plm` and one `.luau`). Both running at once, both
freshly started, same 60 Hz monitor, on an RTX 2060.

| | Quickshell | pleamar |
| --- | --- | --- |
| real memory (PSS) | 336 MB | **81 MB** |
| time to first frame | 1876 ms | **175 ms** |
| frames per second while animating | ~24 | **60** |
| CPU per painted frame | 0.257 % | **0.083 %** |
| CPU with nothing moving | never drops | **0.3 %** |
| opening a panel | 6.2 % → 12.3 %, and 9.5 % after closing it | 4.4 % → 4.9 %, and back to 4.5 % |

On raw CPU, with both animating, the difference is 20 %: **4.96 % against
6.17 %**. That is what it is, and it should be said before the pretty numbers.
What changes the picture is that pleamar is painting **two and a half times more
frames** while spending less, and that when nothing moves it **sleeps**.

The numbers, how they were taken and what to watch out for: `marea-plm/MEDIDAS.md`.

## What comes out of building it this way

- **The renderer animates on its own.** Springs, layers, gestures and rules are
  its job. The logic reports what happens and nothing else; if it stalls, the
  screen never finds out.
- **Everything is checked on load.** A misspelled name is an error with file,
  line, arrow and "did you mean…?", not an `undefined` at runtime.
- **Nothing declared can hang:** no free loops, no recursion.
- **Someone else's plugin runs with the permissions you approve**, not with
  yours. In Quickshell, a piece of foreign config is JavaScript with everything
  you can do.
- **Shapes that melt into each other**, with shadow, rim, light and gradients, no
  layers and no tricks: underneath it is all signed distance. An svg comes in
  the same way —as paths, by layers—, so a piece drawn in Inkscape melts into
  what carries it instead of sitting on top of it like a sticker.
- **Cross-platform by design:** everything system-specific behind
  `src/platform/`, and `./portable.sh` checks it still builds for Windows and
  macOS.

## A whole scene

```plm
language 0.1
scene Clock {
    surface { size: 240, 96; anchor: top; margin: 12 }
    permissions { services: "clock" }

    //  The time comes from the system: not a single line of logic here.
    service clock as now { time: text = "--:--"; date: text }

    prop hot = 0 ~quick

    body {
        color: #151616
        rim: 6%
        shadow: 0, 6, 18, 35%
        box { from: 0, 0; size: 240, 96; corner: 20 }
    }
    text now.time { at: 120, 44; anchor: center; size: 34; color: #f5f7f5 }
    text now.date { at: 120, 72; anchor: center; size: 12; color: #9ed6bd; opacity: 45% + hot * 55% }

    zone box whole { from: 0, 0; size: 240, 96; corner: 20 }
    on enter whole { hot: 1 }
    on leave whole { hot: 0 }
}
```

`pleamar --scene clock.plm`. Save the file and it reloads without losing
whatever was in motion; save it broken and the last good scene stays on screen,
with a band on top saying what does not compile and where. Rebuild pleamar itself and the
running one starts again with the same arguments.

## Getting started

```sh
cargo build --release
./target/release/pleamar --scene examples/bar.plm       # a bar: workspaces, window, time and volume
./target/release/pleamar --scene examples/long-list.plm # five thousand rows in sixteen copies
./target/release/pleamar --scene examples/paths.plm     # paths, curves and gradients
./target/release/pleamar --scene examples/window.plm     # a normal window, with its frame
./target/release/pleamar --scene examples/icons.plm      # the system tray; right-click opens an icon's menu
./target/release/pleamar --scene examples/settings.plm     # saves what you pick in its own folder
./target/release/pleamar --check examples/bar.plm    # reads it, says whether it is fine, exits
./run-tests.sh                                               # the language tests, and the examples in its reference
```

Options used daily: `--screen A,B` (which monitors), `--say` (talk to it from
outside, or from a compositor shortcut), `--mouse "360,90@500 click@3200"` (a
pretend mouse, to rehearse without touching the real one), `--stall MS` (stall
the logic on purpose and watch the screen carry on).

## The language in five minutes

A `.plm` file is a scene: what is seen, and how it reacts.

| | |
| --- | --- |
| `surface { size: full, 44; anchor: top }` | the window it asks for. Several per scene, and `kind: window` for a normal one |
| `fact open = false` · `text title = "…"` | what the logic may report. Typed too: `fact mode: low \| normal \| critical` |
| `prop x = 360 ~calm` | something that moves. Every property is a spring |
| `service audio { volume: number; muted: bool }` | a system service, by name, with no logic |
| `model rows max 14 { label: text }` · `for r in rows { … }` | a list the logic fills |
| `box`, `ellipse`, `arc`, `line`, `path`, `text`, `image`, `input` | what gets drawn |
| `figure hat = file "hat.svg"` | an svg **as geometry**: its layers are paths, so they melt, tint and turn |
| `body { color/gradient/rim/light/shadow }` | several shapes melted into one silhouette |
| `row` / `column` | layout, with gap, padding, alignment, `view:` to scroll and `wrap:` for a grid |
| `component Row(r: record) { … }` | something to copy, with typed parameters and named slots |
| `on press hit { open = true }` · `every 2s { … }` | rules, run by the renderer |
| `follow`, `blink`, `wave`, `spin`, `look` | movement that carries itself |
| `layer`, `gesture`, `posture` | who wins a slot, and timelines |
| `permissions { run: "date"; services: "audio" }` | undeclared, the logic cannot |

The full reference, with its grammar and its checked examples, is in
[`docs/11-language-reference.md`](docs/11-language-reference.md). To
start from zero, [`docs/guide.md`](docs/guide.md); to copy and paste,
[`docs/recipes.md`](docs/recipes.md).

## The logic

If there is a `bar.luau` next to `bar.plm`, that is its logic: Luau in a
sandbox, on its own thread, which the renderer never waits for. It can only cross
the boundary the scene declares — `fact.open = true`, `text.title = …`,
`model.rows = {…}`, `emit`, and listening with `on(…)` — plus timers, `sys` for
services and `run` for system commands, all behind permissions. Reference in
[`docs/10-luau-logic.md`](docs/10-luau-logic.md).

A library with its own `.luau` next to it is a **plugin**: its boundary lives
under its name (`Clock.now`), it runs on its own thread, and its permissions are
approved by whoever uses it, with `pleamar --approve`. Unapproved, it runs
touching nothing.

## In the editor

`pleamar --lsp` is a language server over stdio, with this same compiler behind
it: mistakes as you type, which words fit here, what the word under the cursor
means, go to where a name was declared, where it is used, and renaming it.
`pleamar --highlight vim|vscode` writes the syntax file straight from the
vocabulary, so it cannot fall behind. Both, already generated, in
[`editor/`](editor/).

## How it is built

Three threads that never wait for each other: **platform** (windows and input),
**logic** (Luau) and **render** (owner of the springs and the clock). Plus one
per plugin, one per service, and a workshop thread for text and images.

| | |
| --- | --- |
| `language/` | From text to scene: `tokens` tokenises, `tree` groups without knowing what anything means, `compiler` gives it meaning and checks the names, `vocabulary` is the list of what exists |
| `scene.rs` | The contract: properties, expressions, drawing instructions, rules, layers, gestures, zones, surfaces |
| `render.rs` | The interpreter. Still, it does not paint a single frame |
| `gpu.rs` · `shape.wgsl` | One quad per element; each pixel only runs the shapes of the element covering it |
| `shapes.rs` | The geometry, written once, used three times: the GPU, the mouse and the bounding boxes |
| `logic_luau.rs` | A scene's logic: Luau in a sandbox, with the boundary and nothing else |
| `text.rs` | Real text (`cosmic-text`) and images (SVG, PNG, JPEG) in an atlas at the monitor's scale |
| `platform/` | The only part that knows about the system: Wayland, the services, files, the clock |
| `lsp.rs` | The language server and the highlighters, both drawn from the vocabulary |

## What is missing

Measured against two real Quickshell configs (648 QML files between them), in
[`docs/07-whats-missing.md`](docs/07-whats-missing.md): session lock, showing a screen
inside the scene, input methods for Japanese or Chinese, and list copies that are
born and die on their own.

Known limitations, **each one with its plan to fix it**, in
[`docs/08-limitations.md`](docs/08-limitations.md). They are written down as
they show up, not at the end.

## Documentation

| | |
| --- | --- |
| [`docs/guide.md`](docs/guide.md) | From zero to a bar, step by step |
| [`docs/recipes.md`](docs/recipes.md) | Patterns that already work, ready to copy |
| [`docs/11-language-reference.md`](docs/11-language-reference.md) | The reference: grammar, types, every element and every property |
| [`docs/10-luau-logic.md`](docs/10-luau-logic.md) | What the logic can do, and what it cannot |
| [`docs/07-whats-missing.md`](docs/07-whats-missing.md) | Parity with Quickshell, told by real usage |
| [`docs/08-limitations.md`](docs/08-limitations.md) | Everything that fails or is missing, with its plan |
| [`docs/02-how-it-works.md`](docs/02-how-it-works.md) | Inside: the threads, the renderer, the why |
| [`.claude/skills/pleamar/`](.claude/skills/pleamar/) | So an AI writes `.plm` without making things up |

The working notes (01, 03–06, 09) are the design logbook: how this got here.

## Licence

**The code is here to be read, but this is not open source.** pleamar is under the
[PolyForm Noncommercial License 1.0.0](LICENSE): use it, study it, change it and
share it for anything **noncommercial** — your own desktop, a hobby project,
research, teaching, a charity — and that includes changing it and passing it on.

**Anything commercial needs permission.** Selling it, shipping it inside a
product, or running it as part of a business is not covered. Ask, and it can be
arranged.

Every dependency is permissive (MIT, Apache-2.0, BSD-3-Clause, Zlib), so none of
them forces anything on this code. If binaries ever get handed out, their
copyright notices have to travel with them: `THIRD-PARTY.md` has the list, and
`cargo about` regenerates the full texts.
