# Design: a page loaded only while it is open (`include … while`)

**State: design, not implemented.** From pleamar#153: Vitreus's Settings, included in its bar's scene, is 6 812 of the scene's 24 451 properties and about 105 MB of the process's PSS. That is 407–409 MB with it and 298–308 MB without it (AMD), or 725 → 535 MB (natepayn3's NVIDIA), open or closed. The ask: build such a page when it opens, and let it go a while after it closes.

## 1. What a property costs (measured)

A made-up settings panel: sections of rows, each with a background lit on hover, a label, a value, a switch with its spring and a slider with its own. That gives 7 040 properties, 10 648 instructions, 3 520 rules and 3 520 zones. It was measured on the RTX 2060 PC with `examples/scene-memory.rs` (Rust's heap after reading the scene, by size of allocation), `PLEAMAR_MEMORY=1` (with the `alloc-report` feature: the heap at the scene read, the first frame and at rest, plus the render's own parts), and the running process's `smaps_rollup` (a film, no compositor).

| | exp.12 | `exp2-mem` |
| --- | --- | --- |
| Heap once read | 30.3 MB (4 514 B a property) | **12.5 MB** (1 868 B) |
| `size_of::<Instr>()` | 1 440 B | **400 B** |
| `size_of::<Style>()` | 872 B | **312 B** |
| Running: PSS / anonymous | 85.6 / 50.6 MB | **75.2 / 39.4 MB** |
| Same, empty scene | 57.6 / 22.6 MB | 57.7 / 22.6 MB |
| Reading it | 116 ms | 116 ms |
| First frame (median of 15) | 287 ms | 281 ms |

