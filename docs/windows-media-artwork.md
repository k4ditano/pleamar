# Windows media artwork

The native `media` service and `media.state` query expose `art`, a local image
path, alongside the existing title/artist/player/capability fields. It comes
from the active GSMTC session's thumbnail. Browser media (including YouTube)
and Spotify use the same path when they publish thumbnails; there is no
provider-specific title search, browser-history access or network fallback.
An unavailable session or absent/invalid thumbnail returns an empty string.

Reads execute on the service worker with the existing bounded WinRT waits.
Encoded data is limited to 4 MiB; image decoding limits dimensions to 4096 and
allocation to 64 MiB. Cached images fit within 192×192 while preserving aspect
ratio. The cache retains 32 content-addressed PNGs in
`%LOCALAPPDATA%/pleamar/media-artwork`. Unchanged metadata is rechecked at most
every 15 seconds and existing images are not decoded again. Player or track
metadata changes invalidate that interval immediately.

Validation on Windows x64 MSVC with default Luau, 2026-10-04:

- `cargo test --lib`: 131 passed, 27 explicitly ignored hardware/desktop tests.
- `python scripts/run-tests.py --binary target/release/pleamar.exe`: 227 passed.
- Image regression: aspect ratio, malformed input and byte-size limit.
- The owned `media-fixture --art --screen "\\.\DISPLAY2"` publishes two different
  thumbnails over real SMTC. Marea's native profile test verifies reception,
  rendering startup and replacement of the image after a next-track command,
  with no physical mouse/keyboard input or foreground change.

Build the fixture with `cargo build --release --example media-fixture`. The
paired Marea test is `windows/test-personalization-native.py`. The fixture is
not a Spotify/YouTube application compatibility test. Actual provider versions,
long-running playback and player-specific thumbnail availability remain manual
checks; absence of a thumbnail does not disable playback controls.
