//! `thumbnails`: every window of the desktop, with a small picture of it —an
//! overview, a switcher, a dock's previews—, including windows the scene does
//! not host. Spoken in the standard protocols: ext-foreign-toplevel-list to
//! know the windows, ext-foreign-toplevel-image-capture-source to name one,
//! and ext-image-copy-capture to copy it. Any compositor that ships them
//! (pleamar-wm, Hyprland, sway, niri…) works the same; where they are missing,
//! the windows are listed with no picture.
//!
//! Nothing is copied until the logic asks for it (`thumbnails.want`, the ids
//! it shows, or "all"), and then only when the window changes: the compositor
//! holds each frame until there is something new, and here no more than a few
//! a second are taken. A frame with the same pixels as the last one —Hyprland
//! fills every frame waiting on a monitor when anything on it redraws— is
//! nothing new: no file is written and nobody is told. Each picture is made small (`SIDE` at most) and written
//! as a PNG under `$XDG_RUNTIME_DIR/pleamar/thumbnails`; `picture` is that
//! path with a version (`…/3.png?12`), which `image … = from` paints and
//! reloads as it changes.
//!
//! `thumbnails.live` asks the same of the windows an overview shows moving:
//! their frames skip the file. Each one is kept here, in memory, as it came,
//! and `picture` names it (`thumbnails:3?12`); `image … = from` takes it from
//! here and makes it the size it is drawn at. Up to `LIVE_EVERY` a second,
//! and still only when the pixels change. A window no longer asked for live
//! gives its frame back as a file, so a whole frame is never held for nobody.
//!
//! A window the compositor stops copying keeps its last picture, and `stale`
//! says it. Only the compositor can say so (the session `stopped`): a frame
//! that does not come is the same, from here, whether the window did not
//! change or the compositor will not copy it —Hyprland copies only windows that
//! overlap a monitor, so a column scrolled away waits for good, not stale—.

use super::SysValue;
use std::collections::{HashMap, HashSet};
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use wayland_client::protocol::{wl_buffer, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1::{self as toplevel, ExtForeignToplevelHandleV1},
    ext_foreign_toplevel_list_v1::{self as toplevel_list, ExtForeignToplevelListV1},
};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
    ext_image_capture_source_v1::ExtImageCaptureSourceV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1::{self as copy_frame, ExtImageCopyCaptureFrameV1},
    ext_image_copy_capture_manager_v1::{self as copy_manager, ExtImageCopyCaptureManagerV1},
    ext_image_copy_capture_session_v1::{self as copy_session, ExtImageCopyCaptureSessionV1},
};

/// The longest side of a thumbnail, in pixels.
const SIDE: u32 = 400;
/// A window's pictures, at most this often.
const EVERY: Duration = Duration::from_millis(300);
/// A live window's, at most this often: Hyprland fills every frame waiting on
/// a monitor each time anything on it redraws, so without a pace a 144 Hz
/// monitor would have every live window read and compared 144 times a second.
const LIVE_EVERY: Duration = Duration::from_millis(33);

/// A live window's last frame, as the compositor left it.
pub struct Frame {
    /// Bumped each time it changes: whoever drew the last one can tell this is the same.
    pub version: u64,
    pub width: u32,
    pub height: u32,
    /// Blue, green, red and alpha, premultiplied, `width * 4` bytes a row.
    pub pixels: Vec<u8>,
    /// XRGB: the alpha byte is not to be read.
    pub opaque: bool,
}

/// Every live window's last frame, by the number in its `picture`.
static FRAMES: Mutex<Option<HashMap<u64, Arc<Frame>>>> = Mutex::new(None);

/// The frame a `picture` names (`thumbnails:3`, without its version), if it is one.
pub fn frame(name: &str) -> Option<Arc<Frame>> {
    let seq: u64 = name.strip_prefix("thumbnails:")?.parse().ok()?;
    FRAMES.lock().unwrap().as_ref()?.get(&seq).cloned()
}

fn forget_frame(seq: u64) {
    if let Some(f) = FRAMES.lock().unwrap().as_mut() {
        f.remove(&seq);
    }
}

/// Which windows to copy.
#[derive(Default, Clone, PartialEq)]
enum Wanted {
    #[default]
    None,
    All,
    These(HashSet<String>),
}

