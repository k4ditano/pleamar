# Renderer optimization — 2026-10-03

Developed with Codex. This follow-up keeps the existing frame rate, effects and
input semantics. It does not declare complete Windows desktop parity.

## Changes

Hit testing and native input regions share evaluated zone geometry. A zone keeps
its flattened curves, transforms, viewport clips and bounds until a property,
spring velocity or fact used by that geometry changes. Active/hidden checks
still run normally. Each scene load replaces the caches, including reloads with
the same number of zones. Storage is bounded by the current scene's zones and
path limits; there is no history of old shapes.

The composition snapshot reuses its vectors and unchanged text strings instead
of allocating a new copy on every animation frame. Scene reload also explicitly
invalidates the previous draw list: a changed constant must repaint even if all
live property/fact values are unchanged.

## Verification

The unit regressions compare cached and original geometry across ellipses,
rectangles, arcs, segments, curves, strokes, rotations, nested transforms and
two polygon viewport clips. They vary property values, velocity, facts and
collapsed dimensions, and replace constant geometry. Native regression input
goes directly into the renderer: no desktop mouse or keyboard is injected.

```powershell
cargo test --release --locked --lib
python scripts/run-tests.py --binary target/release/pleamar.exe
python scripts/windows-zone-cache.py --binary target/release/pleamar.exe --screen '\\.\DISPLAY2'
$env:PLEAMAR_ZONE_BENCH_SCENE = '../marea-plm/marea-desktop.plm'
cargo test --release --locked --lib scene_geometry_benchmark -- --ignored --nocapture
```

The native test uses a panel on the specified monitor, with no keyboard focus,
work-area reservation or system services. Exactly three scripted clicks must
reach it; clicks at its old position, while disabled, and at the old position
after a hot reload must not. It also checks the output names and successful
Luau/runtime shutdown. This is a native rendering/input test, not a screenshot
review or physical input test.

## Measurements

Windows 11 x64/MSVC, RTX 5070, default Luau. A game remained running on the
primary display. Compiles used below-normal priority and two Cargo jobs.

An isolated release benchmark evaluates bounds and hit tests for all 586 Marea
zones, 300 rounds per case. The unchanged and changing paths must agree with the
original implementation. One run measured:

| Geometry | Original, µs/round | Cached, µs/round |
|---|---:|---:|
| Unchanged | 504.72 | 41.45 |
| Every property changes every round | 424.83 | 316.75 |

These are geometry costs, not total application CPU or display FPS. The native
comparison uses the same generated Marea drawing on the secondary display,
Classic look, two 20-second samples each with its panel closed and open. It uses
isolated state and a minimal Luau driver, without Marea's desktop commands or
user preferences; the five declared read-only services remain active. Other
surfaces are closed, keyboard requests and work-area reservation are disabled,
and `--screen` confines creation to the selected output. The ordinary installed
Marea stays running throughout both variants.

The final native comparison completed with no runtime errors and only DISPLAY2
surfaces. Means of the two samples, expressed as a percentage of one CPU core:

| Panel | Previous engine | Updated engine |
|---|---:|---:|
| Closed | 13.91% | 14.50% |
| Open | 22.97% | 21.21% |

Update means stayed around 16.7 ms. The open-panel samples improved, but the
closed-panel samples did not; this does not establish a general CPU percentage
saving. Final resident/private process samples were 271.20/335.92 MiB before
and 284.22/368.84 MiB after. There is **no demonstrated RAM reduction** in this
comparison. The geometry cache is a small bounded addition; process/driver and
allocator variation cannot be attributed to it from these short samples.

The final MSVC/default-Luau build passed 126 library tests (26 opt-in tests
ignored), 227 language/documentation checks and 15 Marea logic suites. The
secondary-monitor native input/reload regression passed. The original native
test attempt selected an absent output because the test generator used JSON
escaping for a PLM literal; correcting that test literal resolved it. No product
failure was inferred from that attempt.

Marea's `windows/measure-desktop.py --screen '\\.\DISPLAY2'` records the actual
surface names and rejects a run that creates a surface on another output.
The fixture and measurement are reproducible with:

