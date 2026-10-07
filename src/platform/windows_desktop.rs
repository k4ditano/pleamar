//! Desktop tools use window identities, not model-supplied HWNDs. All work is
//! serialized by Luau's desktop service worker and expires with that worker.
use super::SysValue;
use std::{cell::RefCell, collections::HashMap, mem::size_of, sync::atomic::{AtomicU64, Ordering}, time::{Duration, Instant}};
use windows::{core::BOOL, Win32::{Foundation::*, Graphics::{Dwm::*, Gdi::*}, System::Threading::*, UI::{Accessibility::*, HiDpi::*, WindowsAndMessaging::*}}};

#[path = "windows_desktop_capture.rs"]
mod capture;
pub(crate) use capture::helper as capture_helper;
#[path = "windows_desktop_input.rs"]
mod input;
#[path = "windows_desktop_move.rs"]
mod move_window;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity { hwnd: isize, process: u32, thread: u32 }
impl Identity {
    fn window(self) -> HWND { HWND(self.hwnd as _) }
    fn current(self) -> bool {
        let mut pid = 0;
        let tid = unsafe { GetWindowThreadProcessId(self.window(), Some(&mut pid)) };
        tid != 0 && self.thread == tid && self.process == pid
    }
}
#[derive(Clone)]
struct Entry { identity: Identity, owner: isize, title: String, program: String, rect: RECT, monitor: String, minimized: bool }
struct Shot { target: Identity, rect: RECT, created: Instant, epoch: u64 }
static EPOCH: AtomicU64 = AtomicU64::new(1);
#[derive(Default)]
struct Catalog { entries: HashMap<String, Entry>, shots: HashMap<String, Shot>, monitors: Vec<Monitor>, hook: Option<Hook>, exact_targets: bool }
struct Hook(HWINEVENTHOOK);
impl Drop for Hook { fn drop(&mut self) { unsafe { let _ = UnhookWinEvent(self.0); } } }
thread_local! {
    static CATALOG: RefCell<Catalog> = RefCell::default();
    static DESTROYED: RefCell<Vec<isize>> = RefCell::default();
}
pub(crate) fn has_thread_state() -> bool {
    CATALOG.with(|catalog| { let catalog = catalog.borrow(); !catalog.entries.is_empty() || !catalog.shots.is_empty() || !catalog.monitors.is_empty() || catalog.hook.is_some() })
}
struct Dpi(DPI_AWARENESS_CONTEXT);
impl Dpi {
    fn physical() -> Result<Self, String> {
        let prior = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if prior.0.is_null() { Err("cannot establish physical desktop coordinates".into()) } else { Ok(Self(prior)) }
    }
}
impl Drop for Dpi { fn drop(&mut self) { unsafe { SetThreadDpiAwarenessContext(self.0); } } }
fn active() -> bool {
    super::windows_capture::service_lifetime().is_none_or(|v| v.load(Ordering::Acquire))
}
fn check_active() -> Result<(), String> { if active() { Ok(()) } else { Err("desktop operation cancelled by reload".into()) } }

pub(crate) fn read_only_picture(handle: usize, process: u32, thread: u32) -> Result<SysValue, String> {
    check_active()?;
    let _dpi = Dpi::physical()?;
    let identity = Identity { hwnd: handle as isize, process, thread };
    let hwnd = identity.window();
    let visible = || identity.current() && unsafe { IsWindowVisible(hwnd).as_bool()
        && !IsIconic(hwnd).as_bool() && GetAncestor(hwnd, GA_ROOT) == hwnd };
    if process == 0 || thread == 0 || !visible() { return Err("capture needs a current visible top-level window".into()); }
    let mut cloaked = 0u32;
    unsafe { DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as _, size_of::<u32>() as u32) }
        .map_err(|e| e.to_string())?;
    if cloaked != 0 { return Err("the capture window is cloaked".into()); }
    let frame = bounds(hwnd)?;
    let picture = capture::window(hwnd, frame)?;
    check_active()?;
    if !visible() || bounds(hwnd)? != frame { return Err("window changed during capture; look again".into()); }
    Ok(picture)
}

