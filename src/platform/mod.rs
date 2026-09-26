//! The boundary with the system. Everything that knows about Wayland —and, the
//! day it's needed, about Win32 or AppKit— lives down here; the rest of the
//! program only knows this.
//!
//! A platform has to know how to do three things:
//!  · put up the surfaces a scene asks for (`Surface`: logical size,
//!    anchor, level, exclusive zone, screens) and hand them to the render as
//!    sheets, also when a monitor arrives or leaves, or changes scale;
//!  · tell the render where the mouse is and when it clicks;
//!  · tell the system where the mouse gets in (`PlatformWindow::update_input_region`),
//!    so that what's transparent lets the click through.
//!
//! And two that are not about windows: finding an icon by its name, and the
//! **services** —what happens in the system: workspaces, active window…—, which
//! the logic asks for by a name that is the same on every system. What a
//! system doesn't have, it says it doesn't have, and the scene decides what to do without it.

#[cfg(not(target_os = "linux"))]
use crate::scene::{Surface, ToRender};
#[cfg(not(target_os = "linux"))]
use std::sync::mpsc::Sender;

/// A piece of system data, shaped like JSON: it's what a service tells
/// the logic, which receives it as a table.
#[derive(Clone, Debug)]
pub enum SysValue {
    Null,
    Bool(bool),
    Num(f64),
    Text(String),
    List(Vec<SysValue>),
    Map(Vec<(String, SysValue)>),
}

/// Starts listening to a service. `notify` is called with the current state and
/// then every time it changes. Returns whether this system has it.
///
/// The names are the same everywhere:
///  · `workspaces` → `{ active = 3, list = { { id, name, windows, monitor }, … } }`
///  · `window`     → `{ title, class }`
///  · `apps`       → `{ { name, exec, icon }, … }`, once
///  · `audio`      → `{ volume = 0.54, muted = false }`
///  · `battery`    → `{ present, percent, charging }`
///  · `network`    → `{ online, kind = "wired" | "wifi" | "none", name, strength }`
///  · `media`      → `{ playing, title, artist, album, player }`, or `{ player = "" }` if nothing is playing
///  · `clock`      → `{ hour, minute, second, day, month, year, weekday, time, date }`, when the minute changes
///  · `clock.seconds` → the same, every second
///  · `files:x.json` → the text of that file when it changes
///
/// And two that can only be asked, with `sys.ask`: `env` (an environment variable) and
/// `clipboard` (whatever has been copied); `clipboard.set` writes to it.
pub fn service(from: &str, name: &str, notify: Box<dyn Fn(SysValue) + Send>) -> bool {
    // The time, without calling anyone.
    if name == "clock" || name == "clock.seconds" {
        return clock::service(name, notify);
    }
    // `files:settings.json`: notifies when that file changes, also if someone else touches it.
    if let Some(which) = name.strip_prefix("files:") {
        return files::watch(from, which, notify);
    }
    #[cfg(target_os = "linux")]
    if name == "apps" {
        // Reading hundreds of files is not a matter of an instant: in its own thread.
        return std::thread::Builder::new().name("apps".into()).spawn(move || notify(desktop::applications())).is_ok();
    }
    #[cfg(target_os = "linux")]
    match name {
        "audio" => return system::audio(notify),
        "battery" => return system::battery(notify),
        "brightness" => return system::brightness(notify),
        "network" => return system::network(notify),
        "media" => return mpris::service(notify),
        "notifications" => return notifications::service(notify),
        "tray" => return tray::service(notify),
        _ => {}
    }
    // `PLEAMAR_GENERIC=1` skips the Hyprland path: it's how to test here what
    // the other compositors will see.
    #[cfg(target_os = "linux")]
    if hyprland::is_present() && std::env::var_os("PLEAMAR_GENERIC").is_none() {
        return hyprland::service(name, notify);
    }
    // Any other Wayland: the protocols everyone understands.
    #[cfg(target_os = "linux")]
    if matches!(name, "window" | "workspaces") {
        return compositor::service(name, notify);
    }
    let _ = (name, notify);
    false
}

