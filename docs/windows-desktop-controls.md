# Desktop controls follow-up — 2026-10-03

Developed with Codex. This is an incremental update of the existing Windows
port PR, not a declaration of complete Linux desktop parity.

## Behavior

Marea fixes stale notification row animations after dismissal/reordering and
stops treating the beginning of a click/drag as a resolve action. An OS rejection
restores the notification; successful removal refills the five visible slots.
Hat/headphones are mutually exclusive. Minimized application icons can fold
into the mascot with a persistent, reachable count. Search supports accent
folding, reordered tokens, initials and one-character mistakes; files cannot be
starved by application results. It does not read browser history/bookmarks.

The media footer controls identifiable Core Audio sessions belonging to the
current Windows media player. Stale-player commands fail. Unknown identities
show unavailable; no fallback changes master output volume. The new worker
coalesces pointer updates and reads back the applied value.

Closed sheets release temporary compositing layers immediately. Windows also
shrinks closed DirectComposition swapchains to a transparent pixel and restores
physical dimensions before reopening. Linux retains its surface sizing.

## Local verification

Windows 11 x64/MSVC, default Luau; RTX 5070, 2560 × 1440 and 1920 × 1080,
both at 125% scaling. Upstream checked again: pleamar `4163c91a`, Marea
`d3c5398c` (no newer upstream commits during this follow-up).

- Native release build with default features and `luau-test`/`media-fixture`.
- `cargo test --release --locked --lib`: 124 passed, 25 opt-in tests ignored.
- `python scripts/run-tests.py --binary target/release/pleamar.exe`: 227 checks.
- Marea `windows/test-logic.py`: generated PLM/Luau and 15 isolated suites pass,
  including notification slot reuse, failed dismissal/count restoration,
  cosmetics, coalesced sliders and search result selection/stale callbacks.
- `windows/test-search.py`: actual native Marea finds an isolated Unicode file
  with reversed words, omitted accents and a typo; stale results are discarded.
- `windows/test-desktop-native.py`: the generated row receives a click and
  drag through scripted renderer input, opens once, dismisses once, resets the
  slot and retains seven notices. No desktop pointer movement or OS dismissal.
- Owned native Marea instance: open leaves a row alive; four acknowledged
  dismissals refill slots and leave a count of four; head exclusivity and fold/
  unfold settle. The notification service is mocked in this instance: no user
  notifications were deleted by the rehearsal.
- `media-fixture --audio` plus ignored `native_media_fixture_volume`: actual
  silent Core Audio session passes 20/70/0/100% roundtrips, stale-player
  rejection, unchanged master output volume and restoration.
- `windows/test-media-volume-native.py`: press/drag/release on the generated
  volume track confirms 70% in an owned silent Core Audio session.
- Native two-monitor tide shader, ten close/reopen cycles and hot reload preserve 2048 × 1152 and
  1536 × 864 logical dimensions after the memory change.

A foreground game obscured the tool's attempted window capture. No focus or
mouse was taken from the game. The new visual layout has **not** received a
successful screenshot review; native rendering/logic checks are not a claim
that its visual appearance was inspected.

## Memory measurement

Same installed preview.4 scene, isolated preferences, Classic look, panel
closed; each run has 5 s initialization, 3 s settling and 30 s sampling.
The engine is the changing variable. No working-set trimming was performed.
A game and other normal applications remained running; this is not a controlled
frame-rate benchmark. The earlier attempted open-panel sample was discarded
because its state changed during collection.

| End of sample | Previous engine | Updated engine |
|---|---:|---:|
| Private committed memory | 390.05 MiB | 321.57 MiB |
| Resident working set | 250.30 MiB | 247.39 MiB |
| Update interval mean | 16.73 ms | 16.67 ms |
| Update interval p99 | 19.4 ms | 19.6 ms |

Private committed memory fell 68.48 MiB (17.6%). Resident RAM fell 2.91 MiB;
these are different measures and must not be reported as the same saving.
Update intervals are not measured physical display FPS. GPU driver caches and
other workloads can change absolute memory results. An earlier long-lived
installed instance used approximately 617 MiB resident / 433 MiB private, which
is not a comparable fresh-process baseline and is not used for this reduction.

Commands (run each variant sequentially with ordinary Marea stopped):

```powershell
python windows/measure-desktop.py --binary <engine.exe> --scene <same-marea-desktop.plm> --output <new-evidence-directory> --seconds 30 --repeats 1 --skins classic --states false
python windows/test-wallpaper-monitors.py --binary ../pleamar/target/release/pleamar.exe
```

Continuous integration and installer validation are reported on the exact
commits in the existing draft PRs. Local GUI/hardware checks remain distinct
from CI and from the limitations documented in the Windows desktop guide.
