# Native scene commands

This work integrates upstream `700bfb2` (0.2.27): named actions, wait/watch,
hello, language 0.3, explicit control roles, values and states, and Linux's
panel-cursor routing. The Windows x64/MSVC engine build, 213 unit tests,
250 language checks (35 documentation scenes), isolated Luau checks and
Marea profile/logic checks pass on the final `v5` sources. The native rerun
of the input-guard correction is pending. The matching
[Windows/Linux CI](https://github.com/SamuelHinestrosa/pleamar/actions/runs/37574606193)
and [preview.35 installer lifecycle](https://github.com/SamuelHinestrosa/marea-plm/actions/runs/37574647213)
passed. They do not replace the pending native input-guard rerun.

The Windows client supports `describe`, `describe json`, `press`, `hold`,
`drag`, `wheel`, `type`, `key`, `wait`, `watch` and `hello`, alongside the
existing scene commands. Actions go through the scene's own render/input
logic. They do not inject the system mouse or keyboard and do not implement
an independent desktop seat.

For a running scene named `notes`, from PowerShell:

```powershell
pleamar --say notes 'hello'
pleamar --say notes 'describe json'
pleamar --say notes 'press save'
pleamar --say notes 'type query España ñ 世界'
pleamar --say notes 'wait dirty == false 3s'
pleamar --say notes 'watch 10s'
```

Use the actual element names returned by `describe`. The matching Windows WM
also exposes `agent scenes`, `agent tree PID [json]`, `agent press PID NAME`,
`agent wait PID CONDITION`, `agent watch PID [SECONDS]` and `agent say PID
ORDER...`. `agent scenes` includes panels that the ordinary native window
catalog excludes. `scene:ENDPOINT` selects an endpoint explicitly when a
process has more than one scene. Discovery verifies the reported PID against
the connected Windows pipe's native server PID. An action reconnects and
checks that PID before sending its order, so an endpoint replaced by another
process cannot receive a stale action. Generic application click/type/open and the
Linux independent cursor remain unsupported and fail explicitly.

## Lifetime and transport

The scene namespace and pipe DACL remain scoped to the current Windows logon.
Eight command workers can run concurrently, with a separate listening instance.
An idle listener sleeps in the kernel. A wait or subscription therefore does
not serialize unrelated commands. Excess requests receive an explicit busy
response; `quit` remains available when all worker slots are occupied.

New clients opt into streaming with the `@stream-v1 ` prefix. Each response
is one JSON string followed by a newline; `null` ends the response. Clients
acknowledge each frame with `ack\n`. This preserves Unicode and embedded
newlines and prevents disconnect from discarding the last reply. Frames are
bounded to 64 KiB. A stalled write or acknowledgement has a two-second deadline.
Legacy clients retain the original one-reply protocol; they must upgrade to
use `watch`. Notification activation pipes retain their separate single-client
protocol and namespace.

`watch` and `wait` check connection liveness while waiting. A disconnected
client releases its renderer lease and wakes an idle scene to retire the
subscription. Reload retires pending conditions, watches and actions with an
explicit message: their stored indices belong to the old scene. Query the
new description before repeating an action. Invalid/non-finite drag and wheel
arguments are rejected before entering the input path.

Queued and active named actions also hold connection leases. Disconnect or a
command deadline cancels pending steps and clears a held virtual drag without
synthesizing a release action. Other connected clients can continue issuing
commands. This cannot undo an action that already ran before cancellation.

Named presses and wheel actions recheck the actual input point immediately
before the effect. A hover or animation may have covered it since the initial
description. A reachable corner elsewhere in the target does not authorize
pressing a different element at the old point. Typing and keys also recheck
that the focused field remains visible and available to agents. Hidden,
person-only, lock and capture-excluded surfaces cannot be named targets.

The pipe ACL isolates Windows logons, not applications in the same logon.
Agent metadata describes intended interaction; it does not turn the legacy
fact/text/event command protocol into an authorization boundary.

## Validation record

Local Windows x64/MSVC validation before the final input guard (`v3`) passed
212 engine unit tests, 20 WM unit tests, 249 language checks, 35 documentation
scenes, the isolated Luau runner, and Marea's generated-profile/logic suites.
Default Luau was enabled. The native fixtures used only their own
non-activating windows on non-primary `DISPLAY2`, at 125% scaling:

- Named press/type/key/drag/wheel/hold, Unicode text, Luau callbacks,
  concurrent wait/watch, hot reload and clean closure passed.
- A filename containing the hello protocol's delimiter words routed to the
  correct native PID. Disconnected queued/held actions produced no delayed
  effect or synthetic release.
- The current generated Marea control center exposed Spanish labels,
  slider values and checked toggles. Its volume drag reached an isolated
  Luau handler. This fixture denied all services and did not change device
  volume or test live hardware integrations.

Two further native regressions failed on `v3`: a person-only overlay opened
between pointing and pressing received the press, and an already-focused
field accepted a key after an overlay covered it. Their captures showed the
unexpected effects. The final input guard addresses both; its build passed,
but its native reruns remain required before claiming acceptance.

The local records are `agent-integration-build-20261006-v3.json`,
`agent-language-20261006-v3.json`, `marea-agent-profile-20261007-v3a.json`, and
the native `report.json` files under `agent-integration-native-20261006-v3`,
`marea-agent-metadata-20261007-v3`, `agent-moving-guard-20261007-v3`, and
`agent-field-guard-20261007-v3a`. Captured actions/reload/Marea controls and
both failed-regression images were inspected. These are local evidence,
not files needed by the installed application.

The checked-in `tests/agent-changing-target.plm` reproduces both cases. On a
test display, launch it with the native executable and `--screen` set to that
display. From a second PowerShell terminal:

```powershell
pleamar --say agent-changing-target 'type query Original'
pleamar --say agent-changing-target 'press target'
# Hover opens the person-only overlay. The press must be rejected.
pleamar --say agent-changing-target 'get forbidden' # false
pleamar --say agent-changing-target 'key !'
# The covered field must reject the key, preserving Original.
pleamar --say agent-changing-target 'get query'
pleamar --say agent-changing-target 'quit'
```

`scripts/windows-agent-guard-ci.py` exercises the same scene using the release
build on a disposable GitHub-hosted Windows desktop. It checks that hover can
expose a blocker before the press, rejects keys to the covered field, and
restores named input after the blocker is removed. It retains the commands,
trees and three screenshots and checks native blocker pixels and unchanged
foreground. This guarded CI fixture refuses local execution; it injects no
OS input. A workflow pass still needs screenshot inspection and does not
establish physical-input or installed-product acceptance.

Named gestures schedule each pause from the preceding delivered input. A slow
render frame may satisfy that pause; it must not restart the timer. This keeps
the requested hold/click intervals while avoiding an extra frame of delay for
every point of a drag. The correction is shared by both platforms; it leaves
the eight-second command deadline and target/cancellation checks unchanged.

The input-point and field-access unit regression passed in the 213-test `v5`
engine run, including upstream 0.2.27/WM 0.2.28. It needs its final native rerun.
At the latest attempt Windows reported only DISPLAY2,
marked primary; the non-primary-only harness refused to start a scene. No
primary-display substitute, physical input or installed-product acceptance
is claimed.

Final engine commands (default Luau enabled):

```powershell
cargo test --release --locked --features windows-notifications --lib -- --test-threads=2
cargo build --release --locked --features windows-notifications --bins --example luau-test
python scripts/run-tests.py --binary target/release/pleamar.exe
python scripts/test-luau-runner.py --runner target/release/examples/luau-test.exe
```

These passed on 2026-10-07. The engine executable SHA-256 is
`e7ac7ed2d07daff57da6c6ab144520016b7b179b2b0bddfb38bb6902b61aeaa7`.
Forty-two opt-in library tests were skipped in the local checkout; they are
not included in the 213 passed count. The complete Marea logic suite passed
with that engine and the paired Luau runner. The exact published revisions
also passed [Windows/Linux CI](https://github.com/SamuelHinestrosa/pleamar/actions/runs/37574606193),
[portable Linux regressions](https://github.com/SamuelHinestrosa/pleamar/actions/runs/37574606276)
and [Marea logic](https://github.com/SamuelHinestrosa/marea-plm/actions/runs/37574645270).
[Preview.35](https://github.com/SamuelHinestrosa/marea-plm/actions/runs/37574647213)
passed its silent installer lifecycle and packaged WM supervision. It was
downloaded and hash-verified, without installing it locally. These checks
do not establish physical input or complete desktop acceptance.

The matching WM CLI and console-free host also build with default Luau.
Its CLI unit suite passed 20 tests, with nine desktop helpers skipped. Its
executable SHA-256 is
`d6e046081738cb0080bc8e4760d2846ee3652d65c902a5fe39b16a9abb6c5cae`.

Remaining validation before declaring this integration accepted:

- Exercise named actions, concurrent wait/watch, Luau and reload through a
  real passive scene on the secondary monitor; inspect the captured result.
- Validate WM scene discovery and routing, including ambiguous identities.
- Repeat the relevant language/Luau, Windows/Linux CI and paired installer
  checks when subsequent source changes require them.

### Disposable CI desktop input

The native-build workflow also exercises the Windows desktop service against an
owned child application: left/right/middle clicks, Unicode text, Backspace,
Ctrl+A, vertical/horizontal scrolling, dragging, and returning from an owned
approval window. Each action requires a fresh capture and verifies the child's
reported state. The `native-desktop-input` artifact retains the captures and
`result.json`. This sends real OS input on a disposable GitHub-hosted runner; both
fixture entry points reject execution without that environment and its explicit
opt-in. It is not an installed-product, elevated-window, or independent-seat test.