impl Wanted {
    fn has(&self, w: &Window) -> bool {
        match self {
            Wanted::None => false,
            Wanted::All => true,
            Wanted::These(ids) => ids.contains(&w.id),
        }
    }
}

/// A memory buffer the compositor copies a window into.
struct Shm {
    buffer: wl_buffer::WlBuffer,
    pool: wl_shm_pool::WlShmPool,
    map: *mut u8,
    len: usize,
    size: (u32, u32),
    format: wl_shm::Format,
}

// SAFETY: the map is only touched with the state's lock held.
unsafe impl Send for Shm {}

impl Drop for Shm {
    fn drop(&mut self) {
        self.buffer.destroy();
        self.pool.destroy();
        // SAFETY: mapped by `Shm::new` with this length, and not used after.
        unsafe { libc::munmap(self.map as *mut libc::c_void, self.len) };
    }
}

impl Shm {
    fn new(shm: &wl_shm::WlShm, size: (u32, u32), format: wl_shm::Format, qh: &QueueHandle<State>) -> Option<Shm> {
        let (w, h) = size;
        let stride = w as usize * 4;
        let len = stride * h as usize;
        if len == 0 {
            return None;
        }
        // SAFETY: a name of our own; the fd is owned from here on.
        let fd = unsafe { libc::memfd_create(c"pleamar-thumbnail".as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return None;
        }
        // SAFETY: just made, and owned by nothing else.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        use std::os::fd::AsRawFd;
        // SAFETY: our own fd, made as long as the buffer.
        if unsafe { libc::ftruncate(fd.as_raw_fd(), len as libc::off_t) } != 0 {
            return None;
        }
        // SAFETY: the whole of the fd just sized, shared with the compositor.
        let map = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd.as_raw_fd(), 0) };
        if map == libc::MAP_FAILED {
            return None;
        }
        let pool = shm.create_pool(fd.as_fd(), len as i32, qh, ());
        let buffer = pool.create_buffer(0, w as i32, h as i32, stride as i32, format, qh, ());
        Some(Shm { buffer, pool, map: map as *mut u8, len, size, format })
    }
}

/// Copying one window: its session, what the compositor said it takes, the
/// buffer, and the frame on its way.
struct Capture {
    session: ExtImageCopyCaptureSessionV1,
    _source: ExtImageCaptureSourceV1,
    size: Option<(u32, u32)>,
    formats: Vec<wl_shm::Format>,
    shm: Option<Shm>,
    frame: Option<ExtImageCopyCaptureFrameV1>,
    next: Instant,
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(f) = self.frame.take() {
            f.destroy();
        }
        self.session.destroy();
        self._source.destroy();
    }
}

struct Window {
    handle: ExtForeignToplevelHandleV1,
    /// Its turn in the list, and the name of its file.
    seq: u64,
    id: String,
    title: String,
    app: String,
    picture: String,
    version: u64,
    /// The pixels of the last picture, to tell a new one from the same again.
    pixels: u64,
    /// Whether `picture` names a frame in memory (`thumbnails.live`) or a file.
    live: bool,
    stale: bool,
    capture: Option<Capture>,
}

#[derive(Default)]
struct State {
    list: Option<ExtForeignToplevelListV1>,
    sources: Option<ExtForeignToplevelImageCaptureSourceManagerV1>,
    copies: Option<ExtImageCopyCaptureManagerV1>,
    shm: Option<wl_shm::WlShm>,
    windows: HashMap<u32, Window>,
    seq: u64,
    wanted: Wanted,
    /// Which windows to copy live, to the renderer and not to a file.
    live: Wanted,
    /// Everyone listening (the scene's `service thumbnails`, the logic's `sys.watch`…).
    dispatch: Vec<Box<dyn Fn(SysValue) + Send>>,
    last: Option<Vec<u8>>,
    dir: std::path::PathBuf,
}

fn key(p: &impl Proxy) -> u32 {
    p.id().protocol_id()
}