Where it went in exp.12: 25.7 of the 30.3 MB were seven lists. Every instruction took as much room as its biggest kind: a text's style held 580 bytes of colours only `markdown:` uses, and a glass took 480 bytes inline. On top of that, a list that doubles while it is read kept up to twice its room. The render's own parts were small: the drawing (elements, shapes, each shape's memo, stretches kept, the frame before), the zones, the properties and what the last drawing read came to **2.4 MB** in all. What remains once read in `exp2-mem`: the instructions (4.3 MB), the zones (552 B each, 1.9 MB), the rules, and the expressions' boxes (32 B each, 2.2 MB).

**What this panel does not explain:** Vitreus pays about 15 KB a property, four times this panel even before the cut. Its scene was asked for. The likely rest is what its pages draw: pictures and icons decoded, text laid out (each text's lines, kept by the text workshop), and the logic's own data. Those are measured with `PLEAMAR_MEMORY=1` on its scene before going on (phase 3).

The general cut (`exp2-mem`) helps every scene, open or not: about 1 KB less per instruction. Deferring a page saves what is left of it, and more for the parts this panel does not have.

## 2. Syntax

```plm
include "pages/settings.plm" while panel == settings
include "pages/settings.plm" while panel == settings keep 30s
```

- **`while EXPR`**: the page is there while it holds. The expression is read like an `open:` (facts, properties, lets of the scene). It may not read anything of the page itself.
- **`keep DURATION`**: how long it stays after the condition stops holding (default 10 s). Opening it again in that time is instant. `keep forever` builds it the first time and never lets it go (only the first opening is paid).
- It goes wherever an `include` goes: at the top of a scene or inside a `group`. A page may include others, plain or with their own `while`. The file is a `part` as today; nothing in it changes.

## 3. What is deferred, and what is not

**Deferred** (built when the page loads, let go when it unloads):
- what it draws (its instructions, the components and `repeat`s it unfolds);
- its properties (springs, `prop`s, a zone's hover and pressed);
- its zones;
- its rules (pressed, dragged, keys, `every`, events heard inside it);
- its behaviours (`follow`, `wave`, `blink`…), its gestures and layer claims, and its `let`s;
- what the render keeps for it: its texts laid out, its glyphs (the atlas lets go of what nothing draws), the pictures only it shows.

**Not deferred** (lifted to the scene, always there, cheap): its `fact`s, its live `text`s (a field's words), its `model`s, its `event`s and `signal`s, and its image declarations (the picture is read only when drawn). That way the logic writes them while the page is closed, and opening it shows the state of now. This is what the issue asks for, and it is what a part's `fact` already is: the scene's.

A page loads with its properties **at their written values**: a scroll or a half-open drawer inside it starts again. What has to last goes in a fact or a live text, as everything that crosses the boundary already does. Keeping a page's properties alive across an unload is left out on purpose: that would keep what we want to free.

## 4. What sees what

**From outside, nothing of the page is named.** No property, zone, `let` or component of it can be read or written from outside, including by a rule, another page, the `while` itself, a `show:`, a `--say … press` or an `import scene`. The compiler says so where it is read:

```
settings.plm:12:9: 'wifi_row.hover' lives in the page `pages/settings.plm`, which is there only
while `panel == settings`: tell it with a fact, or listen to one of its events.
```

**The page sees the whole scene**: its facts, its properties, its lets, its components. It may write the scene's facts and properties and emit events, because the scene is always there.

So the only way into a page is the scene's own boundary: facts, live texts, models and events. These are exactly what survive its unloading.

## 5. When it is not there

- **An event to an unloaded page is not heard.** Its rules are not there; nothing is queued for it. It is the same as a surface that is closed today (its copies asleep, `Span`). A page that has to know something when it opens reads it from a fact, which is there.
- **The logic** names no zones or properties (only facts, texts, models and events), so nothing in it breaks. An event it emits that only the page listens to goes nowhere, as above.
- **An agent** (`describe`, `press`, `wait`): the page is not on screen, the same as a closed popup: `? there is nothing called 'save' on screen`. A `wait` on one of its names waits until it is there.
- **A screen reader** (AccessKit): its nodes are not in the tree. When it loads they come in one update (a diff), and they go the same way. Their node ids are their zones' places, which stay the same across unloads (section 7), so a reader that kept a node finds the same one when the page comes back.

## 6. Opening it for the first time

Measured on the PC (exp.12 and `exp2-mem` alike): reading the whole 7 040-property panel takes **116 ms**, against 5 ms for an empty scene, about 16 µs a property. A page compiled alone, in the scene it lives in, should cost about the same per property:

| | estimate |
| --- | --- |
| compiling a 7 000-property page | ~110 ms on that CPU |
| putting it in place (its lists, its zones, the render's indexes) | a few ms |
| its first drawing | as a frame where everything changed: some ms on this panel |
| laying out its texts | in the text workshop, off the render's thread: they appear as they are laid out, as at a scene's start |

So a first opening of Vitreus's Settings would be about **one or two frames late at 60 Hz** (120–150 ms), plus its texts coming in. After that, within `keep`, it is instant. Two ways to hide it, both later and both optional:
- **Read ahead:** the page's source is compiled on another thread when the logic sets a hint (`prefetch settings`), for a menu about to open.
- **When idle:** compiled once the render has been at rest for a while, and kept compiled but not put in place (only its instructions, not the render's parts). That gives back part of the saving.

The page's source is kept (a few KB) so that it is not read from disk again, and so that a page saved while closed is the new one when it opens.

## 7. How it would be built

The render reaches a scene's things by their place in its lists: an instruction's index (`Zone::at`, `Span`, `skip`, each shape's memo, the stretches kept), a property's, a zone's (AccessKit's node ids too), a rule's. Those places have to stay put while a page comes and goes.

1. **Places set aside.** The scene is compiled whole once at start, to check it and to give every page its places in each list: properties, zones, rules, behaviours, texts' facts. A page's places stay its own across unloads. Its compiled form is then dropped, except its boundary (section 3).
2. **Its instructions in a list of their own.** The scene's instructions keep, where the page is included, one `Instr::Page(k)`. Composing goes into page `k`'s list when it is loaded and skips it when it is not, as `skip` does now for a closed copy. An instruction's place becomes (page, index) where it is kept (`Zone::at`, spans, shape memos, stretches). Of everything here, this is what touches the most code.
3. **The rest in place, empty when unloaded.** Properties are 16 bytes each (`Animated`) and are kept as they are; their springs are not run. Zones and rules unloaded are an empty entry (`Option<Box<Zone>>`), so 8 bytes instead of 552.
4. **Loading** compiles the page alone against the scene (its names, components and lets), with the places it was given. It checks that it gives the same counts. If they differ (the file was saved while closed), the whole scene is reloaded, as a save does today. Then it fills its places and tells the render, which drops its memos for those places.
5. **Unloading**, `keep` after the condition stops holding, empties those places and drops what the render kept for them. That includes what the text workshop laid out for its texts and the pictures only it draws.

## 8. Phases

| # | What | Gives | Risk |
| --- | --- | --- | --- |
| 0 ✅ | The general cut (`exp2-mem`): small instructions and styles, lists without spare room | every scene, ~1 KB less an instruction; the panel 30 → 12.5 MB | low: the same frames, a size test |
| 1 | `include … while` that only **sleeps** (rules and drawing skipped, as a closed copy's `Span`), plus the compiler's rule of section 4 | the CPU of a closed page; the language and its errors settled | low: it reuses what closed copies already do |
| 2 | **Unloading**: places set aside, a page's instructions in their own list, zones and rules emptied, load and unload with `keep` | the memory of what the page holds (on the panel, most of its 12.5 MB) | **high**: every index into the instructions; tests open, close and reopen pages with films, the agent's tree and AccessKit's |
| 3 | What the render keeps for a page (laid-out texts, glyphs, pictures), measured first on Vitreus's scene | the rest of Vitreus's 105 MB | medium: who else uses a laid-out text or a picture |
| 4 | Reading ahead (`prefetch`) and compiling when idle | a first opening without a late frame | low |

Risks along the way:
- An index missed in phase 2 draws or presses the wrong thing. Anything kept by instruction index in the render (zones, spans, `skip`, shape memos, kept stretches, `z:` arrangements) has to become page-aware at once, behind the films that compare a page opened, closed and opened again with one never closed.
- A page whose `while` flips often (a hover) loads and unloads all the time. `keep` covers it, and a page under a second old is never let go.
- Hot reload: a page saved while closed loads its new version, or the scene reloads if its counts changed. Saving while open reloads the scene as today.
- `screens: each` copies and `repeat` inside a page: each copy is a page of its own, or the page is one for all copies. To be decided in phase 2; phase 1 treats a page in a copy as part of that copy.
