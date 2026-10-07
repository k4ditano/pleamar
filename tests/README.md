# Language tests

One small `.plm` per thing the language promises. The first line says what is expected:

```
// expect: ok
// expect: error «the records of «rows» have no field»
```

`./run-tests.sh` runs them all through `pleamar --check` and says which ones do not hold. An expected error is checked by a piece of its message: that way we also watch that errors keep saying something useful.

On Windows, use `./run-tests.ps1` or `python scripts/run-tests.py` (Python 3).

Windows unit tests also deliver `TaskbarCreated` to owned hidden HWNDs and check
that active panels invalidate their old reservation and queue a placement, while
normal/retired windows remain unchanged. A queued placement cannot move a retired
panel. These tests do not restart Explorer, reserve desktop space or prove actual
Shell recovery; test the latter interactively with a reserving panel.

`python scripts/windows-run-lifetime.py --binary target/release/pleamar.exe --output "run lifetime ñ"`
checks saved-file Luau reloads with bounded native PowerShell helpers. It verifies
that the old helper exits and its callback is discarded, both with captured
output and without output/callbacks. Use a new output directory. The fixture
selects an absent monitor and is not graphical validation. Portable unit tests
also cover retirement and preserving stdout/exit codes with heavy stderr.
Pass `--binary target/debug/pleamar.exe` to Python to test an existing build.
This runner also checks exit codes, documentation scenes, vocabulary coverage
and editor highlighters. `native-interaction.plm` and its Luau companion provide
the interactive checklist described in [Windows validation](../docs/windows.md).

When a language bug gets fixed, its minimal case ends up here.

The libraries the tests import live in `tests/common/`; those are not checked on their own, because a library is not opened: it is imported.

Windows keyboard-focus regression: run `native-underlay.plm` and
`native-overlay.plm` in separate processes. Activate the underlay, click the
overlay's green area, and confirm the underlay reports `focused = false`.
Send `emit disable` to the overlay. Without clicking or activating another
window, the underlay must report `focused = true` and an increased
`focus_gains`. Also check that closing a panel after selecting a different
application does not steal focus from that application.

`native-drag.plm` exercises the native transfer protocol without launching or
opening received content. Drag the Unicode card onto the right-hand target and
check its text, MIME type and transfer counter. Set `PLEAMAR_DRAG_FILE` to a test
file's absolute path (include spaces and Unicode), then drag the lower card.
Repeat between two processes and from Explorer. Cancel a drag with Escape,
drop outside the target, and close/reload while a drag is active: none should
leave the button held or copy/delete the source file. The ticking Luau counter
should continue during an OLE drag. Check both window and transparent-panel
surfaces and monitors with negative origins/different DPI. These manual steps
are not executed by the language checker.

`native-drag-panel.plm` supplies the same Unicode payload from a transparent
panel, for a receiver window placed beneath it. Computer Use currently rejects
a drag whose endpoint belongs to another HWND; that cross-window check requires
manual input or another explicitly supported GUI test facility.

## Native recording (opt-in)

`native-recording.plm/.luau` provides a real-window recorder fixture with an
animated marker and Luau counter. `scripts/windows-recording-smoke.py --desktop`
explicitly enables local monitor/system-audio recording; it is not run in the
ordinary scene suite. Build `cargo build --release --example recording-inspect`
to decode the resulting MP4's video and audio using Windows Media Foundation.
The smoke supports normal stop, reload, process exit, timed exit and startup
cancellation. `--tone` plays a quiet test signal into the system output.

The ignored `native_recording_unicode_roundtrip` Rust test uses a synthetic
GPU image, requires D3D11/H.264/AAC support, and never captures the desktop. It
encodes and reopens a file in a temporary path containing spaces and Unicode.

Native notification checks (Windows desktop, explicit opt-in):

```powershell
cargo run --locked --example notifications-status
cargo test --locked live_notification_inventory -- --ignored --nocapture
cargo test --locked native_toast_roundtrip -- --ignored --nocapture
$env:PLEAMAR_NOTIFICATION_HOLD = '1'
cargo test --locked native_toast_roundtrip -- --ignored --nocapture
Remove-Item Env:PLEAMAR_NOTIFICATION_HOLD
```

The inventory prints counts/access only. The round trip creates a uniquely named
Start-menu shortcut and an owned Unicode toast, reads it via the native service,
dismisses only that toast, then removes its shortcut/history entry. The optional
hold leaves the test toast for two minutes to cover delayed sender metadata and
permit visual inspection. These tests do not request consent or alter privacy
settings. They fail when Windows denies access or notifications are unavailable;
CI only compiles them and executes the pure ID/metadata regression tests.

