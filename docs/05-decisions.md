# Decisions and open questions

## Decided

| Date | Decision | Why |
| --- | --- | --- |
| 2026-09-19 | **The renderer owns the animations**; the logic declares intentions | It is the idea this set out to prove, and it was measured: 38 frames against 0 with the logic blocked |
| 2026-09-19 | **A scene is data**, not code | If the renderer has to work on its own, it cannot depend on running anything foreign |
| 2026-09-19 | **SDF shapes** in an interpreter shader | Blending, shadowing and hit-testing all come out of the same formula |
| 2026-09-19 | **Own language, A+B**: element tree + everything declarative the renderer can do on its own | C (plain Luau) is more powerful but cannot guarantee what runs in the renderer. That boundary is the project |
| 2026-09-19 | **Luau only for the logic** | Sandbox, gradual types, LSP already done, and the AIs write it well |
| 2026-09-19 | **Boring syntax, new semantics** | So anyone coming from QML reads it first time; the only thing to learn is what makes it different |
| 2026-09-19 | **`capa` is the central concept, not `estado`** | The test against Marea: `forma` is a slot many claim, and today it is defended with eight copied guards |
| 2026-09-19 | **Two ways to animate**: springs and keyframe gestures | Marea has 25 gestures that are already keyframe data; a spring does not tell a story |
| 2026-09-19 | **Semantics in the runtime first, the parser after** | A parser freezes the syntax; which concepts are needed is not fully known yet |
| 2026-09-19 | **A layer shape is only seen with more than half presence** | Blending two eye shapes gives a smear; since the presences add up to one, this way one leaves and the other comes in |
| 2026-09-19 | **An `estado` is a claim that sets properties**; there is no separate construct | Opening/closing Marea's card came out as `capa tarjeta { abierta mientras abierta?; reposo }`, with nothing new |
| 2026-09-19 | **The language keywords, in English** (`layer`, `gesture`, `while`, `after`…) | Abel's decision. k4 already gets outside PRs and changing it later is expensive. The runtime's Rust code stays in Spanish for now; the parser will act as the boundary |
| 2026-09-19 | **Cross-platform: not now, but not ruled out — and always present.** Everything system-related goes behind `src/platform/`; the core only uses crates that exist on Linux, Windows and macOS; `cargo check` against Windows has to pass | Abel's decision. 80 % is portable already (`wgpu`, the model, the language, Luau). What is not: the window (layer-shell does not exist elsewhere) and the services, which go through the data graph. First candidate to port: **Marea**, not k4 —on a Mac the Dock cannot be replaced—. First system: Windows |
| 2026-09-19 | **Limitations get written down as they come up, each with its fix plan** ([[pleamar · 08 Limitaciones conocidas]]) | Abel's decision: no hidden debt |

## What implementing it taught

- **Marea's logic came down to ten lines.** Opening, closing, falling asleep, highlighting the button and the hop on press are rules. What reaches the logic is `Capa("tarjeta", "abierta")`, to do *its* job, and `Suceso("ver-evento")`.
- **The island's logic decides nothing**: two layers and three rules.
- `Alternar(hecho)` was needed —press to open and close— and `Fuera{durante}` has to "arm" itself: it only counts if the pointer was over it before, or it would close right after starting.
- What a keyframe emits is handled on the next frame. For a camera shutter that is 16 ms; whether it matters remains to be seen.
- **Pending:** two layers setting the same property (today the last one to change wins), different transitions depending on *where* it comes from, and gestures with parameters (`señalar(lado)`).

## Open

| Question | Options | Note |
| --- | --- | --- |
| What is the language called, and its extension? | — | pleamar is the runtime; the name came from Claude and can be changed |
| The priority order of `capa forma` | See [[pleamar · 04 Prueba - cabe Marea]] §3.2 | Claude deduced it from the comments. It has to be reviewed line by line |
| Does `confirmado()` while recording wipe the red disc? | — | Seen by reading `ExpressionController.qml`, without running it. Check in Marea |
| Written or inferred types? | `hecho cuenta: entero` · `hecho cuenta = 0` | |
| What happens if two layers set the same property? | Error at load · the layer declared first wins | Probably an error |
| Components, repetition, layout, dynamic text | — | Not designed. Without this there is no notification list |
| CPU rendering for the static parts? | — | It would cut the ~100 MB the Vulkan driver adds |
