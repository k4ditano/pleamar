# Native Windows port: validation and remaining work

This draft is based on pleamar `8bb920f0` (0.2.6). The accompanying Marea
profile is based on `a5891891`, including the newly published Deriva source.
It is an initial native Windows port, not a claim of complete desktop parity.

## Original PR review

[PR #1](https://github.com/k4ditano/pleamar/pull/1), head
`3b6812e17a976a8f90b55fa32dac38e1663f39c3`, supplied a useful Win32,
DirectComposition, monitor-placement and AppBar starting point. Its original
validation used a cross build without Luau and limited runtime tests. Those
results do not validate this implementation. Popups, desktop services, Unix
assumptions, lifecycle and input handling required substantial additional work.
The last inspection returned no comments, reviews or checks and reported it
unmergeable against the then-current base.

## Executed locally on October 1, 2026

Windows 11 x64, MSVC, Rust 1.98.1; default Luau enabled. The latest validation
does not operate the desktop, inject input or claim current graphical success.

| Command/check | Result |
|---|---|
| `cargo build --release --locked --bin pleamar --example luau-test` | Passed natively, default Luau; six warnings |
| `cargo test --locked` | 119 unit tests and one integration passed; 24 opt-in helpers ignored |
| `python scripts/run-tests.py --binary target/release/pleamar.exe` | 225 checks passed, including 31 documentation scenes |
| `python scripts/windows-smoke.py --binary target/release/pleamar.exe` | CLI, Unicode PowerShell autostart, absent-output Luau/IPC, report discovery/CPU/memory/redaction, timed and requested shutdown passed; no GUI switch |
| Marea `python windows/test-logic.py --binary ../pleamar/target/release/pleamar.exe --luau-runner ../pleamar/target/release/examples/luau-test.exe` | Generated profile and thirteen isolated logic suites passed; 19 translation notices and an existing loose-clip note remain |
| Marea `cargo test --locked --manifest-path deriva/Cargo.toml` | 87 tests passed using real bundled SQLite |
| Marea `cargo build --release --locked --manifest-path deriva/Cargo.toml` | Native worker built; two unused-item warnings remain. Disabling unused C++ tokenizer training removed the mixed-CRT linker warning |
| Marea `python windows/test-deriva-native.py --worker deriva/target/release/deriva-worker.exe` | Real persistence, Unicode paths, 24 concurrent writers, duplicates, FTS, blob bytes, integrity, backup, export/import passed on isolated libraries |
| Marea `python windows/test-deriva-runtime.py --binary ../pleamar/target/release/pleamar.exe --worker deriva/target/release/deriva-worker.exe` | Actual default-Luau adapter launched the worker and saved, listed and searched real SQLite data; absent output, no graphical claim |

The upstream refresh exposed and corrected Windows-only diagnostic assumptions,
Deriva's Unix paths/RNG/cleanup and canonical-path self-ingestion guard, and a
Luau rollback regression where a failed candidate replaced the retained VM's
native watcher. A regression test now verifies future events after restoration.
An older test was updated to distinguish failed initial load from upstream's
new retain-previous-VM contract; the hidden AppBar test now checks preservation
of actual z-order rather than assuming a retired window accepts placement.

The first Windows CI run on this integration exposed a saved-scene reload race:
logic could replay a quiet service's cached snapshot before the renderer knew
the newly added field. The watcher now queues the scene definition before
notifying logic. A deterministic synchronous-replay regression test passes;
temporarily restoring the old order makes that test fail. The native saved-file
test retains its original deadlines, and CI preserves reload logs on failure.

Live Open-Meteo geocoding and forecast requests also passed through the Windows
inbox `curl.exe`, using a fixed public example city and checking the response
contract. This does not validate the calendar layout or user interaction.

The text-atlas fix reproduces exhaustion and verifies glyph recovery, live-image
churn, stale-generation rejection and oversized-working-set throttling. It
retains a single 2048-square RGBA atlas (16 MiB); a current working set larger
than that remains a declared limitation. A real visual confirmation of the
reported missing letters on this revision remains pending.

## Historical desktop and CI evidence

Earlier Windows sessions exercised native DX12 presentation, pointer/keyboard,
Unicode clipboard, popups, click-through, saved-file reload and Marea pages.
That evidence applies to those earlier builds, not automatically to 0.2.6.
An earlier long performance sample was invalidated when its panel state
changed; short CPU observations are not an interaction-latency benchmark.

The previous source snapshots passed both systems:

- pleamar `530b8b9c968610c7572ea7965b2aacca3288dfeb`:
  [Windows and Linux CI](https://github.com/SamuelHinestrosa/pleamar/actions/runs/36761899496).
- Marea `b78a47b7e65f923fd1b6d1cff4391f8afc4b9919`:
  [Windows and Linux CI](https://github.com/SamuelHinestrosa/marea-plm/actions/runs/36761953663).

These runs predate the latest upstream integration and native Deriva work.
Current-source CI is recorded separately in the draft PR, with exact commits.
CI does not validate an interactive Windows or Linux desktop.

## Remaining validation and limits

- Current-profile visual review; sustained CPU/GPU/memory and interaction
  latency; mixed DPI, physical monitor changes, IME, accessibility and resume.
- Actual Explorer restart, full-screen/autohide interaction and work-area
  restoration on the current build. Hidden HWND dispatch tests pass.
- Physical Wi-Fi join/password/persistence; no WLAN adapter on the host.
- Bluetooth new-device pairing/PIN/cancellation; generic non-audio profile
  connection control remains unavailable.
- Internal-panel WMI brightness on a laptop. DDC/CI worked on a supported
  monitor earlier, but later sessions reported no physical monitor interface.
- New Core Audio watcher validation through actual device/service changes.
- HDR/4K/long recording, protected content, multi-monitor screenshot coverage,
  protected tray icons and complete cross-application drag/drop.
- Deriva interactive drop/open and semantic inference with real optional model
  weights. Its Unix socket server is explicitly unavailable; Marea uses the CLI.
- The latest weather/calendar layout and interaction.
- Live authenticated Claude quotas; cache/error paths are covered.
- Linux desktop execution; automated checks do not establish desktop parity.

Compositor-specific rain/snow/window effects require pleamar-wm and are not
implemented on Windows. Secure locking uses Windows rather than a custom
password screen. Consult [Windows APIs and capabilities](windows.md) and Marea's
desktop guide for other documented restrictions. No unsupported operation is
presented as successfully implemented.

Developed with Codex. Human review and the outstanding native visual/hardware
checks are required before marking this port ready to merge.