/// Ask a service to do something: `workspaces.focus`, 3.
pub fn command(from: &str, name: &str, args: &[SysValue]) -> Result<(), String> {
    if name.starts_with("files.") {
        return files::command(from, name, args);
    }
    if let ("clipboard.set", [SysValue::Text(t)]) = (name, args) {
        clipboard_write(t);
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if let ("apps.launch", [SysValue::Text(o)]) = (name, args) {
        return desktop::launch(o);
    }
    #[cfg(target_os = "linux")]
    if name.starts_with("brightness.") {
        return system::brightness_command(name, args);
    }
    #[cfg(target_os = "linux")]
    if name.starts_with("audio.") {
        return system::audio_command(name, args);
    }
    #[cfg(target_os = "linux")]
    if name.starts_with("session.") {
        return system::session_command(name, args);
    }
    #[cfg(target_os = "linux")]
    if name.starts_with("tray.") {
        return tray::command(name, args);
    }
    #[cfg(target_os = "linux")]
    if name.starts_with("notifications.") {
        return notifications::command(name, args);
    }
    #[cfg(target_os = "linux")]
    if name.starts_with("media.") {
        return mpris::command(name, args);
    }
    #[cfg(target_os = "linux")]
    if hyprland::is_present() && std::env::var_os("PLEAMAR_GENERIC").is_none() {
        return hyprland::command(name, args);
    }
    #[cfg(target_os = "linux")]
    if name.starts_with("workspaces.") {
        return compositor::command(name, args);
    }
    let _ = args;
    Err(format!("this system cannot do '{name}' yet"))
}

/// Where pleamar keeps what it has to remember from one run to the next —which
/// plugins have been approved—: each system's settings folder.
pub fn config_dir() -> std::path::PathBuf {
    let from = |v: &str| std::env::var_os(v).filter(|x| !x.is_empty()).map(std::path::PathBuf::from);
    let base = if cfg!(target_os = "windows") {
        from("APPDATA")
    } else if cfg!(target_os = "macos") {
        from("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        from("XDG_CONFIG_HOME").or_else(|| from("HOME").map(|h| h.join(".config")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("pleamar")
}

/// Ask a service something and wait for the answer: `tray.menu`, of whom.
/// It can take a while —there's another application on the other side—, and
/// that's why it's up to the logic, which can wait without it showing.
pub fn query(from: &str, name: &str, args: &[SysValue]) -> Result<SysValue, String> {
    if name.starts_with("files.") {
        return files::query(from, name, args);
    }
    match (name, args) {
        // The environment: what the session told this program at startup.
        ("env", [SysValue::Text(which)]) => {
            return Ok(std::env::var_os(which).map_or(SysValue::Null, |v| SysValue::Text(v.to_string_lossy().into_owned())));
        }
        ("env", _) => return Err("`env` takes the name of one variable: sys.ask(\"env\", \"HOME\")".into()),
        ("clipboard", []) => return Ok(clipboard_read().map_or(SysValue::Null, SysValue::Text)),
        // Is that the password of whoever owns the session? What a lock screen
        // needs in order to open. It is slow on purpose when it isn't.
        #[cfg(target_os = "linux")]
        ("auth.check", [SysValue::Text(password)]) => return auth::verify(password).map(SysValue::Bool),
        ("auth.check", _) => return Err("`auth.check` takes the password, and answers true or false: sys.ask(\"auth.check\", text.password)".into()),
        _ => {}
    }
    #[cfg(target_os = "linux")]
    if name.starts_with("tray.") {
        return tray::query(name, args);
    }
    let _ = args;
    Err(format!("this system cannot answer '{name}' yet"))
}

/// Opens or closes the scene's popup number `k`: a child surface of the
/// main one, at `[x, y, width, height]` inside it, that shows the scene
/// from `origin`. The sheet reaches the render like the others; if the system
/// closes it (someone clicked outside), it reports with `ToRender::PopupClosed`.
///
/// On Wayland, an `xdg_popup`. On Windows it will be a frameless window with
/// `WS_EX_NOACTIVATE`; on macOS, an `NSPanel`.
pub fn popup(k: usize, what: Option<([i32; 4], (f32, f32))>) {
    #[cfg(target_os = "linux")]
    wayland::popup(k, what);
    let _ = (k, what);
}

/// Engages or releases the session lock for surface `which`, which is
/// `kind: lock`: `Some((bounds, origin))` engages it —one surface per monitor, with
/// those bounds centred on each— and `None` releases it. Whether it is REALLY
/// engaged is said by the system, with `ToRender::LockScreen(true)`.
///
/// On Wayland it's `ext-session-lock`: the compositor guarantees it, not the drawing.
/// On Windows it will be `LockWorkStation`, which brings its own screen; on macOS, the
/// screen saver with password.
pub fn lock_screen(which: usize, what: Option<((u32, u32), (f32, f32))>) {
    #[cfg(target_os = "linux")]
    wayland::lock_screen(which, what);
    let _ = (which, what);
}

/// Sticks surface `which` to another edge, while running. On Wayland it's a
/// layer-shell request —`set_anchor` and `set_margin` work on a live
/// surface, without creating it again—; on Windows it will be moving the window and
/// repositioning its AppBar, and on macOS moving the `NSPanel`.
/// A surface changes level while running: `level: top, overlay while open`.
pub fn relayer(which: usize, level: crate::scene::Level) {
    #[cfg(target_os = "linux")]
    wayland::relayer(which, level);
    let _ = (which, level);
}

pub fn reanchor(which: usize, anchor: crate::scene::SurfaceAnchor) {
    #[cfg(target_os = "linux")]
    wayland::reanchor(which, anchor);
    let _ = (which, anchor);
}

/// That a process we launch does not outlive us, not even if we're killed
/// forcibly. On Linux we ask the kernel; on Windows it will be a Job Object.
pub fn die_with_parent(command: &mut std::process::Command) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // It only calls `prctl`, which is one of those that can be used between `fork` and `exec`.
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
    }
    let _ = command;
}

/// Commands from outside: a global compositor shortcut, a script, another
/// application. One line of text per command, through a socket named after the
/// scene. `pleamar --say "emit toggle"` is the other end.
///
/// On Linux and macOS, a Unix socket. On Windows it will be a named pipe.
#[cfg(unix)]
pub fn listen_for_commands(scene: &str, receive: Box<dyn Fn(String) -> Option<String> + Send>) {
    use std::io::{BufRead, BufReader, Write};
    let Some(mut path) = command_socket_path(scene) else { return };
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    // If there's already ANOTHER scene with this name listening, the socket is its.
    // It used to be deleted without looking: the newcomer took it, and when it left
    // it left the first one deaf forever —a ten-second trial run took the
    // keyboard shortcuts away from the everyday bar—. Only the socket of
    // someone no longer there is deleted; if they are there, this one answers to its name and its pid.
    if std::os::unix::net::UnixStream::connect(&path).is_ok() {
        let own = format!("{scene}-{}", std::process::id());
        eprintln!("orders · another '{scene}' is already listening: this one answers to '{own}' (pleamar --say {own} …)");
        let Some(other) = command_socket_path(&own) else { return };
        path = other;
    }
    let _ = std::fs::remove_file(&path);
    let Ok(listener) = std::os::unix::net::UnixListener::bind(&path) else { return };
    std::thread::Builder::new()
        .name("commands".into())
        .spawn(move || {
            for mut c in listener.incoming().map_while(Result::ok) {
                let Ok(reader) = c.try_clone() else { continue };
                for line in BufReader::new(reader).lines().map_while(Result::ok) {
                    // A question (`get open`) is answered back the way it came.
                    if let Some(answer) = receive(line) {
                        let _ = writeln!(c, "{answer}");
                    }
                }
            }
        })
        .ok();
}

#[cfg(unix)]
fn command_socket_path(scene: &str) -> Option<std::path::PathBuf> {
    let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
    Some(std::path::Path::new(&base).join("pleamar").join(format!("{scene}.sock")))
}

/// Tell a running scene something. Without a name, to the only one there is.
#[cfg(unix)]
pub fn send(scene: Option<&str>, command: &str) -> Result<(), String> {
    use std::io::Write;
    let path = match scene {
        Some(e) => command_socket_path(e).ok_or("I don't know where the sockets are")?,
        None => {
            let dir = command_socket_path("x").and_then(|r| r.parent().map(|p| p.to_owned())).ok_or("I don't know where the sockets are")?;
            let alive: Vec<_> = std::fs::read_dir(&dir).map_err(|_| "there is no scene running")?.filter_map(Result::ok).map(|e| e.path()).filter(|p| std::os::unix::net::UnixStream::connect(p).is_ok()).collect();
            match alive.as_slice() {
                [one] => one.clone(),
                [] => return Err("there is no scene running".into()),
                several => return Err(format!("there are several scenes running; say which: {}", several.iter().filter_map(|p| p.file_stem()).map(|n| n.to_string_lossy()).collect::<Vec<_>>().join(", "))),
            }
        }
    };
    let mut s = std::os::unix::net::UnixStream::connect(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    writeln!(s, "{command}").map_err(|e| e.to_string())?;
    // Everything has been said: if it was a question, now comes whatever they answer.
    let _ = s.shutdown(std::net::Shutdown::Write);
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(2)));
    let mut answer = String::new();
    let _ = std::io::Read::read_to_string(&mut s, &mut answer);
    if !answer.is_empty() {
        print!("{answer}");
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn listen_for_commands(_: &str, _: Box<dyn Fn(String) -> Option<String> + Send>) {}
#[cfg(not(unix))]
pub fn send(_: Option<&str>, _: &str) -> Result<(), String> {
    Err("this system has nowhere to receive commands yet: the named pipe is missing".into())
}

/// The system clipboard. `arboard` speaks it on all three systems; it lives
/// here so the core keeps knowing nothing about any of them.
pub fn clipboard_read() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

pub fn clipboard_write(text: &str) {
    // On Wayland whoever copies has to stay alive to serve what was copied: a
    // thread that holds on to it until someone else copies something else.
    let text = text.to_owned();
    std::thread::spawn(move || {
        if let Ok(mut c) = arboard::Clipboard::new() {
            #[cfg(target_os = "linux")]
            {
                use arboard::SetExtLinux;
                let _ = c.set().wait().text(text);
            }
            #[cfg(not(target_os = "linux"))]
            let _ = c.set_text(text);
        }
    });
}

/// Give back to the system the memory that was used to read a scene. Compiling
/// Marea (3 700 instructions) goes through 128 MB for a moment to settle at 9,
/// and glibc keeps what was freed in each thread's arenas: the process
/// stayed at 355 MB of RSS forever. It's done a while later, when the
/// workshop and the logic have already done their thing with it. On other systems, or with
/// another libc, it's not needed or we don't know how to ask: it does nothing.
pub fn release_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    std::thread::Builder::new()
        .name("release".into())
        .spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(3));
            // It only gives back free pages: it touches nothing alive.
            unsafe { libc::malloc_trim(0) };
        })
        .ok();
}

