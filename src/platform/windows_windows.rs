//! Application windows for the shell's minimized-window shelf.
use super::SysValue;
use std::{collections::HashMap, mem::size_of, sync::{Mutex, OnceLock}, time::{Duration, Instant}};
use windows::Win32::{Foundation::*, Graphics::{Dwm::*, Gdi::*}, System::Threading::*, UI::WindowsAndMessaging::*};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Identity { handle: isize, process: u32, thread: u32 }
impl Identity {
    fn hwnd(self) -> HWND { HWND(self.handle as _) }
    fn current(self) -> bool {
        let mut process = 0;
        let thread = unsafe { GetWindowThreadProcessId(self.hwnd(), Some(&mut process)) };
        process == self.process && thread == self.thread && thread != 0
    }
}
#[derive(Default)]
struct Registry { next: u32, entries: HashMap<u32, Identity>, icons: HashMap<Identity, String> }
impl Registry {
    fn update(&mut self, windows: &[Identity]) -> Vec<u32> {
        let previous: HashMap<_, _> = self.entries.iter().map(|(id, window)| (*window, *id)).collect();
        self.icons.retain(|window, _| windows.contains(window));
        self.entries.clear();
        windows.iter().map(|window| {
            let id = previous.get(window).copied().unwrap_or_else(|| {
                self.next = self.next.checked_add(1).expect("window identifiers exhausted");
                self.next
            });
            self.entries.insert(id, *window);
            id
        }).collect()
    }
    fn get(&self, id: &SysValue) -> Result<Identity, String> {
        let SysValue::Num(id) = id else { return Err("window.restore requires an identifier from window.state".into()); };
        if !id.is_finite() || id.fract() != 0.0 || *id < 1.0 || *id > u32::MAX as f64 { return Err("invalid window identifier".into()); }
        self.entries.get(&(*id as u32)).copied().ok_or("that window is no longer in the desktop catalog".into())
    }
}
fn registry() -> &'static Mutex<Registry> { static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new(); REGISTRY.get_or_init(Mutex::default) }
fn eligible(visible: bool, cloaked: bool, own: bool, owned: bool, style: WINDOW_EX_STYLE, class: &str) -> bool {
    visible && !cloaked && !own
        && !matches!(class, "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
        && (style.contains(WS_EX_APPWINDOW) || (!owned && !style.contains(WS_EX_TOOLWINDOW)))
}
fn title(hwnd: HWND) -> String {
    let mut value = [0u16; 4096];
    let length = unsafe { GetWindowTextW(hwnd, &mut value) }.max(0) as usize;
    String::from_utf16_lossy(&value[..length])
}
fn class(hwnd: HWND) -> String {
    let mut value = [0u16; 256];
    let length = unsafe { GetClassNameW(hwnd, &mut value) }.max(0) as usize;
    String::from_utf16_lossy(&value[..length])
}
fn monitor(hwnd: HWND) -> String {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    let handle = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if !unsafe { GetMonitorInfoW(handle, &mut info.monitorInfo) }.as_bool() { return String::new(); }
    let length = info.szDevice.iter().position(|v| *v == 0).unwrap_or(info.szDevice.len());
    String::from_utf16_lossy(&info.szDevice[..length])
}
fn icon(process: u32) -> String {
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process) else { return String::new(); };
        let mut path = vec![0u16; 32768];
        let mut length = path.len() as u32;
        let result = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, windows::core::PWSTR(path.as_mut_ptr()), &mut length);
        let _ = CloseHandle(handle);
        if result.is_err() { return String::new(); }
        format!("windows-file:{}", String::from_utf16_lossy(&path[..length as usize]))
    }
}
struct Entry { identity: Identity, title: String, class: String, minimized: bool, monitor: String }
struct Enumeration { entries: Vec<Entry>, started: Instant, truncated: bool }
unsafe extern "system" fn enumerate(hwnd: HWND, data: LPARAM) -> windows::core::BOOL { unsafe {
    let result = &mut *(data.0 as *mut Enumeration);
    if result.entries.len() >= 1024 || result.started.elapsed() > Duration::from_millis(250) {
        result.truncated = true;
        return false.into();
    }
    let mut process = 0;
    let thread = GetWindowThreadProcessId(hwnd, Some(&mut process));
    // GetWindowText can dispatch synchronously within our own process. Skip it
    // before reading captions, so this worker never waits for our UI thread.
    if process == 0 || process == GetCurrentProcessId() || !IsWindowVisible(hwnd).as_bool() { return true.into(); }
    let style = WINDOW_EX_STYLE(GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32);
    let mut cloaked = 0u32;
    let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as _, size_of::<u32>() as u32);
    let class = class(hwnd);
    let owned = GetWindow(hwnd, GW_OWNER).is_ok_and(|owner| !owner.is_invalid());
    if !eligible(true, cloaked != 0, false, owned, style, &class) { return true.into(); }
    let title = title(hwnd);
    if title.is_empty() { return true.into(); }
    result.entries.push(Entry { identity: Identity { handle: hwnd.0 as isize, process, thread }, title, class,
        minimized: IsIconic(hwnd).as_bool(), monitor: monitor(hwnd) });
    true.into()
} }
pub fn read() -> Result<SysValue, String> {
    let mut enumeration = Enumeration { entries: Vec::new(), started: Instant::now(), truncated: false };
    let result = unsafe { EnumWindows(Some(enumerate), LPARAM(&mut enumeration as *mut _ as isize)) };
    if !enumeration.truncated { result.map_err(|e| e.to_string())?; }
    let mut catalog = registry().lock().unwrap();
    let ids = catalog.update(&enumeration.entries.iter().map(|e| e.identity).collect::<Vec<_>>());
    let foreground = unsafe { GetForegroundWindow() };
    let mut process_icons = HashMap::new();
    let list = enumeration.entries.into_iter().zip(ids).map(|(entry, id)| {
        let icon = if entry.minimized {
            catalog.icons.entry(entry.identity).or_insert_with(|| {
                process_icons.entry(entry.identity.process).or_insert_with(|| icon(entry.identity.process)).clone()
            }).clone()
        } else { String::new() };
        SysValue::Map(vec![
            ("id".into(), SysValue::Num(id as f64)), ("title".into(), SysValue::Text(entry.title)),
            ("class".into(), SysValue::Text(entry.class)), ("monitor".into(), SysValue::Text(entry.monitor)),
            ("minimized".into(), SysValue::Bool(entry.minimized)), ("focused".into(), SysValue::Bool(entry.identity.hwnd() == foreground)),
            ("icon".into(), SysValue::Text(icon)),
        ])
    }).collect();
    let mut process = 0;
    unsafe { GetWindowThreadProcessId(foreground, Some(&mut process)); }
    let own = process == unsafe { GetCurrentProcessId() };
    Ok(SysValue::Map(vec![
        ("title".into(), SysValue::Text(if own { String::new() } else { title(foreground) })),
        ("class".into(), SysValue::Text(class(foreground))), ("monitor".into(), SysValue::Text(monitor(foreground))),
        ("list".into(), SysValue::List(list)), ("truncated".into(), SysValue::Bool(enumeration.truncated)),
    ]))
}
pub fn restore(args: &[SysValue]) -> Result<(), String> {
    let [id] = args else { return Err("window.restore takes one catalog identifier".into()); };
    let identity = registry().lock().unwrap().get(id)?;
    if !identity.current() { return Err("the window closed or its handle was reused".into()); }
    let hwnd = identity.hwnd();
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() { return Err("the window is no longer visible in the desktop".into()); }
        if IsIconic(hwnd).as_bool() {
            if !ShowWindowAsync(hwnd, SW_RESTORE).as_bool() { return Err("Windows rejected the restore request".into()); }
            let started = Instant::now();
            while identity.current() && IsIconic(hwnd).as_bool() {
                if started.elapsed() > Duration::from_secs(3) { return Err("the application has not restored its window".into()); }
                std::thread::sleep(Duration::from_millis(20));
            }
            if !identity.current() { return Err("the window closed while it was being restored".into()); }
        }
        // Respect foreground-lock policy. Never attach to another app's input
        // queue or synthesize keyboard input to take focus from it.
        if !SetForegroundWindow(hwnd).as_bool() { return Err("window restored, but Windows kept keyboard focus with the current application".into()); }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_preserves_live_ids_but_expires_closed_and_reused_handles() {
        let a = Identity { handle: 100, process: 10, thread: 11 };
        let b = Identity { handle: 200, process: 20, thread: 21 };
        let mut catalog = Registry::default();
        let ids = catalog.update(&[a, b]);
        assert_eq!(catalog.update(&[b, a]), [ids[1], ids[0]]);
        catalog.update(&[b]);
        assert!(catalog.get(&SysValue::Num(ids[0] as f64)).is_err());
        let reused = Identity { process: 30, ..a };
        let new_id = catalog.update(&[b, reused])[1];
        assert_ne!(new_id, ids[0]);
        assert_eq!(catalog.get(&SysValue::Num(new_id as f64)).unwrap(), reused);
        for invalid in [f64::NAN, f64::INFINITY, -1.0, 0.0, 0.5, u32::MAX as f64 + 1.0] { assert!(catalog.get(&SysValue::Num(invalid)).is_err()); }
    }
    #[test]
    fn shelf_filters_shell_tool_owned_hidden_and_cloaked_windows() {
        assert!(eligible(true, false, false, false, WINDOW_EX_STYLE(0), "Normal"));
        assert!(!eligible(false, false, false, false, WINDOW_EX_STYLE(0), "Normal"));
        assert!(!eligible(true, true, false, false, WINDOW_EX_STYLE(0), "Normal"));
        assert!(!eligible(true, false, true, false, WINDOW_EX_STYLE(0), "Normal"));
        assert!(!eligible(true, false, false, true, WINDOW_EX_STYLE(0), "Normal"));
        assert!(!eligible(true, false, false, false, WS_EX_TOOLWINDOW, "Normal"));
        assert!(eligible(true, false, false, true, WS_EX_APPWINDOW, "Normal"));
        assert!(!eligible(true, false, false, false, WS_EX_APPWINDOW, "Shell_TrayWnd"));
    }
}
