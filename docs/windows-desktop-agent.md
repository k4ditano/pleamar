# Native desktop tools (work in progress)

The Windows `desktop` service supplies Marea's desktop agent with native
window identities, per-window WGC pictures, input, focus and monitor movement.
It requires `desktop.*` permission. Linux's compositor path is unchanged.
This branch is not a declaration of complete Windows/Linux agent parity.

Read with `sys.ask_async` on the `desktop` service worker:

| Query | Arguments | Result |
| --- | --- | --- |
| `desktop.windows` | `{}` | `{epoch, input="foreground", windows, monitors}` |
| `desktop.look` | `{id}` | `{width, height, data, method}`; `data` is a base64 PNG |

Window ids are opaque decimal strings, not HWNDs or process ids. Each window
reports its separate OS process id, program, title, physical box, monitor name,
focus, minimized state and known owner (`dialogof`). Monitor numbers refer to
the last returned list, sorted by device name. A selected monitor keeps its
device identity if another output disconnects; it is not reinterpreted as a
different output at the same list index. Disconnected or geometrically changed
destinations require a fresh catalog.
The catalog lives with the service worker; refresh it at the start of a turn.

Each screenshot closes its WGC session, frame pool and GPU readback resources.
The loaded Windows capture module is pinned once until process exit because
Windows 11 can otherwise unload it before internal callbacks finish when the
last capture worker exits. This retains the library code, not screenshots,
COM apartments or capture devices. See Microsoft's
[GetModuleHandleExW pin semantics](https://learn.microsoft.com/en-us/windows/win32/api/libloaderapi/nf-libloaderapi-getmodulehandleexw).


Commands use `sys.call_async` and report OS errors in their callbacks. Their
first two arguments are the catalog epoch and window id:

| Command | Remaining arguments |
| --- | --- |
| `desktop.click` | `x, y, "left"/"right"/"middle", count` (1–3) |
| `desktop.type` | text (up to 4000 Unicode characters; no NUL) |
| `desktop.type_secret` | a name saved through the [scene credential service](windows-credentials.md); no value is returned to Luau |
| `desktop.key` | enter, tab, escape, backspace, space, arrows, delete, home, end, pageup, pagedown, f1–f12 |
| `desktop.hotkey` | e.g. `ctrl+shift+t`; ctrl, alt and shift modifiers |
| `desktop.scroll` | `x, y, direction, steps` (1–30) |
| `desktop.drag` | `x1, y1, x2, y2` within the picture |
| `desktop.focus` | none; requests restore/foreground focus |
| `desktop.send` | monitor number |

Monitor transfer preserves the outer window's logical size and relative work-area
position across DPI, bounded to the destination work area and capture limits.
It uses the native outer rectangle, including resize borders; a capture's DWM
visible frame is not a placement rectangle. Sending to the current monitor is
a no-op. A transfer requests no activation or Z-order change, checks the actual
result, and invalidates its previous picture. A constrained application can
reject the requested size; errors then report that its position may already
have changed. Maximized windows must be restored before cross-monitor transfer.
Real mixed-DPI transfer acceptance remains pending; geometry tests and a
single-monitor CI regression are not that acceptance.

`sys.call("desktop.cancel")` synchronously increments a process-wide epoch.
It performs no UI/COM work and invalidates actions already queued on service
workers. An OS input batch already inserted cannot be recalled. Afterwards,
request a new window catalog. `desktop.done`, with no arguments, runs on the
service worker to discard its catalog, pictures and window lifetime hook.
Marea invokes these on stop/new chat, worker death and normal completion.

Coordinates are physical pixels in the latest picture, including the frame.
Input rejects a missing/stale picture, changed geometry or modal target,
closed window, covered point, invalid coordinates, held modifier/button/Escape,
cursor confinement outside the gesture or mouse capture held by another window.
Pointer actions first insert only movement and check the actual cursor position;
buttons and wheel events are withheld if it did not arrive. The backend never
releases another application's confinement. These checks reduce shared-input
races but cannot make the user's mouse independent.
Every inserted gesture invalidates its picture: look again to verify the
application's actual result. A successful SendInput call only acknowledges
event insertion; it does not prove the application's task succeeded.

Windows uses the user's shared foreground input queue. There is no independent
compositor input seat, separate mint pointer, monitor glow or compositor stop
pill in this implementation. Marea's stop button remains available. Focus
requests can be denied by Windows; protected/elevated applications can reject
input. If approving a card gave Marea focus, an approved input action requests
focus back for its target. It refuses to steal focus from a different application
the user selected. The backend does not attach to foreign input queues, raise privileges
or substitute unacknowledged PostMessage events for real input.
See Microsoft's [SendInput contract](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput).

Approval focus return now waits up to one second for Windows to finish its
[asynchronous cross-queue activation](https://devblogs.microsoft.com/oldnewthing/20161118-00/?p=94745).
It requests activation only once, observes the foreground window, and aborts
on another window taking focus, cancellation, target destruction or held input.
No keystrokes are inserted until the target is foreground. The disposable CI
fixture reached Unicode selection/replacement and then exposed the former
immediate focus check; the native rerun of this correction is still pending.

Captures use [CreateForWindow](https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow),
not a desktop crop. An occluding application is not copied into the picture.
Known modal dialogs route through their disabled owner; a new/ambiguous dialog
requires a catalog refresh. Separate popup menus, protected content and HDR
fidelity still need explicit validation. Capture is bounded to 8192 pixels per
axis, 16 megapixels and 5 MiB of encoded PNG, with a 3-second frame deadline.
Images travel through the existing pipe without granting the agent filesystem
access outside its sandbox. GPU frames/pools/sessions and readback buffers are
released after each picture.

When WGC rejects an HWND with `E_INVALIDARG`, the backend can request its
GDI content using [`PrintWindow`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-printwindow).
Windows arranges rendering into the helper's DC across processes; sending the
raw `WM_PRINT` message did not paint the native dialog in the regression test.
This is window-only: no desktop rectangle, owner substitution or style change. A disposable child owns the bitmap/DC and is terminated
after three seconds or cancellation. The target must explicitly allow capture,
keep its identity/geometry and return a successful print. Untouched destination
pixels, protection and timeout produce errors and revoke the previous input
permit. PrintWindow can supply cached content; the destination marker does not
prove that every application pixel was repainted. GPU-only applications can
return incomplete or blank content through this GDI route.
`method` distinguishes `windows-graphics-capture` from `window-print`.
Companions using the library dispatch `windows_desktop::capture_helper` before
their ordinary CLI options. The native modal capture and button click passed in
[run 37638516551](https://github.com/SamuelHinestrosa/pleamar-wm/actions/runs/37638516551),
with actual client pixels checked. The following negative test was invalid: it
ignored WM_PRINT, yet Windows still returned the complete client image. The
replacement stalls the actual paint path. Timeout/protection and tool-window
acceptance remain pending until that revised end-to-end test passes.

The initial real-window regression exposed a second-capture access violation
in a generated WinRT static factory cache after its apartment was retired.
`windows_capture_winrt` obtains scoped factories instead; both agent captures
and the existing video recorder use this helper. On 2026-10-05 the repeated
native capture regression passed on DISPLAY2: 404×292 initial, 544×352 after
resize, 284×232 modal dialog, and a new identity after close/reopen. No physical
input was sent, and foreground focus/pointer position remained unchanged.

The real generated Marea desktop adapter also ran through a native Luau scene:
catalog lookup, a 13,140-character PNG response, cancellation and clean exit.
The scene rendered on DISPLAY2 and advanced 33 timer ticks during the run.
This is a component integration test, not a model conversation or a full Marea
UI acceptance test. Separately, 148 ordinary Rust tests and twenty isolated
Marea logic suites passed; six Node tests passed. Subsequent input changes
still require positive native input validation.

## Validation commands

```powershell
cargo test --release --locked --lib platform::windows_desktop
cargo test --release --locked --lib platform::windows_desktop::native_tests::native_catalog_capture_and_lifetimes -- --ignored --exact --nocapture
```

The second command requires active **non-primary `\\.\DISPLAY2`**. It creates
only a child fixture's windows there, captures known pixels, resizes/moves the
fixture within that monitor, follows its modal dialog, rejects invalid/stale
input and checks closed/reopened identities. It sends no physical mouse or
keyboard input. The emitted evidence directory contains PNGs and `result.json`.
This test does not prove positive input, arbitrary-app compatibility or a real
AI conversation; those validations and background-input parity remain open.

Implemented with Codex; validation scope must be kept separate from full parity.