/// What the render asks of a system window.
pub trait PlatformWindow: Send {
    /// Where the mouse gets in: only through these boxes, in logical pixels.
    fn update_input_region(&self, boxes: &[[i32; 4]]);
    /// Which cursor shows while the mouse is over it.
    fn cursor(&self, c: crate::scene::Cursor);
    /// Grab or release the keyboard with the scene running: a launcher wants it
    /// entirely while it's open, and not at all when it isn't.
    fn keyboard(&self, t: crate::scene::Keyboard);
    /// That the system report (`ToRender::Frame`) when it wants the frame after
    /// the one about to be presented. Where that's unknown, it doesn't report, and the clock sets the pace.
    fn request_frame(&self) {}
    /// What the system blurs behind the window: rectangles in logical
    /// pixels, those of its glass. Empty, nothing. Where it can't be asked for, it does nothing.
    fn update_blur_region(&self, _boxes: &[[i32; 4]]) {}
    /// Ask for what's shown on screen in that box of the window —x, y, width and
    /// height, logical—, **with the window on top**: it arrives as `ToRender::Backdrop`. It's
    /// what lets you see what's behind the glass: what the window itself painted
    /// gets subtracted from it. With `on_change`, it doesn't arrive until something changes in that
    /// box; without it, the next time the compositor paints the screen.
    /// `false` if it can't be asked for, or if there's already one on the way.
    fn capture_backdrop(&self, _bounds: [i32; 4], _on_change: bool) -> bool {
        false
    }
    /// Forget the capture that's on the way: it won't arrive anymore.
    fn cancel_backdrop(&self) {}
    /// On which monitor it is, by name, and where on it: its top left corner,
    /// logical. `None` where that is not known.
    fn desktop_place(&self) -> Option<(String, (i32, i32))> {
        None
    }
}