```powershell
python scripts/windows-render-benchmark.py --marea ../marea-plm --binary <engine.exe> --screen '\\.\DISPLAY2' --output <new-evidence-directory>
```

Update intervals and process CPU are recorded separately. Concurrent gameplay
and driver/allocator caches make these short samples unsuitable for a general
RAM-saving or game-FPS claim. No working-set trimming or lower animation rate
is used.

## Composition preparation follow-up

The renderer now prepares instruction order and hidden-group jumps once, then
keeps them until a new scene, a changed z order or a changed diagnostic banner.
The previous path copied the z arrangement, allocated a sequence and walked all
instructions to find group endings on every composition. Marea's generated
desktop contains 9,776 instructions, including many hidden controls.

The prepared traversal preserves the previous treatment of particle emitters:
their clocks are visited even inside hidden groups. Visibility, opacity, effects
and transforms are still evaluated each frame. Clip/transform/opacity stacks,
the closed-surface mask and gradient stops reuse storage; a scene reload drops
that scratch storage. The public `DrawList::compose` entry point still builds its
own traversal, so callers outside the runtime do not acquire a new invalidation
obligation.

GPU uploads also compare each of the four drawing buffers against the existing
previous-frame snapshot. Only changed buffers are queued. This adds no second
copy of drawing data; comparison uses float bits, including signed zero and
NaN. Newly allocated buffers and invalidated snapshots always get a full upload.
The public `Gpu::upload` entry point retains its unconditional behavior.

Reviewing invalidation also exposed two diagnostic-banner omissions: a banner
could reuse the previous draw list, and z ordering could leave the appended
banner outside the draw sequence. Banner changes now invalidate composition,
and the banner follows the ordered scene. This does not reorder scene controls.

Regression tests compare the new traversal with the previous algorithm for
groups, effects, emitters and changing z order; compare drawing buffers and
hidden regions across visibility changes; and cover diagnostic append and a
same-length structural edit. The native `windows-zone-cache.py` fixture also
changes z order and reloads a group with a different structure on its selected
output. Its input is renderer-scripted, not physical desktop input.
Buffer-upload tests cover independent changes, size changes, signed zero, NaN
and invalidation.

The isolated preparation benchmark can be repeated without opening a window:

```powershell
$env:PLEAMAR_COMPOSE_BENCH_SCENE = (Resolve-Path ../marea-plm/marea-desktop.plm).Path
cargo test --release --locked --lib scene_preparation_benchmark -- --ignored --nocapture
```

This measures preparation that is avoided, not a whole-application speed ratio.
The retained sequence and jump tables take 156,416 bytes for this scene, released
when replaced. Native comparisons use the one-output benchmark above, with
`--seconds 15 --repeats 2`, and compare against preview.6 with the same generated
scene, animation rate and concurrent desktop workload.

On this machine, the previous structural preparation took 48.7 microseconds per
composition. Reusing it removes that preparation on unchanged structure; the
benchmark's constant-time access is not a useful application speed ratio.

The native comparison against preview.6 completed with the same fixture hash,
no runtime errors, no desktop input and only DISPLAY2 surfaces. Mean process CPU
over two 15-second samples per state, as a percentage of one CPU core:

| Panel | Preview.6 | Follow-up |
|---|---:|---:|
| Closed | 13.49% | 11.98% |
| Open | 22.92% | 19.80% |

Mean update intervals stayed at 16.67–16.69 ms. Final resident/private memory was
284.01/371.28 MiB before and 274.66/329.41 MiB after. These are observed endpoints,
not a demonstrated long-term RAM saving; gameplay, driver caches and allocator
variation remain uncontrolled. No animation rate or effect was reduced.

The final default-Luau MSVC release passed 130 library tests (27 opt-in tests
ignored), 227 scene/documentation checks and 15 Marea logic suites. The native
input/reload test passed after fixing its startup polling: the pipe can be ready
before DX12 answers a renderer request. An earlier test setup also mistakenly
loaded the existing negative nested-effects example as a valid scene; that test
input was replaced with the valid group-modes example. Neither failed attempt is
counted as a passing run. This follow-up adds no physical-input/screenshot claim.
