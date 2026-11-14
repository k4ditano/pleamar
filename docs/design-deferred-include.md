# Design: a page loaded only while it is open (`include … while`)

**State: phase 1 done** (`include … while` sleeps, and the compiler keeps the page's names inside it: section 9); **phase 2 not started**, see section 10. From pleamar#153: Vitreus's Settings, included in its bar's scene, is 6 812 of the scene's 24 451 properties and about 105 MB of the process's PSS. That is 407–409 MB with it and 298–308 MB without it (AMD), or 725 → 535 MB (natepayn3's NVIDIA), open or closed. The ask: build such a page when it opens, and let it go a while after it closes.

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

## 1b. Vitreus itself (measured)

`natepayn3/Vitreus` at `2c8cda4`, its bar's scene and its Luau only, inside pleamar. It ran as a client of a headless pleamar-wm on the RTX 2060, with a fresh HOME. Its `run:` programs could not start (an empty `PATH`) and its D-Bus services were turned off (no bus), so nothing of the system answered it. Sampled 25 s in, after pleamar gives back what reading the scene used (`release_memory`).

| | with Settings | without (the `include` commented out) | Settings |
| --- | --- | --- | --- |
| scene | 24 460 properties, 70 753 instructions | 17 658, 51 233 | 6 802, 19 520 |
| exp.12: PSS / anonymous | 471 / 414 MB | 379 / 326 MB | **92 MB** |
| `exp2-mem`: PSS / anonymous | 384 / 331 MB | 325 / 272 MB | **59 MB** |
| `exp2-mem`: heap once read | 163.7 MB | 117.1 MB | 46.6 MB |
| `exp2-mem`: heap at rest | 182.4 MB | 131.7 MB | 50.7 MB |
| render's own parts | 5.5 MB | 4.5 MB | 1.0 MB |
| Expressions' nodes (32-byte boxes) | 3 627 101 (110.7 MB) | 2 538 428 (77.5 MB) | 1 088 673 (33.2 MB) |

- The general cut alone takes **87 MB** off the bar with Settings (−18 %), and Settings' own cost goes from 92 to 59 MB.
- Its absolute PSS is not the author's 407/303 MB: there is no compositor, card or services of his here. The difference with and without Settings is comparable, and it is near his (92 MB here against his 104, 725 → 535 MB on NVIDIA).
- **What Settings costs is its compiled scene**: 46.6 of its 50.7 MB of heap, against 1 MB for the render's parts. And its compiled scene is mostly **expressions**: about 160 nodes for each of its properties, 56 for each instruction. They are written inline, as Vitreus does throughout (`at: floor(lx + 424 * pu + 0.5), floor(lry + (lr * 44 + 4 + 9) * pu + 0.5)`), and copied once more for every turn of a `repeat` and every use of a component.
- Each node is an allocation of its own: 32 bytes asked for, 48 taken by glibc. Over the 3.6 million nodes of the whole bar that is about 174 MB of resident memory, close to half the process. This is where the ~15 KB a property goes.
- `let`s are not the culprit: one of more than 8 nodes is already worked out once, as a property (`computed_once`).

What would cut it, for every scene (not done here: it touches every expression the language makes):

