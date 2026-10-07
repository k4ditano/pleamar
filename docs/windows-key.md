# Optional Windows-key layer

Windows scenes can opt into `hotkeys.windows` through `sys.call_async`.
Pass a map of chords to event names, or `false` to release the layer. Events
arrive through the existing `hotkeys` service (`event`, `sequence`).
`hotkeys.state` returns `available` and `windows_key`. The ordinary
`hotkeys.bind` API still uses RegisterHotKey for Ctrl/Alt/Shift combinations.

Supported chords are Win alone, or Win plus optional Ctrl/Alt/Shift and one
letter, digit, Space, Tab, Enter or Escape. The map is limited to 16 bindings.
The layer consumes physical Win gestures and keys pressed during them,
including unassigned combinations. Tapping Win fires on release; combinations
fire once on key-down. Already-held keys retain their releases, and injected
input is passed through. Applications must expose this as an explicit opt-in:
it replaces Start and ordinary Windows-key shortcuts while enabled.

The low-level keyboard hook has a dedicated message thread. Its callback only
updates bounded state and queues an action; it never runs Luau, storage or
subscribers. A session-local mutex prevents competing instances. A different
live scene cannot replace or disable the owner. The async service lifetime
releases the hook on scene retirement/reload (checked every 250 ms while
enabled), and process exit releases OS resources. No registry remapping,
injected input or persistent system setting is used.

## Validation and limits

Ordinary unit tests cover parsing, repeated key-downs, unknown combinations,
preexisting keys, both Windows keys and injected events. The opt-in native test
installs the actual hook on a newly created private Win32 desktop, checks
ownership conflicts, lease expiry, repeated enable/disable and thread exit:

```powershell
cargo test --locked --lib native_layer_private_desktop -- --ignored --nocapture
```

This test never switches the visible desktop or generates input. CI runs it on
Windows in addition to the existing Windows/Linux/default-Luau suites. It does
not prove physical shortcut dispatch, foreground activation, secure-desktop
behavior or behavior inside elevated applications/games. Those interactive
checks remain separate. Windows may silently remove a hook that stalls; state
reports the owned registration, not independent OS confirmation of delivery.

References: [LowLevelKeyboardProc](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc),
[SetWindowsHookEx](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowshookexw).

AI assistance: implemented and reviewed with Codex.
