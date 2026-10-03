# Windows x64 (MSVC)

The native backend uses Win32 windows and DX12/DirectComposition. Luau remains
enabled by default and is built from source. WSL, Wayland and Unix shell tools
are not runtime requirements. Native interactive scenes have been validated;
the limitations below still prevent full desktop-shell parity.

Native screenshots use `sys.ask_async("screenshot.freeze", { scope }, callback)`
followed by `screenshot.finish` with the returned numeric identifier, on the
same service worker. Scopes are `region`, `display` (pointer's monitor) and
`active_window` (visible DWM frame bounds). Declare `screenshot.*` permission.
`finish` returns `{path, width, height, clipboard, clipboard_error}` or
`{cancelled = true}`. PNG files are created without overwriting existing photos
in the Pictures known folder's `Marea` directory. `screenshot.cancel` discards
an unused identifier. A worker retains at most one frozen image; a new freeze
replaces it and worker shutdown drops it. Finishing a frame older than one
minute fails. Reload/exit cancels an active selector through its worker lifetime.
The selector has a two-minute limit and supports Escape, right-click and focus
loss cancellation. Capture is bounded to 256 MiB and 16,384 pixels per axis.
This SDR desktop crop includes occluding windows, excludes the pointer, and
does not promise HDR fidelity or capture of protected/secure content.
Physical bounds use [DWM frame coordinates](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowrect)
and a per-monitor-DPI-aware worker; [GDI BitBlt](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/nf-wingdi-bitblt)
provides the pixels before region selection begins.

## Requirements and PowerShell build

- Windows 10/11 x64 with a desktop session and a working DX12 graphics driver.
- Rust stable, using `x86_64-pc-windows-msvc` (tested with Rust 1.98.1).
- Visual Studio 2022 or Build Tools, with **Desktop development with C++**, an
  MSVC x64 compiler and Windows SDK. Luau needs the C++ compiler.
- Python 3 for the repository test scripts. Git is needed to obtain the source.

A prebuilt executable also needs the x64 Microsoft C++ v14 runtime. The current
MSVC build imports `MSVCP140.dll`, `VCRUNTIME140.dll` and `VCRUNTIME140_1.dll`;
copying only `pleamar.exe` to a machine without them is insufficient. Install
Microsoft's [supported x64 Visual C++ Redistributable](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)
at least as recent as the compiler used for the build. Build Tools usually
supply this on a development machine; the installer does not download it.

The executable imports Media Foundation for native recording. Windows 10/11 N
requires Microsoft's [Media Feature Pack](https://support.microsoft.com/en-us/windows/experience/platform-variants/media-feature-pack-for-windows-n).
The validation host already has the runtime/media components; this is not a
test of a fresh Windows installation lacking them.

Run these commands from a checkout containing this Windows port. This work is
published for review in a fork; it has not been merged upstream. Cloning
upstream `main` alone does not obtain these changes. Use the review branch linked
from the Windows pull request.

```powershell
rustup default stable-x86_64-pc-windows-msvc
cargo build --release --locked
# Recommended: app-local DXC compiler, downloaded from a pinned Microsoft release.
.\scripts\prepare-windows-runtime.ps1
cargo test --locked
python scripts/run-tests.py --binary target/release/pleamar.exe
.\target\release\pleamar.exe --scene examples/window.plm --no-hud
.\target\release\pleamar.exe --scene tests/native-interaction.plm --no-hud --stall 0
```

Quote paths containing spaces. Scene paths, imports, companion `.luau` files
and LSP file URIs support Unicode. Scenes and Luau reload when saved. A broken
scene leaves the last good scene on screen with an error banner. Stop a running
Windows executable before replacing/rebuilding it: Windows locks executable
files, so automatic replacement/relaunch of the binary is not offered.

`cargo check --no-default-features` is only an additional diagnostic. It does
not validate the runtime, services or Luau.

## Install and run

```powershell
.\install.ps1
& "$env:LOCALAPPDATA\pleamar\bin\pleamar.exe" --scene "$PWD\examples\window.plm"
# Or choose a destination:
.\install.ps1 -Prefix 'C:\Tools\pleamar'
# Build/install without downloading DXC (the system FXC fallback is slower):
.\install.ps1 -SkipDxc
```

The installer builds this repository only. It does not install pleamar-wm or
Marea, change execution policy, or modify PATH. Add the displayed `bin` folder
to your user PATH if desired. Re-run the installer to update, after closing
pleamar. Remove that installation folder to uninstall; configuration is separate.

The Windows installer also prepares an app-local shader compiler from Microsoft's
[DXC 1.9.2609 release](https://github.com/microsoft/DirectXShaderCompiler/releases/tag/v1.9.2609).
It checks the pinned archive SHA-256 before extracting the x64 compiler,
validator and license notices beside `pleamar.exe`. `dxc-runtime.json` records
their individual hashes and provenance. The compiler is loaded by wgpu's normal
DX12 selection; it is not installed into Windows or added to PATH. `-SkipDxc`
skips this preparation step, and a bare executable remains usable with the
system compiler. A package that already contains DXC retains it on such an update.

For an offline build, pass a previously downloaded, matching official archive:

```powershell
.\scripts\prepare-windows-runtime.ps1 -Archive 'C:\Downloads\dxc_2026_09_29.zip'
# For a custom output directory:
.\scripts\prepare-windows-runtime.ps1 -BinaryDirectory 'C:\Tools\pleamar\bin' -Archive 'C:\Downloads\dxc_2026_09_29.zip'
```

Keep the DLLs, `dxc-runtime.json` and `licenses/dxc` with a redistributed runtime.
The preparer refuses a running target or locked output files, and a rejected
archive leaves the previous package intact. Changing the compiler does not
disable shader validation or remove scene effects. Linux dependencies and its
shader-compiler path are unchanged.

Shell configuration is in `%APPDATA%\pleamar`, overridden by `PLEAMAR_CONFIG`.
Scene storage is in `%APPDATA%\pleamar\<scene>`; plugin approvals also use the
platform configuration directory. The `autostart` file contains one PowerShell
command per line on Windows. Comments beginning with `#` and `wm:` lines are
skipped. `pleamar --autostart` executes it and returns; registering that command
with Windows Startup is a user choice, not performed by the installer.

```powershell
.\target\release\pleamar.exe --say native-interaction 'get ticks'
.\target\release\pleamar.exe --say native-interaction 'emit clicked'
.\target\release\pleamar.exe --say native-interaction quit
```

Commands use local named pipes; a second scene cannot take over an existing
scene name. With multiple running scenes, specify the file stem. Use
`PLEAMAR_SOCKET_DIR` as an isolated namespace for tests (it is a namespace salt
on Windows, not a directory of Unix sockets). Sender and receiver must run as
the same user with access to the pipe. `--screen '\\.\DISPLAY1'` selects a
monitor; the runtime logs available names, scale and refresh rate as sheets open.
If the selected output is absent at startup, the process keeps Luau and IPC
running and waits for it to appear. `--say ... quit` and `--seconds` still work
while no window exists. An available output whose native window creation fails
is reported as an error rather than being mistaken for an unplugged monitor.

## Capability matrix

| Capability | Windows status |
| --- | --- |
| Normal windows, resize, DX12 rendering, alpha | Tested on RTX 5070; maximize/restore and transparent panels observed |
| Keyboard, mouse, wheel, Unicode paste, Luau timers/events | Tested in a visible native window |
| Fonts and image/text renderer | 175 Windows system fonts detected; Latin, CJK and emoji observed |
| Scene and Luau hot reload; invalid scene fallback | Tested on paths containing spaces and Unicode; failed Lua initialization releases partial callbacks and recovers on reload |
| Clock, scene files, environment, clipboard | Native/shared implementations; clipboard writes report errors |
| Battery and window services | Native Windows APIs; active window/monitor plus a filtered top-level window catalog. Eight owned windows, Unicode rename/removal, a stone click and restoring seven windows beyond the six stones passed. Foreground-policy refusal, hung applications and live HWND reuse remain unverified. Unavailable battery fields are omitted |
| `--say` and process lifetime | Named pipes and a Job Object; forced parent termination tested |
| Anchored panels, AppBar, dynamic anchor/level/reservation | Panels observed; conditional reservation, toggling and restoration on timed exit measured |
| Multiple monitors and per-monitor DPI | Two outputs at 125% previously enumerated; native repeated-output and hidden-HWND topology tests preserve one normal window per scene surface and separate panel copies. Placement tests include negative coordinates and full height. Actual mixed-DPI migration and hotplug remain unverified |
| Monitor hot plug and DPI changes while running | Reconciliation implemented; physical unplug/mixed-DPI validation pending |
| Popups | Native owned HWNDs; real opening click, option activation and dismissal tested |
| Input regions and click-through | GPU visual window plus a region-clipped input HWND; tested against a separate process, including region removal |
| Popup placement | Uses the surface that last received input, falling back to a live surface; edge constraint and parent-move tracking remain limited |
| Global cursor outside surfaces | GetCursorPos with monitor-relative DPI conversion; no global mouse hook |
| Audio | Native volume/mute and output/input selection; all three default roles changed and restored on real hardware |
| Network | WinRT connection status; native WLAN scan, radio, saved-profile connection/disconnection and new open/WPA2-Personal profiles; no Wi-Fi hardware on the validation host |
| Bluetooth | WinRT radio, classic and LE catalogs, explicit discovery and pairing, plus audio-driver connection control. Native LE watching/discovery and Marea pagination passed. New-device pairing/PIN/cancellation still need an identified test device; generic non-audio connection control is unavailable |
| Brightness | DDC/CI change/readback/restore passed earlier on a ViewSonic. The current display exposes no physical monitor interface and reports unavailable with both old and new binaries; this is not a current hardware pass. WMI internal-panel backend is implemented; laptop validation is pending |
| Media | Native Windows media sessions, Unicode metadata and asynchronous Marea controls. Buttons follow the player's capabilities, including paused sessions; a changed application identity rejects stale commands. Requires a participating player; artwork remains Marea's original gradient |
| Applications/files | AppsFolder catalog and launch, lazy Shell icons, asynchronous bounded filename search, native shell open |
| Global hotkeys | RegisterHotKey, bounded registrations, reported conflicts; Marea search tested from another process |
| Wallpapers | IDesktopWallpaper state/catalog/change with independent stable confirmation and rollback. Actual Marea card clicks, scrolling, persistence and reload cancellation/retry passed. Full unobstructed tide visuals, physical mixed-monitor coverage and slideshow management remain unverified |
| Desktop glass | Cropped native GDI/DWM capture feeds the GPU lens/blur pipeline; Classic avoids capture; SDR, protected content/HDR fidelity unverified |
| Screenshots | Native frozen-desktop region, monitor and visible-window capture to PNG/image clipboard. Actual region drag and visible-window captures passed with pixel-for-pixel clipboard readback; Escape/right-click/reload/exit cancellation passed. Mixed-DPI and multi-monitor regions remain unverified |
| Drag and drop | Native OLE Unicode text and file lists, copy-only; real drags passed within a test HWND. Transfers between applications remain unverified because the desktop automation tool rejects a different destination HWND |
| Recording | Native WGC capture, GPU NV12 conversion and Media Foundation H.264/AAC MP4 at 60 fps with WASAPI system-output audio. Real 1080p clips decoded; stop, Lua reload and process exit finalized correctly. Saved-card Watch/Folder/Copy path passed through physical clicks. HDR, 4K, long recordings and device changes need further validation |
| Notifications | Native live toast collection and individual dismissal, with access status and explicit consent request. Classic Win32 toasts may omit sender metadata; those retain text but offer no app launch. Original toast actions, forced retention and OS-wide do-not-disturb are unavailable |
| Tray | Live Explorer catalog, activation and application-owned context menus; GUID and HWND/UID fixtures passed through real input. Incomplete enumeration of protected/system icons; Explorer-version and animated-icon coverage remain limited |
| Workspaces | Unavailable; no fabricated service data or success |
| Session | Native lock, suspend, logout, reboot and shutdown; no forced application exit; power actions intentionally not executed during automated tests |
| Custom lock surfaces / authentication | Rejected/unavailable; a drawn scene is never presented as a secure Windows lock screen |
| Other applications embedded with `windows` | Requires the separate Linux compositor project; outside this port |
| Unix `kill(id, "int"/"term")` | Explicit error; `kill(id)` terminates the child instead |
| Binary replacement and automatic agent-skill discovery | Stop the binary before replacing it; agent skill discovery still follows the existing HOME-based behavior |

## Validation

The reproducible command tests are:

```powershell
cargo test --locked
cargo build --locked --example luau-test
python scripts/test-luau-runner.py --runner target/debug/examples/luau-test.exe
.\run-tests.ps1 -Binary target/release/pleamar.exe
python scripts/windows-smoke.py --binary target/release/pleamar.exe
# Requires an interactive desktop and DX12. Opens real windows; drives actions by IPC.
python scripts/windows-smoke.py --binary target/release/pleamar.exe --gui
# Read-only live services and repeated Lua reloads; needs Windows desktop services.
python scripts/windows-services.py --binary target/release/pleamar.exe
# Measures an actual visible desktop scene, process CPU/RAM and render cadence.
.\scripts\windows-performance.ps1 -Binary target/release/pleamar.exe -Scene ..\marea-plm\marea-desktop.plm
```

CI builds and tests with default Luau on Windows MSVC and Linux, then checks the
core without default features as an extra diagnostic. The CI CLI test deliberately
does not claim graphical validation. The existing Linux `run-tests.sh` remains
available; the Python runner covers its scene, documentation, vocabulary and
highlighter checks and additionally verifies process exit codes.
The September 30 source snapshot `304ab966` passed both Windows and Linux in
[the fork's CI run](https://github.com/SamuelHinestrosa/pleamar/actions/runs/36754047849).
Windows passed 103 unit tests and one integration; Linux passed 48 unit tests
and one integration. Both passed 225 language checks. This is compilation and
automated runtime evidence, not a Linux desktop session or Windows graphical
validation. Subsequent changes require their own checks.

For manual GUI validation, open `tests/native-interaction.plm`: check the timer,
click the button, click the popup, scroll over the button, type/paste Unicode and
press Enter. Resize/minimize/restore the window. Save edits to the scene and its
Luau companion, including a temporary syntax error, and verify recovery. Copy
the files before changing them. Repeat on each monitor, with mixed scaling and
after unplugging/reconnecting a monitor. For an AppBar scene, compare the work
area before launch, while `reserve` is enabled, after disabling it, after `quit`
and after `--seconds` expiry. Test click-through against another process, not
just another HWND in the same process.

Panels handle the Shell's [`TaskbarCreated` notification](https://learn.microsoft.com/en-us/windows/win32/shell/taskbar#taskbar-creation-notification)
by invalidating their cached AppBar registration and queuing placement against
the new Shell. The notification can also accompany a primary-DPI change, so
the old registration is removed before retrying to avoid duplicate registration.
Retired surfaces ignore pending placements. Hidden native-window tests cover
notification dispatch and retirement; an actual Explorer restart, primary-DPI
change and desktop work-area recovery still require interactive verification.

Registered AppBars also honor the Shell's
[`ABN_FULLSCREENAPP`](https://learn.microsoft.com/en-us/windows/win32/shell/abn-fullscreenapp)
notification: both drawing and input HWNDs drop below full-screen content, stay
there during restacking, and restore the current scene layer when it ends.
Activation and position changes notify Shell so it can order autohide bars on
the same edge. Hidden-window tests reproduce the previous always-on-top fault
and verify restoration, including a layer change. Full-screen applications and
taskbar autohide still need interactive validation; panels without a reservation
do not receive this AppBar notification.

See [the validation record and PR review](windows-validation.md) for what was
actually executed and what remains pending.

Native drag/drop uses [OLE's source and target protocol](https://learn.microsoft.com/en-us/windows/win32/shell/dragdrop)
on the window's STA message thread. Incoming `CF_HDROP` becomes UTF-8
`text/uri-list`; `CF_UNICODETEXT` becomes `text/plain;charset=utf-8`. Outgoing
`carries:` zones offer Unicode text and, for absolute Windows paths or file
URIs, an actual file list. Transfers are copy-only and bounded to 4 MiB/1,024
paths. Pleamar does not execute or open the received payload. Virtual Outlook
attachments, arbitrary binary formats and move semantics are not implemented.
`tests/native-drag.plm` is the interactive test fixture; unit tests exercise
real COM data objects, Unicode/UNC conversion and malformed payload rejection.

The Windows Marea control center now calls these operations directly. Explicit
`*.settings` commands remain available to other scenes, but are not substituted
for its device/radio/brightness/power controls. Errors remain in the panel.
The audio watcher subscribes to Core Audio
[volume/mute notifications](https://learn.microsoft.com/en-us/windows/win32/api/endpointvolume/nn-endpointvolume-iaudioendpointvolumecallback)
and [endpoint notifications](https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nn-mmdeviceapi-immnotificationclient).
Callbacks coalesce wakeups; the audio worker reads state and rebinds changed
default endpoints. A five-second recovery check covers missed notifications
and rebuilds invalid subscriptions. When registration fails, queries provide
fallback state while registration is retried; failed reads publish unavailable
state, rather than keeping old levels clickable. Commands still resolve the
default endpoint when they execute. This does not add an artificial delay to
the renderer's local slider preview. Device unplug/replug, audio-service restart
and end-to-end volume-change latency need separate interactive validation of
this event-driven implementation.
Default endpoint selection uses the isolated, undocumented `IPolicyConfig` COM
ABI; failure is reported. Bluetooth audio uses Microsoft's documented
[KS connection requests](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/kspropsetid-btaudio).
Accepted wireless requests are not reported as successful connections until
native state confirms them. Wi-Fi scanning can require Windows location consent;
no scan is attempted on startup and this application does not change privacy policy.
New open/WPA2 joins use a temporary profile. The supplied password is persisted
only after the selected adapter reports a connected temporary profile with the
matching SSID and name, using
[WlanSaveTemporaryProfile](https://learn.microsoft.com/en-us/windows/win32/api/wlanapi/nf-wlanapi-wlansavetemporaryprofile).
Existing profiles are never overwritten. Failure to save after connecting is
reported as a partial failure. The confirmation wait is bounded to 30 seconds;
timeout does not forcibly disconnect a connection another application may have
started. Wrong-password retry and profile persistence need Wi-Fi hardware tests.

`bluetooth.state` and the `bluetooth` subscription expose `present`, `enabled`,
`devices`, `le_ready`, `error` and `warning`. LE rows use an opaque `ble:` ID;
classic addresses and audio container IDs retain their own namespaces. Pass the
returned ID unchanged. LE `connection_known = false` means Windows supplied no
connection observation; `paired` is never substituted for `current`.

An LE watcher follows paired association endpoints. `bluetooth.discover` starts
an eight-second unpaired LE watcher concurrently with classic inquiry; ordinary
polling does not start nearby discovery. The bounded LE catalog retains at most
256 entries per watcher, reports overflow/metadata errors without discarding
healthy rows, and expires scanned unpaired entries after 120 seconds. No new
scan is started while one is pending. Windows may end a watcher early; errors
are reported, and a failed paired watcher retries after ten seconds.

`bluetooth.pair(id)` requires a discovered or paired LE identity, re-reads its
pairing availability, invokes Windows' basic pairing, and checks the resulting
native pairing state. Windows owns the PIN/consent ceremony; no PIN is accepted
automatically. See Microsoft's [device enumeration](https://learn.microsoft.com/en-us/windows/apps/develop/devices-sensors/enumerate-devices)
and [pairing](https://learn.microsoft.com/en-us/windows/apps/develop/devices-sensors/pair-devices)
contracts. Generic LE GATT/profile connections, unpairing and an in-panel scan
cancel action are not implemented. An explicit scan ends on its bounded timer.
New physical pairing, PIN cancellation and an unreachable-device retry remain
unverified; an already-paired identity readback is not a substitute for them.

The window catalog excludes pleamar's own surfaces, desktop/taskbar windows,
hidden/cloaked windows and ordinary owned/tool windows. It reports truncation
at 1,024 entries or a 250 ms enumeration budget, and caches executable icon
paths for live windows. `window.restore` accepts a catalog ID and revalidates
its process/thread identity before acting. It posts restoration rather than
waiting in another application's window procedure. Windows may refuse keyboard
focus under its [foreground-lock policy](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow);
that partial result is returned explicitly, without synthetic input or input-queue
attachment. Minimizing a native test window, clicking its Marea stone to restore
it, and entering Unicode text afterward passed on 0.2.2. Restore-all and focus
refusal still require desktop validation.

## Marea desktop profile

### Native recording

The Windows `recording.*` queries run through `sys.ask_async`, in the same scene
service worker. `recording.start` takes an optional GDI monitor name (otherwise
the pointer's monitor); its reply can still be `starting`. Poll `recording.state`
for `recording`, `finalizing`, `saved`, `cancelled` or `error`. Only `saved` means
the MP4 index was finalized. `recording.stop` signals cancellation even during
codec startup. State includes the path, dimensions, encoded frame count, duration,
audio frame/nonzero counts and audio discontinuities. `recording.folder` returns
the actual Windows Videos known folder plus `Marea`. Files are created exclusively,
without overwriting an existing recording; errors can retain a partial-file path.
One recording can run per process. Reload cancels the scene's owned recording;
normal exit waits for finalization. Forced termination cannot make that guarantee.

Capture requires Windows Graphics Capture, a D3D11 video processor, the Windows
H.264/AAC codecs and an available system output endpoint. Windows N installations
may need Microsoft's media components. Hardware encoding is requested through
Media Foundation; this alone does not prove a particular encoder was selected.
No microphone, Unix recorder or FFmpeg executable is opened. The path is SDR;
HDR fidelity, protected content, 4K, long recordings and audio/monitor removal
still need device-specific validation. A monitor size change stops with an error
instead of silently recording a wrongly sized surface.

The opt-in smoke records the real desktop locally and fully decodes both streams:

```powershell
cargo build --release --locked --example recording-inspect
python scripts/windows-recording-smoke.py --binary target/release/pleamar.exe --inspector target/release/examples/recording-inspect.exe --desktop --tone --report recording-normal.json
```

Use `--stop reload`, `--stop exit`, `--stop timed`, or `--stop cancel` for lifecycle checks. The
quiet test tone is optional. Clips stay in Videos/Marea and are never uploaded.
The decoder can also inspect an existing clip without capturing the desktop.

See Microsoft's [WGC free-threaded frame pool](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.direct3d11captureframepool.createfreethreaded),
[WASAPI loopback](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording)
and [hardware transform attribute](https://learn.microsoft.com/en-us/windows/win32/medfound/mf-readwrite-enable-hardware-transforms).

### Install Marea

The companion checkout adds `windows/build-desktop.py` and
`install-desktop.ps1`; it preserves the upstream Linux scene and generates
`marea-desktop.plm/.luau`. The profile keeps desktop surfaces, 60 Hz animation,
outside-click closing, monitor selection/following, real audio controls and
application search. Unsupported Linux integrations fail explicitly. Agent quotas
use Marea's packaged Node reader (Node.js 22.7+ on PATH); Codex reads local usage
observations, while Claude may use its existing token for a read-only usage
request, with cached data as fallback. Missing/old data is labelled and tokens
are never refreshed by Marea. Notifications use the native Windows collection.
Install from that checkout with a native pleamar binary:

```powershell
.\install-desktop.ps1 -PleamarBinary ..\pleamar\target\release\pleamar.exe
```

The default package is `Documents\Marea Windows`, with a Start menu shortcut.
The standalone pleamar installer above does not fetch or install Marea.

## Windows notification contract

Scenes can publish their own informational reminders with
`sys.call_async("notifications.publish", {title, body, tag}, callback)`. A stable
tag (1–16 ASCII letters/digits/`_`/`-`) makes identical retries idempotent while
the entry remains in Windows' history. Text is added as XML text nodes, never as
markup. Titles are limited to 256 characters and bodies to 2048. The publisher
uses a stable identity derived from this executable's path; it does not borrow
another application's identity or require permission to read other apps' toasts.

The installer must first create a Start menu shortcut that opens the app, then
run the **installed executable** with
`--register-notification-shortcut "C:\...\Programs\My app.lnk"`. This preserves
the shortcut's target/arguments/icon while assigning its AppUserModelID. Moving
the executable requires registering the shortcut again. Marea's installation
and update scripts do this and preserve the previous shortcut on update.
This follows Microsoft's [desktop toast registration contract](https://learn.microsoft.com/en-us/windows/win32/shell/enable-desktop-toast-with-appusermodelid).

`sys.ask("notifications.publisher")` reports `app_id`, `setting` and `error`.
A new classic publisher may have no setting before its first send; that is
reported as unavailable, not enabled. Known disabled states reject publishing.
`Show` acceptance alone is insufficient: success waits up to five seconds for
the exact tagged content in the publisher's own Windows history. Windows' DND
and notification policies remain in control of visible banners. These toasts
are silent, have no custom activation actions, and are not OS-scheduled while
the scene is closed. MSIX publisher identity has not been validated.

Opt-in native publishing validation (creates/removes only its own shortcut and
toast): `cargo test --locked native_publisher_roundtrip -- --ignored --nocapture`.
In Marea, `python windows/test-calendar-desktop.py --binary <pleamar.exe>` checks
an isolated due event, native readback, persisted receipt, reload and dismissal.
Stop any other desktop Marea instance for this test; `--hold 120` leaves the
owned test desktop available for visual inspection before cleanup.

`notifications` watches the newest 256 live toasts. `notifications.state` watches
access (`allowed`, `denied`, `unspecified`, `unavailable`), a pending consent
request and any read error. The corresponding queries are `notifications.list`
and `notifications.state`. Enumeration runs off the UI/Luau threads, at most once
per second, with a five-second timeout; unchanged snapshots are not delivered.
Revoked access/errors invalidate actionable IDs and clear the live snapshot.
No notification content is written to runtime logs by this service.

`notifications.dismiss(id)` removes that live Windows toast. IDs are process-local
and checked against Windows' native ID, creation time and available app identity
before use. `notifications.invoke(id, "open-app")` launches a known AppsFolder app
when sender metadata exists. It does not activate the original message, reply or
other private toast buttons. Classic Win32 notifications can return `E_NOTIMPL`
for `AppInfo`; they still show their actual text, with empty sender/icon/actions.
Urgency and original action buttons are omitted, not invented.

`notifications.request_access` dispatches the Windows consent request on the UI
thread only when explicitly requested by scene logic. Marea exposes the action
in Settings → Notices when access is unspecified, and never requests it at
startup. Denied access must be re-enabled by the user in Windows privacy settings.
No privacy setting is changed by the port. On this Windows 11 host the actual
unpackaged process reports allowed access and reads toasts; other Windows
versions/policies and the denied/consent path still need native validation.
Microsoft documents the `userNotificationListener` manifest capability for
packaged applications; this run did not install or validate an MSIX package.

Windows remains the notification owner: `notifications.keep` returns an explicit
unsupported error. Marea's history/snooze/mute/DND apply inside Marea; Windows
banner suppression, deep activation and arbitrary third-party retention are not
implemented. Marea's own calendar reminders use the separately registered native
publisher described below.

`--register-notification-shortcut <existing.lnk>` assigns a stable installation
identity to an existing Unicode Start menu shortcut. It preserves the shortcut's
target and arguments. `notifications.publish` creates an informational, silent
toast and acknowledges delivery only after the exact tagged title/body appears
in that publisher's Windows history. Known disabled states, asynchronous failure
and the confirmation deadline are returned as errors. This does not bypass DND
or guarantee a visible banner. Marea persists the tag before sending and retries
within its existing due window; closed-app scheduling and custom toast actions
are not provided. The installed publisher and reminder/reload/dismissal flow
were exercised on Windows 11; other OS versions and policy states remain untested.

References: [notification listener](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/notification-listener),
[desktop toast identity](https://learn.microsoft.com/en-us/windows/win32/shell/enable-desktop-toast-with-appusermodelid).

## Windows tray contract

`tray` publishes the same list shape as the Linux tray service; `tray.list`
queries it. Explorer's per-user `NotifyIconSettings` cache is a candidate list,
never evidence that an application is running. Each GUID or HWND/UID must pass
`Shell_NotifyIconGetRect`; window/thread/process identities and the Explorer
process are checked again before actions. Removed entries lose their opaque keys.
An unavailable catalog clears the published list. Polling runs on a worker every
two seconds, shares snapshots and does not open the Windows overflow panel.

`tray.activate(key)` invokes the actual Explorer accessibility element;
`tray.context(key)` asks that element to show the application's native menu.
Use `sys.call_async` for these calls. Only an explicit action may expand Explorer's
hidden-icon panel to locate an icon. Matching uses the confirmed native rectangle
and notification-icon provider identity, excluding the overflow chevron, taskbar
app buttons and system quick-settings buttons. Requests are serialized, stale or
ambiguous targets fail, and provider calls have connection/transaction timeouts.
The implementation does not inject into Explorer, write tray preferences, read
another process's memory, move the mouse or synthesize keyboard input.

`tray.menu` returns an empty tree for a valid icon: Windows applications own
their menus, so clients can use `tray.context`. `tray.menu_click` is unsupported.
The `tray.state` service/query exposes availability, initialization, errors,
`enumeration = "explorer-cache"`, `cached_metadata = true`, and `complete = false`.
Titles use executable file descriptions/names, not historical tooltip progress
or login states. Icon PNGs come from Explorer's cache and may lag animated icons.

This is deliberately **limited support**, not a complete notification-area host.
The current host exposed 20 confirmed icons while Explorer also displayed system
Bluetooth and a protected NVIDIA icon that this catalog could not identify.
Windows 10, alternate Explorer versions, Explorer restart, taskbar auto-hide,
mixed DPI and elevated-app actions still need validation. The cache schema and
Explorer accessibility element names are version-dependent. No registry policy
or process access permission is relaxed to obtain more icons.

Run `cargo run --example tray-fixture` (or append `-- --legacy`) and open
`tests/native-tray.plm` for the owned interactive probe. The fixture expires after
15 minutes and removes its own icon on close. CI compiles it but does not assert
that a hosted runner exercised an interactive desktop.

References: [notification icon identity and rectangles](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyicongetrect),
[UI Automation context menus](https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomationelement3-showcontextmenu),
[UI Automation threading](https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-threading),
[executable description property](https://learn.microsoft.com/en-us/windows/win32/properties/props-system-filedescription).

## Windows media contract

`media` subscriptions and `media.state` queries read Windows' current system
media session. The map includes `available`, `error`, `player`, `title`, `artist`,
`album`, `playing`, `can_toggle`, `can_next` and `can_previous`. No session or a
failed subscription read clears the stale metadata and capabilities. A paused
session remains available. Players must participate in Windows' media transport
API; the service cannot add controls that an application does not expose.

`media.toggle`, `media.play`, `media.pause`, `media.next` and `media.previous`
accept either no arguments or the `player` string from the observed snapshot.
Supplying it rejects a different current application before sending a command.
This identifies an application, not an individual session within an application.
The runtime checks current capabilities again; toggle falls back to a supported
play or pause operation when the player lacks a dedicated toggle operation.
Declined operations return an error rather than inventing a playback state.

Use `sys.call_async` and `sys.ask_async` from interactive scenes. WinRT async
operations have a two-second deadline and are cancelled when it expires; this
does not preempt arbitrary synchronous COM calls. Marea suppresses duplicate
pending actions, re-reads native state after a reply, discards stale callbacks
and releases its pending state after 4.5 seconds with a visible timeout notice.
Its media artwork remains upstream's gradient, not a downloaded album image.

The manager is reused across reads; a failed session inventory invalidates it.
`tests/README.md` describes the owned SMTC fixture and opt-in native test.
Mouse, Unicode, capability and reload observations are recorded in
`windows-media-validation.json`; the fixture itself does not play audio.

References: [Windows media control capabilities](https://learn.microsoft.com/en-us/uwp/api/windows.media.control.globalsystemmediatransportcontrolssessionplaybackcontrols.isnextenabled?view=winrt-28000),
[SMTC integration](https://learn.microsoft.com/en-us/windows/apps/develop/media-playback/integrate-with-systemmediatransportcontrols).

## Performance reports (0.2.6)

`pleamar --report --seconds 30 --out report.md` discovers scenes in the current
Windows pipe namespace. `PLEAMAR_SOCKET_DIR=marea-desktop` selects an installed
Marea session. Native APIs provide process CPU time, working set, system CPU and
physical memory; scene probes include available frame/GPU/monitor information.
System process rankings, temperatures, core clocks and system GPU load are
explicitly unavailable in the Windows report. No Unix `date`, `/proc` or socket
server is needed. Windows and Unix path-valued environment entries are redacted.
Absence of frames or hardware counters is not a successful rendering benchmark.

## Per-player volume and bounded filename matching

`media.state` additionally returns `can_volume`, `volume` (0–1) and
`volume_error`. `media.volume(player, level)` resolves the current SMTC player
again and changes only matching Core Audio output sessions. Packaged process
identities, explicit window AppUserModelIDs and exact executable identities are
supported. Unknown identities fail; the endpoint master volume is never used
as a fallback. Multiple identifiable output sessions of one application are
changed together, with best-effort rollback if a setter fails. Exclusive audio
or a player without an identifiable Core Audio session may be unavailable.
This is application volume, not an individual browser tab or media track.

The owned `media-fixture --audio` creates a silent Core Audio stream for
`native_media_fixture_volume` (opt-in, with `PLEAMAR_MEDIA_FIXTURE_PLAYER`). It
checks 20%, 70%, 0%, 100%, stale-player rejection, restoration and unchanged
endpoint master volume. It does not record or play any user audio.

`search.files` ranks filename tokens, accent variants and small typing mistakes
with fixed traversal/time/result budgets. It reads names/metadata only and
reports `truncated` rather than claiming exhaustive indexing.

See [desktop controls validation](windows-desktop-controls.md).
