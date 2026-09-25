# pleamar — Index

**What it is:** a new desktop runtime —what would be "a better Quickshell"— and the language its scenes are written in. Born on 19 Sep 2026 out of a conversation about why QtQuick cannot animate while the logic works.

**Where it lives:** `~/Proyectos/pleamar` · [github.com/k4ditano/pleamar](https://github.com/k4ditano/pleamar) (private). These notes are mirrored in the repo's `docs/`; **if they differ, the Edinot copy wins**.

## The notes

| Note | What for |
| --- | --- |
| [[pleamar · 01 Por qué existe]] | The problem, the idea, and what was measured |
| [[pleamar · 02 Cómo funciona hoy]] | The runtime that already runs: threads, scene, shader |
| [[pleamar · 03 El lenguaje - borrador 0]] | The Spanish sketch from before it was implemented. Historical; note 09 wins |
| [[pleamar · 04 Prueba - cabe Marea]] | Marea's `ExpressionController` against the language |
| [[pleamar · 05 Decisiones y preguntas abiertas]] | What was decided, and why, and what is still to be decided |
| [[pleamar · 06 Bocetos A-B-C]] | The three syntaxes that were compared. Historical. |
| [[pleamar · 07 Qué falta para igualar a Quickshell]] | The honest inventory, by stages, and the order of work |
| [[pleamar · 09 El lenguaje v0]] | **The language guide**: reads straight through, with the reason for every choice |
| [[pleamar · 10 La lógica en Luau]] | The boundary seen from the logic: what a `.luau` can do, and its sandbox |
| [[pleamar · 11 Referencia del lenguaje 0.1]] | **The reference**: lexicon, EBNF grammar, every element with what it accepts, and the version number. Its examples are compiled by `./run-tests.sh` |
| [[pleamar · 08 Limitaciones conocidas]] | Everything that is half-done, with its severity, to cross off one by one |

## Status (19 Sep 2026)

- ✅ **The renderer animates on its own.** With the logic blocked for 600 ms, 38 frames at ~17 ms. The same trial in QML: a 600 ms gap.
- ✅ **A scene is data.** Properties, expressions, draw list, behaviors and zones. Two scenes (Marea, island) on the same renderer.
- ✅ **The language's direction was chosen:** tree (A) plus the declarative part of (B), Luau for the logic only.
- ✅ **It was tested against the real Marea.** It fits, with three pieces the sketch did not have: facts and events, layers, gestures.
- ✅ **Layers, gestures, facts, events and rules in the runtime** (still written in Rust). Tested with Marea's face: the red disc holds through searching and confirming while it records; when recording stops, the magnifier comes back on its own. And Marea opens, highlights its button and closes with the logic blocked for 5 s.
- ✅ **Platform boundary** (`src/platform/`): everything Wayland lives behind it, and `./portable.sh` checks that the core compiles for Linux, Windows and macOS. It passes.
- ✅ **Real text and images**: `cosmic-text` (ligatures, Arabic, Japanese, emoji, line breaking, ellipsis, alignment), glyphs at the monitor's scale, **live texts** the logic changes, SVG/PNG/JPEG, icons by name and tinting. Marea and the island no longer use bitmaps.
- ✅ **Measuring text from an expression** and a **text workshop** on its own thread: not one slow frame after the first.
- ✅ **The language, v0**: a `.plm` file → a scene. Tokenizer, parser, name checking with "did you mean…?", errors with line and arrow, and **hot reload** that loses nothing. `examples/marea.plm` and `examples/cara.plm` run without a line of Rust.
- ✅ **Components, `repeat` and layout** (`row`/`column`): each copy with its own names, zones and rules; each child goes to its slot with a spring. `examples/bandeja.plm` is a list of notices that grows and shrinks. And the file is read in four passes: the order is the reader's.
- ✅ **The logic in Luau**: a `.luau` beside the scene, in a sandbox (no `io`, 64 MB, cut off at 2 s), with facts, texts, events with a payload, timers and system commands. It hot-reloads. `bandeja.luau` drives a list from data.
- ✅ **The first real bar** (`examples/barra.plm` + `barra.luau`): Hyprland's workspaces, the active window, the clock and the volume, across the full width of the monitor. With it came the **system services** behind `plataforma/` (`sys.watch`, `sys.call`), `spawn`/`kill`, `size: full`, `screen.width`, `anchor:` in layouts and `text number(…)`.
- ✅ **The input that was missing**: wheel, drag (with local coordinates, and it keeps tracking outside the zone), release, press and hold, right and middle button, keys, and cursor shape. The bar already changes the volume: wheel, drag and mute.
- ✅ **Typing, dropping and sending from outside**: `input` (a field the renderer edits: caret, selection, clipboard), `on submit`, `on focus` / `on blur`, `on key Ctrl+k`, `on drop` for what other applications drop, the keyboard held only while it is needed, and `pleamar --say lanzador "emit toggle"` so a compositor shortcut can open a scene. With it, **a launcher** (`examples/lanzador.plm` + `.luau`) on top of the `apps` service.
- ✅ **Per-element renderer**: one quad per element with its box. 600 shapes cost 0.42 ms per frame against 3.66 ms for the per-pixel interpreter; 2000, 0.57 ms. With it came **ring/stroke, arc, segment, rotation** (its own and inherited), **linear gradient, border**, nested clips (up to four) and real blending between elements. And after that, closing its limitations: **affine transforms that compose** (rotation, scale, translation), **group opacity** with an intermediate layer, an exact box for rotated textures and mouse zones under transforms. All on show in `--scene muestrario`.
- ✅ **Real surfaces**: one per monitor —and for the ones plugged in later—, fractional scale (tested at 2 and at 1.5 on a virtual monitor), and an input region that follows the zones: what is transparent lets the click through. The scene declares its surface (size, anchor, level, margin, reserved space).
- ✅ **Real system services**: `audio`, `battery`, `network` and `media` (MPRIS), with the same names and tables on every system; the bar no longer calls `pactl`. Whatever is spawned dies with the program, even if the program is killed. And four fewer limitations: a layout's measurements readable before it is declared, **images the logic picks** (`image … = from some_text`: the launcher has icons), `--say … "get name"` that answers back, and the system's key repeat.
- ✅ **Permissions for the logic** (`permissions { run: …; services: … }`: undeclared is nothing) and **a renderer with no ceilings**: the card's buffers grow on their own (6000 shapes at 17 ms).
- ✅ **Notifications and tray**: `notifications` (pleamar is the server; `examples/bandeja` shows the real ones) and `tray` (with the Telegram and ChatGPT icons drawn from their pixels). That closes B1: audio, battery, network, music, applications, workspaces, window, notifications and tray.
- ✅ **Popup surfaces** (`popup`: an `xdg_popup` showing another piece of the same scene) and **the tray menus** (`sys.ask("tray.menu", …)`, with submenus). `examples/iconos` is the whole tray: the Telegram and ChatGPT icons, and their real menus.
- ✅ **Models and `for`**: shaped data crossing the boundary (`model rows max 14 { label: text; enabled: bool = true }`, `for r in rows { Row(r) }`, `model.rows = list`). The three scenes with lists, rewritten without a slot by hand. And **the language's first test suite** (`tests/`, `./run-tests.sh`), which watches the error messages too.
- ✅ **Texts with slots**: `text "{n.title}{? · {n.body}}"`, with expressions (`{volume * 100} %`), `upper`/`lower`, stretches that disappear if their text is empty, and errors that point at the exact character inside the string.
- ✅ **Several files**: `library` and `import`, with errors that say which file, and hot reload of what was imported. `examples/common/` holds the palette and the menu row. The language test suite is up to 22.
- ✅ **The language, written down**: note 11 is its reference, version **0.1** (`language 0.1`, `pleamar --version`), taken from the compiler, with its examples compiled on every `./run-tests.sh` and its vocabulary compared against the one the compiler consults (`pleamar --grammar`): 68 checks. Writing it uncovered two gaps, now closed: there was no `==`, and `while` only worked in two rules.
- ✅ **Components that say what they need**: parameters with a type and a default, arguments by name, and events as a parameter (`MenuRow(r, chosen: choose)`). An error inside a component says where it was used from. With **slots for children** (`children`, named when several are needed), `between` and `list.count` in layouts, and **`strict` libraries**, which only read what they ask for.
- ✅ **Types**: `bool` facts and enums (`fact mode: low | normal | critical`), with their names in expressions, in slots and in the logic; and `image` and `list` fields (records inside records).
- ✅ **Plugins**: a library with its `.luau` beside it, with its boundary under its own name (`Clock.now`), its own Luau state —which sees only what is its own— and its own permissions. Tested with a hostile plugin. `examples/plugins/reloj` is the first one.
- ✅ **Plugins worth trusting**: their permissions are approved by whoever uses them (`pleamar --approve`; unapproved, they run with none), each one on its own thread, with `require`, and they can bring their own gestures, layers and images. And the compiler checks the enums.
- ✅ **Several windows in one process** (`surface panel { … }`), **lists that scroll with the wheel** (`view:`), **services that do not depend on Hyprland** (`ext-workspace`, `wlr-foreign-toplevel`) and a core with nothing of any particular shell in it.
- ✅ **The messages speak English**: the 210 a pleamar user sees, with their "did you mean…?".
- ✅ **One surface per monitor, with its own state** (`screens: each`): `examples/barra` is now one bar per screen, each with its own active workspace.
- ⬜ session lock, IME, and showing a screen inside the scene: see note 07. Ordinary windows are done (`kind: window`).

## Next step

**The direction:** this is **an alternative to Quickshell**, not the engine of one particular shell. The yardstick is [[pleamar · 07 Qué falta para igualar a Quickshell]], measured against real configurations. **Nothing in red.** After that: files and clock as services, paths (`path`), copies born at runtime (G12), windows and lock (S7), IME (E7). **Still to be tried by real hands**: popups and the tray menu (S11), the launcher with its bind (E8), the wheel (E1), notifications and tray with k4 stopped (B12, B14).