Native tray checks (Windows desktop):

```powershell
cargo test --locked native_tray_inventory -- --ignored --nocapture
cargo build --locked --example tray-fixture
target/debug/examples/tray-fixture.exe
target/release/pleamar.exe --scene tests/native-tray.plm --no-hud --stall 0
```

Run the scene in a second PowerShell session while the fixture is open. Its two
buttons operate only `tray-fixture`, never the user's applications. Verify the
fixture title/log changes on activation; the context menu's first item records
selection without affecting the system. Close both, then repeat with
`tray-fixture.exe --legacy` for HWND/UID icons. The default uses a GUID and
NOTIFYICON_VERSION_4. Close the fixture before rebuilding/testing: Windows locks
running executables. Its 15-minute timer and normal close remove its own icon.
The inventory is read-only and prints only count/timing; it does not open the
Explorer overflow panel or inspect notification contents.

Windows shader-runtime packaging (PowerShell 5.1 or newer):

```powershell
./scripts/test-windows-runtime.ps1 -Archive C:/Downloads/dxc_2026_09_29.zip
```

This checks the real pinned archive, compiler/validator/license copies, rejection
of a corrupt archive, and refusal to partially update a locked runtime. It uses
an owned temporary fixture and does not execute its placeholder binary. Marea's
`windows/test-runtime-files.ps1` separately checks package discovery, changed
DLLs, missing manifests and rejection of arbitrary manifest paths. Neither check
proves rendering: run the full native scene, all three appearances and shader
reload with the prepared runtime as well as the bare-executable fallback.

## Luau child-process lifetime

`cargo test --locked --lib lifecycle_tests` includes real subprocess checks for
reload after a helper closes stdout but stays alive, discarded exit callbacks,
and repeated completion with the original exit codes. The shutdown registry
must release completed helpers. The subprocess entry points are ignored by the
ordinary runner and invoked only by their parent tests; they do not open UI or
change system settings. The stdout-closed helper exits after four seconds even
if the old blocking wait returns, so that regression fails without hanging CI.
The two new checks failed against the previous wait/registry implementation and
passed with the fix on Windows. Their Linux execution remains a CI requirement.

## Windows media controls

The default unit run checks playback capability fallback and malformed arguments.
For a native desktop rehearsal, build the owned SMTC fixture, then launch it in
a separate PowerShell session:

```powershell
cargo build --locked --example media-fixture
target/debug/examples/media-fixture.exe
```

The fixture publishes two Unicode tracks and receives actual Windows transport
commands, without playing audio. Use its printed `org.pleamar.validation.media.*`
identifier for the opt-in test while it is the current session:

```powershell
$env:PLEAMAR_MEDIA_FIXTURE_PLAYER = 'org.pleamar.validation.media.REPLACE_WITH_PRINTED_PID'
cargo test --locked native_media_fixture_controls -- --ignored --nocapture
Remove-Item Env:PLEAMAR_MEDIA_FIXTURE_PLAYER
target/debug/examples/media-fixture.exe --read
```

The test refuses a different player identity before sending commands. It checks
pause/play, previous/next, Unicode metadata, disabled capabilities and stale
application refusal. Do not interact with another player during this test.
The independent `--read` command prints the current Windows session; it can
include the user's track metadata, so redact that output before sharing logs.

With Marea open, click pause, next, the disabled next button, resume and previous.
The fixture title/log must show exactly the accepted commands. Reload Luau and
repeat one click to check that handlers are not duplicated. Close the fixture
normally; Marea should adopt Windows' next current session or show no player.
The fixture also closes after fifteen minutes. This is real native transport
validation, not an audible playback test or proof of every third-party player.
CI builds the fixture but does not perform these interactive checks.

Marea's `windows/test-media-controls.py --luau-runner PATH` independently checks
duplicate requests, player changes, stale queries, queue failures, subscription
failure and timeouts using a mocked native API. It is included in
`windows/test-logic.py`; these isolated tests do not control the desktop.

## Long-running render and allocation diagnostics