/// A capture of what was seen on screen in a box of a window.
pub struct Backdrop {
    /// Which window it belongs to.
    pub sheet: u32,
    /// In real pixels, four bytes per pixel: blue, green, red and alpha.
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    /// With what opacity the compositor blended ours: 1 except in a
    /// window whose opacity is lowered (Hyprland: `decoration:active_opacity`).
    pub opacity: f32,
    /// The pixels, where the compositor left them, without copying them. `None` if the capture failed.
    pub data: Option<std::sync::Arc<dyn AsRef<[u8]> + Send + Sync>>,
}

/// Whether the scene running wants to know where the mouse is on the whole
/// desktop (`cursor.x`): the render says so every time a scene arrives.
pub static CURSOR_WANTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Where the mouse is when it is not over us. Wayland does not tell a surface
/// that —on purpose—, so it is asked of whoever knows: Hyprland, through its
/// socket, about thirty times a second and only while a scene wants it. It
/// reaches the render as `ToRender::Cursor` when it moves. Elsewhere nothing
/// arrives, and `cursor.x` is only known over the scene, like `pointer.x`.
pub fn watch_cursor(to_render: std::sync::mpsc::Sender<crate::scene::ToRender>) {
    #[cfg(target_os = "linux")]
    if hyprland::is_present() && std::env::var_os("PLEAMAR_GENERIC").is_none() {
        hyprland::watch_cursor(to_render);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = to_render;
}

/// The keyboard layout pleamar's own window was given, as xkb text.
static HOST_KEYMAP: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn set_host_keymap(k: String) {
    *HOST_KEYMAP.lock().unwrap() = Some(k);
}

pub fn host_keymap() -> Option<String> {
    HOST_KEYMAP.lock().unwrap().clone()
}

/// The compositor inside the scene (`windows`): where to tell it things.
pub type NestSender = Box<dyn Fn(crate::scene::ToNest) + Send>;

/// What starts one: given how many windows the scene holds and where to
/// report to the render, it answers where to talk to it —or nothing, if it
/// could not start—.
pub type NestStarter = fn(usize, std::sync::mpsc::Sender<crate::scene::ToRender>) -> Option<NestSender>;

static NEST: std::sync::OnceLock<NestStarter> = std::sync::OnceLock::new();

/// pleamar itself holds no compositor: whoever builds one on top of it
/// (pleamar-wm) hands it over here before `run()`, and the scenes' `windows`
/// fill from it.
pub fn provide_windows(start: NestStarter) {
    let _ = NEST.set(start);
}

pub fn start_nest(max: usize, to_render: std::sync::mpsc::Sender<crate::scene::ToRender>) -> Option<NestSender> {
    match NEST.get() {
        Some(start) => start(max, to_render),
        None => {
            eprintln!("windows · this pleamar holds no other programs' windows: open the scene with pleamar-wm");
            None
        }
    }
}

#[cfg(target_os = "linux")]
mod auth;
#[cfg(target_os = "linux")]
mod notifications;
#[cfg(target_os = "linux")]
mod tray;
#[cfg(target_os = "linux")]
mod compositor;
mod files;
mod clock;
#[cfg(target_os = "linux")]
mod desktop;
#[cfg(target_os = "linux")]
mod hyprland;
#[cfg(target_os = "linux")]
mod mpris;
#[cfg(target_os = "linux")]
mod system;
#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
pub use wayland::run_event_loop;

/// No platform yet: the core compiles —it's the guard that it stays
/// portable— but there's nowhere to paint.
#[cfg(not(target_os = "linux"))]
pub fn run_event_loop(_: Vec<Surface>, _: u32, _: wgpu::Instance, _: Sender<ToRender>) {
    eprintln!("pleamar cannot put windows on this system yet: its src/platform/ is missing");
    std::process::exit(1);
}

/// Where an icon's file is, by its name.
#[cfg(target_os = "linux")]
pub use wayland::icon;
#[cfg(not(target_os = "linux"))]
pub fn icon(_: &str) -> Option<std::path::PathBuf> {
    None
}