unsafe extern "system" fn destroyed(_: HWINEVENTHOOK, _: u32, hwnd: HWND, object: i32, child: i32, _: u32, _: u32) {
    if object != OBJID_WINDOW.0 || child != 0 { return; }
    // Win32/COM calls can reenter a hook. Defer catalog mutations until after
    // the message pump, so a callback never borrows an in-use catalog.
    DESTROYED.with(|pending| { let mut pending = pending.borrow_mut(); if pending.len() < 4096 { pending.push(hwnd.0 as isize); } });
}
fn prepare() -> Result<Dpi, String> {
    check_active()?;
    let dpi = Dpi::physical()?;
    CATALOG.with(|c| {
        let mut c = c.borrow_mut();
        if c.hook.is_none() {
            let hook = unsafe { SetWinEventHook(EVENT_OBJECT_DESTROY, EVENT_OBJECT_DESTROY, None, Some(destroyed), 0, 0, WINEVENT_OUTOFCONTEXT) };
            if hook.is_invalid() { return Err("could not watch desktop window lifetimes".to_owned()); }
            c.hook = Some(Hook(hook));
        }
        Ok(())
    })?;
    pump();
    Ok(dpi)
}
fn pump() {
    unsafe {
        let mut message = MSG::default();
        for _ in 0..256 {
            if !PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() { break; }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    let destroyed = DESTROYED.with(|pending| std::mem::take(&mut *pending.borrow_mut()));
    CATALOG.with(|c| {
        let mut c = c.borrow_mut();
        if destroyed.len() >= 4096 { c.entries.clear(); c.shots.clear(); }
        else {
            c.entries.retain(|_, e| !destroyed.contains(&e.identity.hwnd));
            c.shots.retain(|_, s| !destroyed.contains(&s.target.hwnd));
        }
    });
}
fn caption(hwnd: HWND) -> String {
    let mut out = [0u16; 2048];
    let length = unsafe { GetWindowTextW(hwnd, &mut out) }.max(0) as usize;
    String::from_utf16_lossy(&out[..length])
}
fn clean(text: &str) -> String { text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect() }
fn bounds(hwnd: HWND) -> Result<RECT, String> {
    let mut rect = RECT::default();
    unsafe { DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut rect as *mut _ as _, size_of::<RECT>() as u32) }.map_err(|e| e.to_string())?;
    if rect.right <= rect.left || rect.bottom <= rect.top { return Err("the window has no drawable area".into()); }
    Ok(rect)
}
fn program(pid: u32) -> String {
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return format!("process-{pid}"); };
        let mut buffer = vec![0u16; 32768];
        let mut len = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, windows::core::PWSTR(buffer.as_mut_ptr()), &mut len);
        let _ = CloseHandle(handle);
        if result.is_err() { return format!("process-{pid}"); }
        std::path::Path::new(&String::from_utf16_lossy(&buffer[..len as usize])).file_stem().unwrap_or_default().to_string_lossy().into_owned()
    }
}
#[derive(Clone)]
struct Monitor { name: String, rect: RECT, work: RECT, primary: bool }
fn monitor_info(handle: HMONITOR) -> Option<Monitor> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if !unsafe { GetMonitorInfoW(handle, &mut info.monitorInfo) }.as_bool() { return None; }
    let length = info.szDevice.iter().position(|v| *v == 0).unwrap_or(info.szDevice.len());
    Some(Monitor { name: String::from_utf16_lossy(&info.szDevice[..length]), rect: info.monitorInfo.rcMonitor,
        work: info.monitorInfo.rcWork, primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0 })
}
fn monitors() -> Result<Vec<Monitor>, String> {
    unsafe extern "system" fn visit(handle: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL { unsafe {
        if let Some(m) = monitor_info(handle) { (*(data.0 as *mut Vec<Monitor>)).push(m); }
        true.into()
    } }
    let mut list = Vec::new();
    if !unsafe { EnumDisplayMonitors(None, None, Some(visit), LPARAM(&mut list as *mut _ as isize)) }.as_bool() { return Err("could not enumerate the desktop monitors".into()); }
    list.sort_by(|a: &Monitor, b| a.name.cmp(&b.name));
    Ok(list)
}
fn enumerate() -> Result<Vec<Entry>, String> {
    struct Found { entries: Vec<Entry>, started: Instant, truncated: bool }
    unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> BOOL { unsafe {
        let found = &mut *(data.0 as *mut Found);
        if found.entries.len() >= 512 || found.started.elapsed() > Duration::from_secs(1) { found.truncated = true; return false.into(); }
        let mut pid = 0;
        let thread = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if thread == 0 || pid == GetCurrentProcessId() || !IsWindowVisible(hwnd).as_bool() { return true.into(); }
        let mut cloaked = 0u32;
        if DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as _, size_of::<u32>() as u32).is_err() || cloaked != 0 { return true.into(); }
        let mut class = [0u16; 128];
        let count = GetClassNameW(hwnd, &mut class).max(0) as usize;
        if matches!(String::from_utf16_lossy(&class[..count]).as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") { return true.into(); }
        let owner = GetWindow(hwnd, GW_OWNER).unwrap_or_default().0 as isize;
        let style = WINDOW_EX_STYLE(GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32);
        if style.contains(WS_EX_NOACTIVATE) || (style.contains(WS_EX_TOOLWINDOW) && owner == 0) { return true.into(); }
        let title = caption(hwnd);
        if title.is_empty() { return true.into(); }
        let Ok(rect) = bounds(hwnd) else { return true.into(); };
        found.entries.push(Entry { identity: Identity { hwnd: hwnd.0 as isize, process: pid, thread }, owner,
            title, program: String::new(), rect, monitor: monitor_info(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL)).map(|m| m.name).unwrap_or_default(),
            minimized: IsIconic(hwnd).as_bool() });
        true.into()
    } }
    let mut found = Found { entries: Vec::new(), started: Instant::now(), truncated: false };
    let result = unsafe { EnumWindows(Some(visit), LPARAM(&mut found as *mut _ as isize)) };
    if found.truncated { return Err("too many windows or desktop enumeration timed out; no partial catalog was used".into()); }
    result.map_err(|e| e.to_string())?;
    let mut names = HashMap::new();
    for e in &mut found.entries { e.program = names.entry(e.identity.process).or_insert_with(|| program(e.identity.process)).clone(); }
    Ok(found.entries)
}
fn refresh() -> Result<(), String> {
    let entries = enumerate()?;
    pump();
    CATALOG.with(|c| {
        let mut c = c.borrow_mut();
        let old = std::mem::take(&mut c.entries);
        for entry in entries {
            if !entry.identity.current() { continue; }
            let id = old.iter().find(|(_, e)| e.identity == entry.identity).map(|(id, _)| id.clone()).unwrap_or_else(|| {
                static NEXT: AtomicU64 = AtomicU64::new(1);
                NEXT.fetch_add(1, Ordering::Relaxed).to_string()
            });
            c.entries.insert(id, entry);
        }
        let alive: Vec<_> = c.entries.values().map(|e| e.identity).collect();
        c.shots.retain(|_, s| alive.contains(&s.target) && s.created.elapsed() < Duration::from_secs(30));
    });
    Ok(())
}
fn id(value: &SysValue) -> Result<String, String> {
    match value {
        SysValue::Text(v) if !v.is_empty() && v.len() <= 20 && v.bytes().all(|c| c.is_ascii_digit()) => Ok(v.clone()),
        SysValue::Num(v) if v.is_finite() && v.fract() == 0.0 && *v >= 1.0 && *v < 9007199254740992.0 => Ok(format!("{v:.0}")),
        _ => Err("use a window id from desktop.windows".into()),
    }
}
fn target(id: &str) -> Result<Entry, String> {
    CATALOG.with(|c| {
        let c = c.borrow();
        let entry = c.entries.get(id).ok_or("the window is no longer in this desktop catalog; list windows again")?;
        if !entry.identity.current() { return Err("the window closed or its handle was reused".into()); }
        if c.exact_targets && !unsafe { windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(entry.identity.window()) }.as_bool() {
            return Err("the selected window is blocked by a dialog; companion input never changes targets implicitly".into());
        }
        let mut entry = entry.clone();
        // Follow a disabled owner's active modal dialog. An unrelated popup
        // never becomes an input target merely because it is in front.
        for _ in 0..8 {
            if unsafe { windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(entry.identity.window()) }.as_bool() { break; }
            let popup = unsafe { GetLastActivePopup(entry.identity.window()) };
            let owned: Vec<_> = c.entries.values().filter(|e| e.owner == entry.identity.hwnd && e.identity.current()
                && unsafe { windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(e.identity.window()) }.as_bool()).collect();
            let dialog = owned.iter().find(|e| e.identity.window() == popup).copied()
                .or_else(|| if owned.len() == 1 { Some(owned[0]) } else { None })
                .ok_or("a new or ambiguous modal dialog needs desktop.windows and desktop.look first")?;
            entry = dialog.clone();
        }
        if !entry.identity.current() || !unsafe { IsWindowVisible(entry.identity.window()) }.as_bool() { return Err("the target is no longer visible".into()); }
        if unsafe { IsIconic(entry.identity.window()) }.as_bool() { return Err("the target is minimized; focus it before looking or acting".into()); }
        entry.rect = bounds(entry.identity.window())?;
        Ok(entry)
    })
}
fn rect_values(r: RECT) -> SysValue { SysValue::Map(vec![
    ("x".into(), SysValue::Num(r.left as f64)), ("y".into(), SysValue::Num(r.top as f64)),
    ("width".into(), SysValue::Num((r.right-r.left) as f64)), ("height".into(), SysValue::Num((r.bottom-r.top) as f64))]) }