impl State {
    fn report(&mut self) {
        if self.dispatch.is_empty() {
            return;
        }
        let mut windows: Vec<&Window> = self.windows.values().filter(|w| !w.id.is_empty()).collect();
        windows.sort_by_key(|w| w.seq);
        let list = SysValue::List(
            windows
                .iter()
                .map(|w| {
                    SysValue::Map(vec![
                        ("id".into(), SysValue::Text(w.id.clone())),
                        ("title".into(), SysValue::Text(w.title.clone())),
                        ("app".into(), SysValue::Text(w.app.clone())),
                        ("picture".into(), SysValue::Text(w.picture.clone())),
                        ("stale".into(), SysValue::Bool(w.stale)),
                    ])
                })
                .collect(),
        );
        // Told only what changed: the same list twice is nothing new.
        let fingerprint = format!("{:?}", windows.iter().map(|w| (&w.id, &w.title, &w.app, &w.picture, w.stale)).collect::<Vec<_>>()).into_bytes();
        if self.last.as_ref() == Some(&fingerprint) {
            return;
        }
        self.last = Some(fingerprint);
        let value = SysValue::Map(vec![("list".into(), list), ("capturing".into(), SysValue::Bool(self.copies.is_some() && self.sources.is_some()))]);
        for d in &self.dispatch {
            d(value.clone());
        }
    }

    fn wants(&self, w: &Window) -> bool {
        self.wanted.has(w) || self.lives(w)
    }

    fn lives(&self, w: &Window) -> bool {
        self.live.has(w)
    }

    /// A window no longer live keeps its picture, but as a file like any
    /// other: the whole frame is not held for nobody, and a window that does
    /// not change again would never come back through `taken`.
    fn settle(&mut self, k: u32) {
        let Some(w) = self.windows.get_mut(&k) else { return };
        let frame = FRAMES.lock().unwrap().as_mut().and_then(|f| f.remove(&w.seq));
        w.live = false;
        let Some(f) = frame else {
            w.picture.clear();
            self.report();
            return;
        };
        match write_png(&self.dir, w.seq, &f.pixels, (f.width, f.height), f.opaque) {
            Some(path) => {
                // Live, it was compared byte for byte and never hashed: hashed
                // now, so the same frame coming again is not written twice.
                w.pixels = fingerprint(&f.pixels, (f.width, f.height));
                w.version += 1;
                w.picture = format!("{}?{}", path.display(), w.version);
            }
            None => w.picture.clear(),
        }
        self.report();
    }

    /// How long the loop may sleep: until the soonest copy is due, and never
    /// more than a tenth of a second (what the logic wants is looked at on
    /// the way round).
    fn wait(&self) -> i32 {
        let now = Instant::now();
        self.windows
            .values()
            .filter_map(|w| w.capture.as_ref())
            .filter(|c| c.frame.is_none() && c.size.is_some())
            .map(|c| c.next.saturating_duration_since(now).as_millis() as i32)
            .fold(100, i32::min)
    }

    /// Each round: copies started and stopped as wanted, and frames asked for
    /// when it is their time.
    fn tick(&mut self, qh: &QueueHandle<State>) {
        let (Some(sources), Some(copies), Some(shm)) = (self.sources.clone(), self.copies.clone(), self.shm.clone()) else { return };
        let keys: Vec<u32> = self.windows.keys().copied().collect();
        for k in keys {
            if self.windows[&k].live && !self.lives(&self.windows[&k]) {
                self.settle(k);
            }
            let want = { let w = &self.windows[&k]; !w.id.is_empty() && self.wants(w) };
            let w = self.windows.get_mut(&k).unwrap();
            if !want {
                w.capture = None;
                continue;
            }
            let handle = w.handle.clone();
            let c = w.capture.get_or_insert_with(|| {
                let source = sources.create_source(&handle, qh, ());
                let session = copies.create_session(&source, copy_manager::Options::empty(), qh, k);
                Capture { session, _source: source, size: None, formats: Vec::new(), shm: None, frame: None, next: Instant::now() }
            });
            // One on its way: the compositor holds it until the window changes.
            if c.frame.is_some() {
                continue;
            }
            let Some(size) = c.size else { continue };
            if Instant::now() < c.next {
                continue;
            }
            // Its buffer, again if the size changed; ARGB if it is offered (a
            // window may be see-through), XRGB if not.
            if c.shm.as_ref().is_none_or(|s| s.size != size) {
                let format = if c.formats.contains(&wl_shm::Format::Argb8888) { wl_shm::Format::Argb8888 } else { wl_shm::Format::Xrgb8888 };
                c.shm = Shm::new(&shm, size, format, qh);
            }
            let Some(s) = &c.shm else { continue };
            let frame = c.session.create_frame(qh, k);
            frame.attach_buffer(&s.buffer);
            frame.damage_buffer(0, 0, size.0 as i32, size.1 as i32);
            frame.capture();
            c.frame = Some(frame);
        }
    }