`cargo test --locked --lib cadence_tests` checks that continuous animation
statistics use bounded storage, keep real late/blocked updates and preserve
the once-only slow-scene warning between reporting windows. `cycle` log lines
now cover at most 3,600 updates (600 with `--no-vsync`); a final shorter line
covers the remaining updates. To summarize a complete run, combine counts and
weighted means; per-window p99 values cannot be averaged into a whole-run p99.
Marea's measurement helper computes percentiles from the individual recorded
update timestamps, independently of these summaries.

For allocation attribution without changing the shipped allocator:

```powershell
cargo test --locked --example allocation-profile
cargo build --release --locked --example allocation-profile
target/release/examples/allocation-profile.exe --scene PATH_TO_SCENE --no-hud --stall 0
```

This runs the actual pleamar runtime and reports live requested Rust bytes,
blocks and peak bytes every five seconds. Counters add overhead; this executable
is a diagnostic, not a performance comparison with the normal binary. Sampling
concurrent counters is approximate. The report excludes allocator-reserved
pages, native Windows/COM, direct C++/driver and other non-Rust allocations.
mlua's VM allocator uses Rust allocation functions and is included. With
`PLEAMAR_TIMING=1`, active Luau timers also report that VM's bytes, timer count,
pending callbacks and children at most once every five seconds. This is a
breakdown of the total, not additional memory; it neither forces garbage
collection nor wakes an otherwise idle VM.
Unchanged Rust live bytes therefore do not prove stable process memory. The
normal `pleamar` executable contains neither this allocator nor its sampler.
See the [Rust allocator contract](https://doc.rust-lang.org/std/alloc/trait.GlobalAlloc.html).

For compiler lifetime regressions, run:

```powershell
python scripts/test-compile-memory.py --profiler target/release/examples/allocation-profile.exe
target/release/examples/allocation-profile.exe --compile-check PATH_TO_SCENE 50
```

This mode warms interned names before repeatedly loading and dropping the same
scene, without starting graphics, Luau or desktop services. The test covers
generated pages and declared services, including errors after expansion. It
checks live requested bytes against a fixed 4 KiB allowance for the entire run;
allocator caches and process working set are outside this measurement. CI runs
the same checks with the debug diagnostic on both platforms.

On Windows build 20348 or newer, set `$env:PLEAMAR_PROFILE_NATIVE_HEAP = '1'`
before launching this diagnostic to include the default process heap's allocated,
committed and reserved bytes using dynamically resolved `HeapSummary`.
Remove the variable afterwards with `Remove-Item Env:PLEAMAR_PROFILE_NATIVE_HEAP`.
Older systems report that the extra counter is unavailable.
This is only the default heap; other native heaps and direct virtual allocations
are not included. Reading its summary can pause heap users (about 40 ms on the
validation host), so do not use this executable to claim
production animation performance. A native test creates an owned private heap
and verifies that the summary follows an allocation and its release.

To attribute native reads without starting graphics or Luau, select one query:

```powershell
$env:PLEAMAR_PROFILE_SERVICE = 'audio.state'
$env:PLEAMAR_PROFILE_SECONDS = '30'
cargo test --locked --lib native_service_heap_profile -- --ignored --nocapture
Remove-Item Env:PLEAMAR_PROFILE_SERVICE, Env:PLEAMAR_PROFILE_SECONDS
```

The diagnostic accepts `audio.state`, `media.state`, `network.state`,
`network.wifi`, `bluetooth.state`, `brightness.state`, `window.state`,
`tray.list` and `apps.list`. It warms three reads, then samples the default
heap for 1–600 seconds at each service's polling interval. Run each query in
a fresh test process. It issues no system-control commands. A structured
unavailable result is still a successful query, so zero query errors does not
prove that hardware works. There is deliberately no allocation-plateau
assertion: these short measurements exclude the renderer and subscription
machinery, and native caches can change during a run. Older Windows versions
without `HeapSummary` print `NOT RUN`, which is not measurement evidence.

`cargo test --locked --lib platform::windows::input_tests` exercises the real
Win32 window procedure on owned hidden windows. It checks capture transfer,
multiple held buttons, cancellation/retry and incomplete UTF-16 input across
focus loss. It does not inject mouse/keyboard input or require a GPU. Normal
release must retain the final pointer position; cancellation must release held
buttons once without releasing the new owner's capture.

For the visible counterpart, run `pleamar --scene tests/native-pointer.plm`.
Drag the bar both ways, reload the scene and retry. Press/Release counts must
match and Held must return to zero. Click the input and type Unicode afterwards.
The bar changes only a scene fact, never the system volume or brightness.
Also repeat `tests/native-drag.plm`: OLE takes over the same native capture,
so text/file transfer, rejection and retry are relevant regression checks.

`cargo build --release --locked --example window-fixture` builds eight ordinary
Win32 windows for Marea's minimized-window shelf rehearsal, without eight GPU
runtimes. It starts them minimized, reports their actual `IsIconic`/foreground
state, and accepts commands only for its own windows. Run Marea's
`windows/test-window-desktop.py` with `--fixture` pointing to that executable;
the script prints when physical stone and restore-all clicks are needed.
Its copied Marea profile filters the native window catalog by the fixture's
class so restore-all cannot affect user applications. The shelf logic and
native restore calls remain unchanged. A 15-minute fixture timeout closes its
windows; the rehearsal also closes them on failure.

`cargo test --locked --test platform-callbacks` checks that an embedding backend
implementing the original `request_frame` contract still receives requests.
Backends without callbacks must explicitly opt out of callback pacing. The
Windows hidden-window suite verifies that the Win32 backend does so; neither
test starts a compositor or substitutes for native Linux validation.

The hidden Win32 suite also runs the production monitor reconciliation loop
against supplied output lists. It verifies one normal window per declared
surface across output/copy selection and renumbering, preserving its HWND,
position and current scale. Panels still get the requested per-output copies;
removed outputs retire them and returning outputs receive fresh IDs. These
tests use a hidden-window factory, not a GPU or actual display changes.
`scripts/windows-smoke.py --gui` additionally repeats the current physical
monitor in `--screen` and counts actual visible HWNDs: one decorated window
must remain one window. Real mixed-DPI migration and hotplug are separate
manual checks.

`windows-smoke.py` also verifies that an absent selected output keeps Luau and
IPC live with no HWNDs, and that both IPC quit and `--seconds` exit cleanly.
Those checks run without `--gui` and are included in the Windows CI CLI step;
they provide no graphical-validation claim.

For real HWND/DX12 lifecycle checks without changing Windows display settings:

```powershell
$artifacts = cargo test --locked --lib --no-run --message-format=json
$harness = $artifacts | ForEach-Object { $_ | ConvertFrom-Json } |
    Where-Object { $_.reason -eq 'compiler-artifact' -and $_.profile.test -and $_.target.name -eq 'pleamar' -and $_.executable } |
    Select-Object -First 1 -ExpandProperty executable
python scripts/windows-display-lifecycle.py --binary target/release/pleamar.exe --harness $harness --output .tools/display-rehearsal --interactive
```

Use a new output directory each time. The ignored Rust helper uses the real
native event loop, renderer and first physical output. Its process-local
observation alternates between that output and an empty list; this hook exists
only in the test helper, not in the installed executable. The script checks
initial waiting, two panels counting as one monitor, popup ownership, three
removal/reappearance cycles, Luau progress and destruction of all owned HWNDs
(including hidden ones) during absence. `--interactive` then waits for an actual
click on the restored main panel. Without it, there is no mouse validation.
This does not exercise physical hotplug, GPU removal, mixed-DPI migration or
driver recovery. The test helper closes after five minutes even if its driver
script stops; the script also cleans up on failure.

## Scroll viewport interaction

`cargo test --locked viewport_tests` checks that rows outside a layout's `view:`
cannot intercept a footer or contribute an off-viewport input region. The tests
cover scrolling, partial visibility, nested layouts, anchors, rounded corners
and rotation. They run on both platforms without a window or GPU. Native Marea
pointer validation is separately recorded in `docs/windows-validation.md`;
passing these assertions alone does not demonstrate a working Windows UI.

## Service listeners across reloads

`cargo test --locked platform::service_tests` checks replacement of a recreated
owner's subscription, immediate replay of its current snapshot, and independent
declarative/Luau subscriptions. The clock checks run on both platforms. Windows
also checks its actual application catalog: unlike Linux's one-shot reader, it
stays active and must release the previous listener when its owner returns.
The tests do not open a window or exercise native pointer input.

`cargo test --locked service_reload_tests` exercises scene-declared services
through real clock subscriptions. It covers added fields on a quiet source,
rebinding an alias, removing/restoring a declaration, permission changes,
queued stale snapshots, plugin delivery and dropping the scene. The six tests
run on both platforms; the native workers remain process-owned while retired
subscriptions stop delivering data.

For saved-file reloads through the Windows executable, with Luau and IPC:

```powershell
python scripts/windows-service-reload.py --binary target/release/pleamar.exe --output ".tools/service reload ñ"
```

Use a new output directory. It retains the scene, native log and a JSON report
even on failure. The test adds a service field, changes its source, revokes and
restores permission, removes/restores the declaration, and reloads Luau six
times. It selects an absent output and opens no window; this checks runtime
and watcher behavior, not graphics, physical input or rendering performance.
Windows CI runs it after the ordinary CLI smoke.

`cargo test --locked watch_reload_tests` checks Luau watcher ownership and
lifetime with actual clock/file services: private scene/plugin files, old queued
data, permission revocation/restoration, removed watchers, scene destruction and
late subscribers. It also covers a newer update arriving before a queued cached
replay. The fixtures clean up only their own files and empty directories.

```powershell
python scripts/windows-watch-reload.py --binary target/release/pleamar.exe --output ".tools/watch reload ñ"
```

This Windows CI rehearsal starts a real scene and plugin with isolated APPDATA.
It approves only its generated file-reading fixture, checks both owners' actual
file changes, revokes/restores scene permission, reloads Luau six times and
removes/recreates the plugin. Editing the plugin's code must invalidate its
approval; an explicit fixture reapproval must restore access on reload. Logs,
approvals and the JSON report remain in the new output directory. `--case
ownership` and `--case approval` isolate the two native negative controls.
No user data or approval registry is modified, and no display is selected.

`cargo test --locked async_tests` covers queued service commands/queries after
permission removal, a completed query whose reply is still pending, restoration
after a denied command and overlapping replies from plugins that occupied the
same reload position. The commands use actual files in unique fixture folders.

```powershell
python scripts/windows-callback-isolation.py --binary target/release/pleamar.exe --output ".tools/callback isolation ñ"
```

The Windows CI rehearsal adds a plugin with a slow native PowerShell helper,
then inserts another before it with a fast helper. Each must receive exactly
its own stdout once. Both helpers are bounded and use only literal fixture
output; their approvals and the scene live under the new output directory.
The report records whether the helpers overlapped. This checks actual process
and plugin lifetimes without selecting a display or driving hardware controls.

Windows unit tests under `windows_audio::watch` exercise COM callback bursts,
all endpoint-change callbacks and recovery deadlines during continuous level
updates. They invoke callback interfaces directly, without changing hardware.
The ignored `live_audio_subscription` test performs twelve actual registration,
snapshot and unregistration cycles; run it explicitly on a Windows desktop:

```powershell
cargo test --locked windows_audio::watch -- --include-ignored --nocapture
python scripts/windows-audio-watch.py --binary target/release/pleamar.exe --output ".tools/audio watch ñ"
```

The Python diagnostic selects no display, changes no volume/device settings,
observes process CPU for 30 seconds and checks five saved-file Luau reloads.
Use a new output directory; `--seconds` accepts 5..300. Its report distinguishes
unavailable service state and notification-registration fallback. This is not
a volume-change latency, physical hotplug, audio-service restart or sound-quality
test. Automatic CI includes the three callback/deadline unit tests; hardware
registration and the read-only observation are opt-in local checks.

`render::rule_tests` covers shared `on change`/`on still` history when the active
monitor copy changes, simultaneous copies, volume bursts, and independent
per-monitor expressions. The regression previously left Marea's volume meter
visible indefinitely when the old copy closed before the hide deadline.

For the native renderer, map both diagnostic copies to one chosen monitor:

```powershell
python scripts/windows-rule-handoff.py --binary target/release/pleamar.exe --screen '\\.\DISPLAY2'
```

This uses fake volume facts, no audio services, no physical input and no keyboard
focus request. It checks the hide deadline and return to the original copy.
The interactive diagnostic is separate from the unit tests run in CI.

Effect groups have CPU regressions for empty groups and 128 visible siblings.
The explicit GPU test checks pixels from all 128 groups against reference tiles,
including overlapping shapes at half opacity, and repeats close/reopen four
times. Each surface uses one compositing texture and releases it when closed.
It renders only an owned fixture and never captures the desktop:

```powershell
$env:PLEAMAR_EFFECT_IMAGE = 'effect-groups.png' # optional fixture export
cargo test --locked gpu::effect_tests:: -- --ignored --nocapture
```

The GPU test is opt-in because a real graphics adapter is required. CPU
regressions and `group-effects-many.plm` are part of the normal checks; the
fixture can also run natively on a chosen display with `--screen`.
