# Native integration of pleamar 0.2.16

This update merges upstream `3d57957` into the existing Windows branch. Luau
remains enabled. The shared compiler includes component-local measurements in
`size:` and list `.content` in rules, which the new Marea chat needs. A regression
checks two independent measured rows, their hit regions and the list's total
height after both measurements change. The previous binary rejects that scene.

The new `spawn(..., {stdin = "open"})` and `write` API keeps the port's process
ownership, permission checks, scene-relative working directory and globally
unique callback IDs. Initial text is sent while stdout and stderr drain.
Persistent writes preserve byte order and report successful pipe delivery;
they never hold the shared logic-state lock. Inputs are bounded to 8 MiB and
two seconds per write. On failure the helper is stopped, preventing ambiguous
retries after a partial protocol message. Reload, kill and shutdown close pending
input. `run(..., {errors = true})` now appends stderr after stdout, draining both
concurrently so either pipe can exceed its buffer without deadlocking.

On Windows the worker uses a nonblocking byte-pipe handle; Microsoft documents
both anonymous-handle support in
[SetNamedPipeHandleState](https://learn.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-setnamedpipehandlestate)
and [partial writes in nonblocking byte mode](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-type-read-and-wait-modes).
The logic receives an acknowledgement after delivery, not just queue acceptance.
Unix uses `O_NONBLOCK` behind the same platform boundary.

## Executed locally, October 5, 2026

- `cargo test --locked`: 145 unit tests and one integration passed; 30 explicit
  opt-in helpers were skipped. The persistent-input tests start real processes:
  Unicode and ordering, simultaneous output/error pressure, initial input before
  EOF, denied foreign IDs, permission revocation, stalled readers, and retirement.
- `cargo build --release --locked --bin pleamar --example luau-test`: native x64
  MSVC build passed with default features, including Luau.
- `python scripts/run-tests.py --binary target/release/pleamar.exe`: 232 checks
  passed, including 33 documentation scenes.
- The updated Marea profile compiles and its eighteen existing isolated logic
  suites pass. This does not test the new AI worker or a model conversation.

Marea's new worker isolation, sign-in, desktop actions and installer packaging
are still being integrated. The currently installed Marea package remains
0.2.15-preview.13. There is no claim of complete desktop parity or new Linux
desktop validation. CI results apply only to the exact revision they test.

Developed with Codex.