pub(crate) fn validate_exact_target(id: &str) -> Result<(), String> {
    let _dpi = prepare()?;
    let entry = target(id)?;
    // Companion focus is shared input too: respect Escape and held input
    // before attempting activation, as the other desktop actions already do.
    input::idle_keyboard(&entry, EPOCH.load(Ordering::Acquire))
}

pub(crate) fn exact_targets(enabled: bool) { CATALOG.with(|c| c.borrow_mut().exact_targets = enabled); }

pub(crate) fn clear_catalog() {
    CATALOG.with(|c| { let mut c = c.borrow_mut(); c.shots.clear(); c.entries.clear(); c.monitors.clear(); c.hook.take(); });
}

pub(crate) fn catalog_id(hwnd: usize, process: u32, thread: u32) -> Result<String, String> {
    let _dpi = prepare()?;
    CATALOG.with(|c| c.borrow().entries.iter()
        .find(|(_,e)|e.identity.hwnd as usize == hwnd && e.identity.process == process
            && e.identity.thread == thread && e.identity.current())
        .map(|(id,_)|id.clone()).ok_or_else(||"window is absent from the current desktop catalog".into()))
}

pub fn query(name: &str, args: &[SysValue]) -> Result<SysValue, String> {
    let _dpi = prepare()?;
    match (name, args) {
        ("desktop.windows", []) => {
            refresh()?;
            let screens = monitors()?;
            CATALOG.with(|catalog| catalog.borrow_mut().monitors = screens.clone());
            let list = CATALOG.with(|c| {
                let c = c.borrow();
                let mut list: Vec<_> = c.entries.iter().collect();
                list.sort_by_key(|(id, _)| id.parse::<u64>().unwrap_or_default());
                list.into_iter().map(|(id, e)| SysValue::Map(vec![
                    ("id".into(), SysValue::Text(id.clone())), ("process".into(), SysValue::Num(e.identity.process as f64)),
                    ("program".into(), SysValue::Text(clean(&e.program))), ("title".into(), SysValue::Text(clean(&e.title))),
                    ("box".into(), rect_values(e.rect)), ("monitor".into(), SysValue::Text(e.monitor.clone())),
                    ("minimized".into(), SysValue::Bool(e.minimized)),
                    ("focused".into(), SysValue::Bool(unsafe { GetForegroundWindow() } == e.identity.window())),
                    ("dialogof".into(), SysValue::Text(c.entries.iter().find(|(_, p)| p.identity.hwnd == e.owner).map(|(id, _)| id.clone()).unwrap_or_default())),
                ])).collect()
            });
            Ok(SysValue::Map(vec![("epoch".into(), SysValue::Num(EPOCH.load(Ordering::Acquire) as f64)), ("windows".into(), SysValue::List(list)), ("input".into(), SysValue::Text("foreground".into())),
                ("monitors".into(), SysValue::List(screens.iter().enumerate().map(|(i, m)| SysValue::Map(vec![
                    ("id".into(), SysValue::Num(i as f64)), ("name".into(), SysValue::Text(m.name.clone())),
                    ("box".into(), rect_values(m.rect)), ("primary".into(), SysValue::Bool(m.primary))])).collect()))]))
        }
        ("desktop.look", [value]) => {
            let epoch = EPOCH.load(Ordering::Acquire);
            let id = id(value)?;
            CATALOG.with(|c| c.borrow_mut().shots.remove(&id));
            let target = target(&id)?;
            let image = capture::window(target.identity.window(), target.rect)?;
            pump();
            let current = self::target(&id)?;
            if current.identity != target.identity || current.rect != target.rect || epoch != EPOCH.load(Ordering::Acquire) { return Err("the window changed or capture was cancelled; look again".into()); }
            CATALOG.with(|c| c.borrow_mut().shots.insert(id, Shot { target: target.identity, rect: target.rect, created: Instant::now(), epoch }));
            Ok(image)
        }
        _ => Err("unknown desktop query or invalid arguments".into()),
    }
}
pub fn command(name: &str, args: &[SysValue]) -> Result<(), String> {
    // Called synchronously by the logic: constant time, no UI/COM calls. This
    // invalidates already queued work even while a capture blocks its worker.
    if name == "desktop.cancel" && args.is_empty() { EPOCH.fetch_add(1, Ordering::AcqRel); return Ok(()); }
    let _dpi = prepare()?;
    if name == "desktop.done" && args.is_empty() {
        clear_catalog();
        return Ok(());
    }
    let (epoch, args) = args.split_first().ok_or("desktop action requires the catalog epoch")?;
    let epoch = id(epoch)?.parse::<u64>().map_err(|_| "invalid desktop epoch")?;
    check_epoch(epoch)?;
    input::command(name, args, epoch)
}
fn check_epoch(epoch: u64) -> Result<(), String> {
    check_active()?;
    if epoch != EPOCH.load(Ordering::Acquire) { Err("desktop action cancelled; list windows before continuing".into()) } else { Ok(()) }
}