    /// A frame came: kept for the renderer if the window is live; if not,
    /// made small, written. And told.
    fn taken(&mut self, k: u32) {
        let live = match self.windows.get(&k) {
            Some(w) => self.lives(w),
            None => return,
        };
        let Some(w) = self.windows.get_mut(&k) else { return };
        let Some(c) = &mut w.capture else { return };
        if let Some(f) = c.frame.take() {
            f.destroy();
        }
        c.next = Instant::now() + if live { LIVE_EVERY } else { EVERY };
        let Some(s) = &c.shm else { return };
        let (pw, ph) = s.size;
        // SAFETY: our own mapping, `len` bytes, filled by the compositor before `ready`.
        let px = unsafe { std::slice::from_raw_parts(s.map, s.len) };
        // The same pixels, kept the same way, are nothing new. Kept the other
        // way —the window went live, or stopped being live—, they are. A live
        // window's are compared with the frame kept for it, byte for byte: a
        // hash reads the whole frame at a fraction of the speed of a compare,
        // and a live window is looked at up to 30 times a second.
        let (same, pixels) = if live {
            let kept = w.live.then(|| FRAMES.lock().unwrap().as_ref().and_then(|f| f.get(&w.seq).cloned())).flatten();
            (kept.is_some_and(|f| (f.width, f.height) == s.size && f.pixels[..] == px[..]), 0)
        } else {
            let pixels = fingerprint(px, s.size);
            (pixels == w.pixels && w.version > 0 && !w.live, pixels)
        };
        if same {
            // The same picture again: nothing to write, and only `stale` to undo.
            if w.stale {
                w.stale = false;
                self.report();
            }
            return;
        }
        let opaque = s.format == wl_shm::Format::Xrgb8888;
        if live {
            // One copy, out of the buffer the compositor fills again next time;
            // made small where it is drawn, to the size it is drawn at.
            w.version += 1;
            let frame = Frame { version: w.version, width: pw, height: ph, pixels: px.to_vec(), opaque };
            FRAMES.lock().unwrap().get_or_insert_with(HashMap::new).insert(w.seq, Arc::new(frame));
            if !w.live {
                let _ = std::fs::remove_file(self.dir.join(format!("{}.png", w.seq)));
            }
            w.live = true;
            w.pixels = pixels;
            w.picture = format!("thumbnails:{}?{}", w.seq, w.version);
            w.stale = false;
            self.report();
            return;
        }
        let Some(path) = write_png(&self.dir, w.seq, px, s.size, opaque) else { return };
        w.version += 1;
        if w.live {
            forget_frame(w.seq);
        }
        w.live = false;
        w.pixels = pixels;
        w.picture = format!("{}?{}", path.display(), w.version);
        w.stale = false;
        self.report();
    }
}

