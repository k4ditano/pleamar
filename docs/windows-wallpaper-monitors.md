# Wallpaper transition on different monitor sizes

Validated on October 3, 2026, with Codex assistance.

The native windows had the correct dimensions, but every copy of a named
`screens: each` surface shared its measured property IDs. The last configured
monitor wrote 1536 × 864 into `tide.width`/`tide.height`, shrinking the drawing
on the 2048 × 1152 primary surface. Each copy now owns its measured properties;
references inside that copy resolve to its own dimensions. Existing native
sizes are republished on scene reload; popup resizes cannot overwrite them.

The Windows Marea profile also used the main 820 × 680 control panel's size
for wallpaper cropping. It now publishes the measured tide surface sizes to
Luau and prepares one crop per monitor before starting the transition. Equal
aspect ratios share the preview cache even at different resolutions or DPI.
Preparation aborts with an error if the display topology changes in flight.

## Executed checks

- Default-feature MSVC release build, including Luau: passed.
- `cargo test --locked --lib`: 122 passed, 24 ignored, none failed. The new
  independent-surface test failed before the compiler fix. It checks logical
  2048 × 1152 and 1536 × 864, mixed DPI dimensions, and portrait geometry.
- `python scripts/run-tests.py --binary target/release/pleamar.exe`: 227
  language/documentation checks passed, including 33 documentation scenes.
- Marea `python windows/test-logic.py --binary ../pleamar/target/release/pleamar.exe
  --luau-runner ../pleamar/target/release/examples/luau-test.exe`: all isolated
  logic suites passed, including per-monitor previews, failure and topology
  change handling. These service responses are mocked.
- Marea `python windows/test-wallpaper-monitors.py --binary
  ../pleamar/target/release/pleamar.exe --hold 25 --overlay`: native DirectX 12
  rendering of the generated tide surface and its actual shader passed on two
  connected displays. Measured dimensions remained correct after a PLM reload
  inserted another property. No mouse or keyboard input was injected and no
  wallpaper setting was changed.

| Native monitor | DPI scale | Logical surface | After reload |
| --- | --- | --- | --- |
| 2560 × 1440, 180 Hz | 125% | 2048 × 1152 | 2048 × 1152 |
| 1920 × 1080, 144 Hz | 125% | 1536 × 864 | 1536 × 864 |

The screenshots below show the actual shader at its settled endpoint, with
separate solid-color test textures. The diagnostic surfaces were temporarily
placed above other windows for capture. Both colors reach all four edges.
These establish endpoint coverage, not a measurement of transition smoothness.

![1440p monitor, full coverage](wallpaper-monitors/1440p.jpg)
![1080p monitor, full coverage](wallpaper-monitors/1080p.jpg)

Mixed DPI and portrait layouts were tested in compiler/logic regressions, not
by changing this machine's display configuration. A physical hot-unplug during
the animation was not tested. Marea's current profile still supports at most
two monitors; this fix does not extend that limit or establish complete Windows
feature parity. Linux CI results are recorded on the existing port PR.