- **Each expression flat**, its nodes in one list (one allocation, about 12 bytes a node) instead of one box each: the bar's 3.6 million boxes would be some 70 000 lists. Estimated: 174 MB → about 45 MB, evaluated as fast or faster (no pointers to follow).
- Or, smaller, **leaves inline**: a constant, a property or a fact kept in its parent instead of in a box of its own (most of a tree's nodes are leaves).

**Leaves inline is done** (`Kid`, 2026-11-14): Vitreus's boxes 3.63 → 1.45 million, PSS with Settings 386–413 → 286 MB, and a frame's putting together 15 % faster. The flat form would take the rest. Either goes before or with phase 2 of this design: a page unloaded no longer holds its expressions, but every scene pays them while it is open.

**Each tree in one block: tried and parked** (branch `exp2-flat`, `acc2850`, on exp.15). The same `Expr`, but each expression the compiler reads is moved, once read, into one allocation of its nodes side by side (a child points inside its parent's block), and the old boxes are freed. Evaluation did not change a line. Measured on the RTX 2060 against exp.15, alternating builds:

| | exp.15 | `exp2-flat` |
| --- | --- | --- |
| Vitreus's 32-byte boxes | 1.45 M | 340 k (the rest: nodes the compiler makes outside `expr`: components, colours, opacity products) |
| Vitreus with Settings: PSS / anonymous | 285–288 / 228–230 MB | 274–275 / 217–218 MB (−12 MB, −4 %) |
| without Settings: PSS / anonymous | 257–259 / 200–202 MB | 250 / 192–193 MB (−8 MB) |
| heap once read | 115.9 / 84.7 MB | the same (the same bytes in fewer blocks) |
| reading the scene (`--check`, median of 9) | 1 572 / 1 082 ms; the 7 040-property panel 115 ms | **1 651 / 1 143 ms; 121 ms (+5 %)** |
| putting a frame together, 7 000 rows moving | 7.14 ms | 6.79 ms (−5 %) |

It was estimated at 15–20 MB and gave 8–12. It was parked: a scene read 5 % slower is felt at every start and every save, and the rule for this work is that nothing gets slower. To get the rest without that, the compiler would have to build each expression in its block from the start instead of moving it there (more work, and touching every place that makes nodes).

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
| 1 ✅ | `include … while` that only **sleeps** (rules and drawing skipped, as a closed copy's `Span`), plus the compiler's rule of section 4 | the CPU of a closed page; the language and its errors settled | low: it reuses what closed copies already do |
| 2 | **Unloading**: places set aside, a page's instructions in their own list, zones and rules emptied, load and unload with `keep` | the memory of what the page holds (on the panel, most of its 12.5 MB) | **high**: every index into the instructions; tests open, close and reopen pages with films, the agent's tree and AccessKit's |
| 3 | What the render keeps for a page (laid-out texts, glyphs, pictures), measured first on Vitreus's scene | the rest of Vitreus's 105 MB | medium: who else uses a laid-out text or a picture |
| 4 | Reading ahead (`prefetch`) and compiling when idle | a first opening without a late frame | low |

Risks along the way:
- An index missed in phase 2 draws or presses the wrong thing. Anything kept by instruction index in the render (zones, spans, `skip`, shape memos, kept stretches, `z:` arrangements) has to become page-aware at once, behind the films that compare a page opened, closed and opened again with one never closed.
- A page whose `while` flips often (a hover) loads and unloads all the time. `keep` covers it, and a page under a second old is never let go.
- Hot reload: a page saved while closed loads its new version, or the scene reloads if its counts changed. Saving while open reloads the scene as today.
- `screens: each` copies and `repeat` inside a page: each copy is a page of its own, or the page is one for all copies. To be decided in phase 2; phase 1 treats a page in a copy as part of that copy.

## 9. Phase 1, as built (`exp2-include`)

- **Syntax** as in section 2: `include "pages/settings.plm" while panel == settings [keep 30s | keep forever]`. `keep` is read and kept (`Page::keep`); while only sleeping it changes nothing.
- **Where the page is.** The include is put in place as always, between two marks: the `include` line, which carries the condition, and a bare `include` where the part ends (one no one can write). In the drawing pass the first mark notes how long the instructions, behaviors and rules are, and the second one makes the page's spans of them (`Scene::pages`, as `Scene::spans` for a monitor's copy). A page inside a copy per monitor is one page per copy, with its condition read in that copy. A loose `clip` in a page holds until its end, so that what is skipped is whole. Each edge of a page gets an instruction that does nothing (a transform that moves nothing and its end): a zone stands at the place of the instruction after it, so without them the button written just before the page —the one that opens it— stood at the page's first place and slept with it, and the page's last zone stood outside it.
- **Asleep**: while its condition does not hold, its instruction span is skipped by the drawing (`draw.skip`) and is absent for the zones, the agent and the screen reader (`absent`, `gone`), its behaviors do not run (they catch up when it wakes, as a copy's), and its rules are not looked at (`asleep_rules`).
- **Section 4 in the compiler.** Before reading, each file is known to be a page's or not (the files between its marks, its own includes too). What a page declares that is not its boundary —props, zones, measures, `let`s, components— is noted as its, and reading one from outside the page (its condition too) is an error, said where it is read. Checked where a name is resolved to a property, a `let` or a zone, and where a component is used; in a copy of a component, by where the copy is placed (a component of the scene used both in the page and outside it is fine: each copy's names are its own). Vitreus with `include "pages/settings_panel.plm" while panel == settings or ds > 0.001` (the `or ds` keeps the drawer's fade-out) loads as it is: nothing outside its Settings names anything of it.
- **Measured** (RTX 2060, headless, Vitreus `2c8cda4` with `run:` and D-Bus off; exp.15 and this branch alternating, 7 rounds for memory and CPU, 11 for reading):

| | exp.15, plain include | this branch, `while` |
| --- | --- | --- |
| PSS / anonymous, Settings closed | 283–310 / 225–252 MB | 282–310 / 225–252 MB |
| the same, opened (`fact panel 6`) | 287–314 / 227–254 MB | 287–316 / 228–256 MB |
| CPU over 10 s of a pointer along the bar, closed | 2 060–2 200 ms (mean 2 130) | 1 970–2 070 ms (mean 2 023, −5 %) |
| reading the scene: CPU (median) · instructions | 1 543 ms · 8.674 G | 1 582 ms · 8.851 G (+2 %: its names checked) |

  A scene without a `while` reads as on exp.15: the same instructions (Vitreus with Settings 8.674 / 8.677 G, without 5.996 / 5.996 G, the 7 040-property panel 0.590 / 0.590 G, counted with `perf_event_open`), CPU within 0–10 ms. Seventeen scenes film the same frames. Memory does not change, as expected: sleeping frees nothing. The CPU saved is small on Vitreus because it already gates its Settings by hand (its rules say `while panel == settings`, its drawing is hidden); a scene that does not gets more. Opening is instant (nothing to build).

## 10. Phase 2: what was learned before starting it

- **A first opening would not be 120–150 ms.** Section 6 estimated it from the made-up panel (16 µs a property). Vitreus's Settings adds **about 470 ms of CPU** to reading the scene on this PC (1 543 against 1 074 ms, with and without it; 2.7 G instructions): 70 µs a property, its expressions being written inline and unfolded by `repeat`s and components. Compiling the page alone would cost about that, so a first opening would be some 30 frames late here (less on the author's CPU), and every opening after `keep` ran out.
- **Compiling a page alone** is what this compiler is least made for. A scene is read in three passes over the whole tree (declarations; `let`s and layers; drawing, with rules gathered and read at the end), and the page's drawing depends on the state at its include: the scopes (a copy per monitor's, a component's), the transforms and clips it is under, the `z:` parents, the `let`s read so far (a group does not keep its lets: the page reads whatever is loose before it), the components, colours and springs. Two ways:
  - **(a) Read the whole scene again** on another thread when the page opens, and take its part (its instructions moved to their own list, section 7, step 2). Simple to get right, but the opening is as late as reading the scene: ~1.6 s here.
  - **(b) Keep what the page needs at its include** (a snapshot of that state, the page's syntax and the `let`s it reads) and compile only it: ~0.5 s here. The snapshot is kept in memory, and it is a new way into the compiler that every statement has to respect.
- Either needs section 7, step 2 (a page's instructions in a list of their own, every index into the instructions made page-aware), which is the riskiest part of all.
- **What it would free**: Settings costs 24–28 MB of PSS here, ~59 MB in the author's session.
- So before phase 2: decide whether a late first opening (~0.5 s, then instant within `keep`) is worth ~25–60 MB, or go first for reading ahead (phase 4: compiled on another thread when a hint says it is about to open), or make a property cheaper to compile and to keep (which also helps every scene).

## 11. Reading a scene: where the time goes (`exp2-read`)

Before unloading pages (phase 2), reading a scene had to get cheaper: a page compiled when it opens costs what it costs to read. Profiled on the RTX 2060 PC with a sampler of its own (`perf_event_open`, cycles with call chains, a build with frame pointers; there is no `perf` there) on `pleamar --check` of Vitreus's bar (exp.16):

- **Almost all of it is the drawing pass.** `PLEAMAR_TIMING=1` now says it: pass 0 (declarations) 14–19 ms, pass 1 (`let`s, layers) 20–24 ms, **pass 2 (drawing) ~1 000 ms**, zones 17 ms, rules 31 ms. Before compiling, reading the files (tokens, includes) is ~7 %, and checking the Luau 2 %.
- **Why 70 µs a property, against the made-up panel's 16:** not a cost per property, but how much expression each one carries. Vitreus writes its expressions inline (`at: floor(lx + 424 * pu + 0.5), …`) and unfolds them by `repeat`s and components: ~148 expression nodes a property (3.6 M for 24 460, section 1b), against ~10 in the panel. Per node, Vitreus is cheaper.
- **Where a node's cost went** (share of the samples, exp.16): hashing names, 13–14 % (SipHash, for every name read, through each scope's aliases and the scene's maps); **working out every operand's name twice**, ~8 %: once to read it and once more to ask whether it is an enum one may not add to (`no_arithmetic`); allocation, ~15 % (malloc, free, copies); printing each big `let` with `{:?}` to find whether it was already computed once, ~3 %; noting every name read for the editor even when nobody asked, ~4 %; cloning a `let`'s tree into every place it is read, ~5 %; constant folding, ~3 %; a scope copied when something is declared in it after a rule kept it, ~3 %.

**Cut** (all of it the same compiled scene: a hash of everything the compiler makes —instructions, rules, behaviors, zones, props, facts, texts, surfaces, `z:` blocks, pages, spans— is the same for all 387 scenes of `tests/`, `examples/` and Vitreus, against exp.16):

1. The compiler's maps hash with Fx (the hasher rustc uses for the same job) instead of SipHash. Their keys are the scene's own names, and nothing depends on their order (the default one already changes each run).
2. An operand just read as a value keeps the scene's name it was worked out to, and the enum check uses it instead of working it out again. This only covers the scene's names: a component's parameter or a turn's own is worked out as before (most of Vitreus's operands are such, so this gives back only part of those ~8 %).
3. A big `let` computed once is found by its tree (`Expr::same`, `Expr::same_hash`: the same as its `{:?}` says, a number by its bits) instead of by printing it.
4. The names for the editor (`declared`, `used`) are noted only when the editor asks for them (`index`).
5. `resolve` builds a copy's marked name in a scratch string, and the name without the mark only when it is needed.

| | exp.16 | `exp2-read` |
| --- | --- | --- |
| Vitreus with Settings: instructions · cycles | 8.68 G · 5.14 G | 6.52 G · 4.13 G (−25 % · −20 %) |
| the same, wall (median of 11) | 1 601 ms | 1 301 ms (−19 %) |
| without Settings: instructions · wall | 6.00 G · 1 104 ms | 4.40 G · 881 ms |
| the 7 040-property panel: instructions · wall | 0.59 G · 116 ms | 0.44 G · 101 ms |

**Running is the same.** Seventeen scenes film the same frames as exp.16. Putting a frame together with 7 000 rows moving: 7.08–7.29 ms on exp.16, 6.88–7.06 ms here. Vitreus with a pointer going along its bar for 10 s: its render thread runs 3.83 G cycles on exp.16 and 3.76 G here (IPC 2.1 both; 1.4 % more instructions, since it starts sooner and draws a few more frames in the same window). Its PSS once read is 285–310 MB on exp.16 and 271–278 MB here: less of the compiler's leftovers stays behind.

**What is left, and would take more:** pass 2 is still ~1 s on Vitreus. The rest is spread: the scope alias walk in `global` and the enum check for a component's parameters (~6 %, kept as it was, for which names count as an enum must not change), cloning `let` trees into each use (sharing them would change `Expr`), the scopes copied on declaring, tokenizing. Reading pass 2 of the copies per monitor in parallel would not help Vitreus (one copy) and the compiler's state is not made to be shared.