/// What tells two pictures apart, quickly: a window's frame is megabytes, and
/// all of them come again whenever the monitor redraws.
fn fingerprint(px: &[u8], (w, h): (u32, u32)) -> u64 {
    let mut f = ((w as u64) << 32 | h as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let mut words = px.chunks_exact(8);
    for c in &mut words {
        f = (f.rotate_left(5) ^ u64::from_le_bytes(c.try_into().unwrap())).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
    for &b in words.remainder() {
        f = (f.rotate_left(5) ^ b as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
    f
}

/// A frame, made small (`SIDE` at most) and written as `<seq>.png` in `dir`:
/// BGRA premultiplied (ARGB8888 in memory) to RGBA as a PNG wants it.
fn write_png(dir: &std::path::Path, seq: u64, px: &[u8], (pw, ph): (u32, u32), opaque: bool) -> Option<std::path::PathBuf> {
    let mut rgba = Vec::with_capacity(px.len());
    for p in px.chunks_exact(4) {
        let a = if opaque { 255 } else { p[3] };
        let un = |c: u8| if a == 0 || a == 255 { c } else { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 };
        rgba.extend_from_slice(&[un(p[2]), un(p[1]), un(p[0]), a]);
    }
    let full = image::RgbaImage::from_raw(pw, ph, rgba)?;
    let k_side = (SIDE as f32 / pw.max(ph) as f32).min(1.0);
    let (tw, th) = (((pw as f32 * k_side).round() as u32).max(1), ((ph as f32 * k_side).round() as u32).max(1));
    let small = if (tw, th) == (pw, ph) { full } else { image::imageops::thumbnail(&full, tw, th) };
    let path = dir.join(format!("{seq}.png"));
    let tmp = dir.join(format!("{seq}.png.new"));
    if small.save_with_format(&tmp, image::ImageFormat::Png).is_err() || std::fs::rename(&tmp, &path).is_err() {
        return None;
    }
    Some(path)
}

static CONTROL: OnceLock<(Connection, Arc<Mutex<State>>)> = OnceLock::new();

/// Starts listing the windows (once), and tells `dispatch` the list each time it changes.
pub fn service(dispatch: Box<dyn Fn(SysValue) + Send>) -> bool {
    let Some(state) = start() else { return false };
    let mut e = state.lock().unwrap();
    e.dispatch.push(dispatch);
    // The newcomer is told now; the others hear the same list again, which is nothing new to them.
    e.last = None;
    e.report();
    true
}

/// `thumbnails.want`: which windows to copy, by their `id`s (a list), `"all"`,
/// or none. `thumbnails.live`, the same, for the ones to copy live.
pub fn command(name: &str, args: &[SysValue]) -> Result<(), String> {
    if name != "thumbnails.want" && name != "thumbnails.live" {
        return Err(format!("the thumbnails cannot do '{name}': only thumbnails.want and thumbnails.live"));
    }
    let wanted = match args {
        [] | [SysValue::Null] => Wanted::None,
        [SysValue::Text(t)] if t == "all" => Wanted::All,
        [SysValue::Text(t)] => Wanted::These(HashSet::from([t.clone()])),
        [SysValue::List(ids)] => Wanted::These(ids.iter().filter_map(|v| if let SysValue::Text(t) = v { Some(t.clone()) } else { None }).collect()),
        _ => return Err(format!("{name} takes a list of window ids, \"all\", or nothing")),
    };
    let state = start().ok_or("this compositor does not list its windows (ext-foreign-toplevel-list)")?;
    let mut e = state.lock().unwrap();
    if name == "thumbnails.live" {
        e.live = wanted;
    } else {
        e.wanted = wanted;
    }
    Ok(())
}

fn start() -> Option<Arc<Mutex<State>>> {
    if let Some((_, s)) = CONTROL.get() {
        return Some(s.clone());
    }
    let connection = Connection::connect_to_env().ok()?;
    let mut queue = connection.new_event_queue::<State>();
    let qh = queue.handle();
    connection.display().get_registry(&qh, ());
    // A folder per process; those of processes already gone, out (a shell
    // started again leaves its last pictures behind).
    let root = std::path::PathBuf::from(std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned())).join("pleamar").join("thumbnails");
    if let Ok(old) = std::fs::read_dir(&root) {
        for d in old.filter_map(Result::ok) {
            let pid = d.file_name().to_string_lossy().parse::<u32>().ok();
            if pid.is_some_and(|p| !std::path::Path::new(&format!("/proc/{p}")).exists()) {
                let _ = std::fs::remove_dir_all(d.path());
            }
        }
    }
    let dir = root.join(std::process::id().to_string());
    let _ = std::fs::create_dir_all(&dir);
    let mut state = State { dir, ..State::default() };
    queue.roundtrip(&mut state).ok()?;
    queue.roundtrip(&mut state).ok()?;
    // Without a list of windows there is nothing to say; without the copies,
    // the list all the same, with no pictures.
    state.list.as_ref()?;
    let state = Arc::new(Mutex::new(state));
    let _ = CONTROL.set((connection.clone(), state.clone()));
    let shared = state.clone();
    std::thread::Builder::new()
        .name("thumbnails".into())
        .spawn(move || loop {
            let wait = {
                let mut e = shared.lock().unwrap();
                if queue.dispatch_pending(&mut e).is_err() {
                    return;
                }
                e.tick(&qh);
                e.wait()
            };
            let _ = connection.flush();
            let Some(guard) = queue.prepare_read() else { continue };
            {
                use std::os::fd::AsRawFd;
                let mut fd = libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
                // Until the next copy is due, a tenth of a second at most.
                // SAFETY: one pollfd of our own, for as long as the call.
                unsafe { libc::poll(&mut fd, 1, wait) };
            }
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return,
            }
        })
        .ok()?;
    Some(state)
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(e: &mut Self, registry: &wl_registry::WlRegistry, ev: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, .. } = ev {
            match interface.as_str() {
                "ext_foreign_toplevel_list_v1" => e.list = Some(registry.bind(name, 1, qh, ())),
                "ext_foreign_toplevel_image_capture_source_manager_v1" => e.sources = Some(registry.bind(name, 1, qh, ())),
                "ext_image_copy_capture_manager_v1" => e.copies = Some(registry.bind(name, 1, qh, ())),
                "wl_shm" => e.shm = Some(registry.bind(name, 1, qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for State {
    fn event(e: &mut Self, _: &ExtForeignToplevelListV1, ev: toplevel_list::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let toplevel_list::Event::Toplevel { toplevel } = ev {
            e.seq += 1;
            e.windows.insert(key(&toplevel), Window { handle: toplevel, seq: e.seq, id: String::new(), title: String::new(), app: String::new(), picture: String::new(), version: 0, pixels: 0, live: false, stale: false, capture: None });
        }
    }

    wayland_client::event_created_child!(State, ExtForeignToplevelListV1, [
        toplevel_list::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for State {
    fn event(e: &mut Self, h: &ExtForeignToplevelHandleV1, ev: toplevel::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let k = key(h);
        match ev {
            toplevel::Event::Identifier { identifier } => {
                if let Some(w) = e.windows.get_mut(&k) {
                    w.id = identifier;
                }
            }
            toplevel::Event::Title { title } => {
                if let Some(w) = e.windows.get_mut(&k) {
                    w.title = title;
                }
            }
            toplevel::Event::AppId { app_id } => {
                if let Some(w) = e.windows.get_mut(&k) {
                    w.app = app_id;
                }
            }
            toplevel::Event::Done => e.report(),
            toplevel::Event::Closed => {
                if let Some(w) = e.windows.remove(&k) {
                    let _ = std::fs::remove_file(e.dir.join(format!("{}.png", w.seq)));
                    forget_frame(w.seq);
                    w.handle.destroy();
                }
                e.report();
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, u32> for State {
    fn event(e: &mut Self, _: &ExtImageCopyCaptureSessionV1, ev: copy_session::Event, k: &u32, _: &Connection, _: &QueueHandle<Self>) {
        let Some(c) = e.windows.get_mut(k).and_then(|w| w.capture.as_mut()) else { return };
        match ev {
            copy_session::Event::BufferSize { width, height } => {
                c.size = Some((width, height));
                c.formats.clear();
            }
            copy_session::Event::ShmFormat { format } => {
                if let wayland_client::WEnum::Value(f) = format {
                    c.formats.push(f);
                }
            }
            // The window is no longer copied (it closed, or the compositor
            // stopped): its last picture stays, and a new session is made if
            // it is still wanted.
            copy_session::Event::Stopped => {
                if let Some(w) = e.windows.get_mut(k) {
                    w.capture = None;
                    w.stale = true;
                }
                e.report();
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, u32> for State {
    fn event(e: &mut Self, _: &ExtImageCopyCaptureFrameV1, ev: copy_frame::Event, k: &u32, _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            copy_frame::Event::Ready => e.taken(*k),
            copy_frame::Event::Failed { .. } => {
                // Another size coming (it says so in the session), or a frame
                // lost: asked for again a little later.
                if let Some(c) = e.windows.get_mut(k).and_then(|w| w.capture.as_mut()) {
                    if let Some(f) = c.frame.take() {
                        f.destroy();
                    }
                    c.next = Instant::now() + EVERY;
                }
            }
            _ => {}
        }
    }
}

macro_rules! ignore {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for State {
            fn event(_: &mut Self, _: &$t, _: <$t as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
ignore!(ExtForeignToplevelImageCaptureSourceManagerV1, ExtImageCaptureSourceV1, ExtImageCopyCaptureManagerV1, wl_shm::WlShm, wl_shm_pool::WlShmPool, wl_buffer::WlBuffer);
