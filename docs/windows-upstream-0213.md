# Native integration of pleamar 0.2.15

The Windows branch incorporates upstream `d6003b7becc3b91eb852a573eac41a6ed0661a1a`
without discarding the previous port's process ownership, surface geometry,
text-atlas, hidden-window and cross-monitor volume-timer corrections.

The shared compiler now uses upstream copy-on-write scopes and shared large
`let` expressions. Startup reuses the compiled scene and dependency list.
`rgb(r, g, b)`, the vocabulary/editor updates, embedded-window resource cleanup,
thumbnail deduplication and the Wayland monitor/lock/reserve/drag changes are
included. The new renderer timing diagnostic is guarded on Linux because it
references Linux-only deferred buffers. IPC quit retains the Windows event-loop
shutdown path; other platforms use upstream's wait for the renderer.

Windows implements `media.players` and `media.choose` with actual SMTC sessions.
The Luau process additions retain the port's cancellation, reload ownership,
callback identity and opt-in scene-relative working directory. This is a native
default-feature build, including Luau.

## Executed on Windows 11 x64/MSVC, October 4, 2026

| Check | Result |
|---|---|
| `cargo test --lib` | 137 passed, 28 explicit opt-in helpers ignored |
| `cargo build --release --locked --examples --bin pleamar` | Passed, six existing platform warnings |
| `python scripts/run-tests.py --binary target/release/pleamar.exe` | 230 checks passed, including 33 documentation scenes |
| `python scripts/test-compile-memory.py --profiler target/release/examples/allocation-profile.exe` | Five success/failure scenes, 50 compilations each, zero retained requested Rust bytes |
| `python scripts/windows-media-choice.py --binary target/release/pleamar.exe --fixture target/release/examples/media-fixture.exe --screen '\\.\DISPLAY2'` | Two actual owned SMTC windows: choice, transport target, missing ID refusal, auto reset, closed-player fallback; foreground unchanged |
| `python scripts/windows-rule-handoff.py --binary target/release/pleamar.exe --screen '\\.\DISPLAY2'` | Native rendered copies, burst deadline and no stale replay; foreground unchanged |
| `python scripts/windows-run-lifetime.py --binary target/release/pleamar.exe --output RUN_DIRECTORY` | Native PowerShell children stopped on reload, with/without captured output and callbacks |
| `python scripts/windows-service-reload.py --binary target/release/pleamar.exe --output RUN_DIRECTORY` | Native IPC, saved service/permission edits and six Luau reloads passed |

The new pipe regression writes over a pipe buffer before reading over a pipe
buffer, checks Unicode stdin/environment and scene-relative cwd, and requires
stderr delivery before `spawn` completion. It executes on both CI platforms.
The initial media fixture needed explicit scene event declarations and Boolean
IPC parsing; after those test-harness corrections the unchanged media backend
passed the native checks above.

The paired Marea profile passed its 18 isolated logic suites. Its software
catalogue, update list and confirmation were visually inspected on DISPLAY2
using read-only WinGet requests and IPC; no physical input was injected. Screenshots
contained the user's desktop behind transparent surfaces and are not published.
The catalogue fixture blocks installation calls. This does not validate arbitrary
third-party installers, UAC interaction or physical device compatibility.

Linux automated CI is reported on the pull request for its exact revision.
No Linux desktop session, sustained performance benchmark, mixed-DPI hotplug,
physical Wi-Fi or Bluetooth pairing was performed for this integration. The
limitations in `windows-validation.md` and the focused Windows capability docs
still apply. Developed with Codex; this remains a reviewable preview.

The final upstream refresh includes 0.2.14 and its scene-authoring skill updates.
The separate desktop-control skill is installed only on Linux: it requires
pleamar-wm's independent Wayland seat. Windows skill discovery now falls back
to USERPROFILE. Isolated CLI tests cover installation, refresh, preserving
a user-owned skill and the absence of the unsupported desktop skill on Windows.

The last upstream snapshot is 0.2.15: relative to 0.2.14 it updates only package
version/reference and adds the Linux desktop agent's `done` instruction. No
Windows runtime behavior changes in that refresh. That installed profile emitted
the renderer's existing 64-group fallback during startup. The correction below
is newer than the installed 0.2.15-preview.13 package.

## Effect-group correction, October 5, 2026

The cap counted groups, including empty ones, although siblings already shared
one GPU texture. Empty groups could exhaust it before any visible effect was
drawn. Composition now records only nonempty groups, and sibling effects and
opacity no longer lose their semantics after group 64. This does not add support
for nested effect groups or promise bounded frame time for arbitrarily big scenes.

- The two new CPU regressions failed against the previous implementation and
  pass with the correction. The complete native suite passed: 139 unit tests,
  one integration test, and 29 opt-in helpers skipped.
- The explicit GPU test passed on an RTX 5070 using DX12. It compares every tile
  in 128 alternating blur/half-opacity groups, after 128 empty groups, and repeats
  close/reopen four times. The fixture image was inspected. Each open surface
  uses one compositing texture; closing releases it.
- A default-feature release build passed. The native `group-effects-many.plm`
  fixture presented on DISPLAY2 (scale 1.25), accepted five opacity changes and
  four close/reopen cycles, published a new fact after a saved-file reload, and
  exited through IPC with status zero. No physical input was injected.

These checks cover this compositing defect. They do not establish sustained
performance, physical input coverage, desktop-agent support or full parity.