pub fn type_secret(owner: &str, args: &[SysValue]) -> Result<(), String> {
    let [epoch, window, SysValue::Text(name)] = args else { return Err("desktop.type_secret takes epoch, window id and saved name".into()); };
    let _dpi = prepare()?;
    let epoch = id(epoch)?.parse::<u64>().map_err(|_| "invalid desktop epoch")?;
    check_epoch(epoch)?;
    super::windows_credentials::with_secret(owner, name, |value| input::type_secret(window, value, epoch))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn identifiers_are_catalog_values_not_arbitrary_handles() {
        for value in [SysValue::Text("-1".into()), SysValue::Text("1.2".into()), SysValue::Num(f64::NAN), SysValue::Num(0.1), SysValue::Num(9007199254740992.0)] { assert!(id(&value).is_err()); }
        assert_eq!(id(&SysValue::Num(27.0)).unwrap(), "27");
        assert_eq!(clean("app\r\nspoof\0"), "app  spoof ");
    }
}

#[cfg(test)]
#[path = "windows_desktop_tests.rs"]
mod native_tests;

#[cfg(test)]
#[path = "windows_scene_capture_tests.rs"]
mod scene_tests;

#[cfg(test)]
#[path = "windows_capture_tests.rs"]
mod capture_tests;

#[cfg(test)]
#[path = "windows_desktop_ci_tests.rs"]
mod ci_input_tests;
