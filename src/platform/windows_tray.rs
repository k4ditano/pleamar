//! Explorer's live notification icons. The registry is only a candidate list:
//! Shell_NotifyIconGetRect must confirm every entry before it is published.
//! No process injection, taskbar replacement or user preference changes.
use super::SysValue;
use std::{collections::{HashMap, HashSet}, mem::size_of, sync::{Mutex, OnceLock}, time::{Duration, Instant}};
use windows::{core::{w, GUID, PCWSTR, PWSTR}, Win32::{Foundation::*, System::{Com::CoTaskMemFree, Environment::ExpandEnvironmentStringsW, Registry::*, Threading::*}, UI::{Shell::*, WindowsAndMessaging::*}}};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Identity {
    pub guid: GUID,
    pub handle: isize,
    pub uid: u32,
    pub process: u32,
    pub thread: u32,
    pub explorer: u32,
}
impl Identity {
    pub fn rect(&self) -> Result<RECT, String> {
        let shell = explorer()?;
        if shell != self.explorer { return Err("Explorer restarted; refresh the tray".into()); }
        if self.guid == GUID::zeroed() {
            let mut process = 0;
            let thread = unsafe { GetWindowThreadProcessId(HWND(self.handle as _), Some(&mut process)) };
            if thread == 0 || thread != self.thread || process != self.process { return Err("the tray owner closed or its window was replaced".into()); }
        }
        unsafe { Shell_NotifyIconGetRect(&NOTIFYICONIDENTIFIER {
            cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32, hWnd: HWND(self.handle as _), uID: self.uid, guidItem: self.guid,
        }) }.map_err(|_| "the application removed its tray icon".into())
    }
}

struct Key(HKEY);
impl Drop for Key { fn drop(&mut self) { unsafe { let _ = RegCloseKey(self.0); } } }
impl Key {
    fn open(parent: HKEY, name: PCWSTR) -> windows::core::Result<Self> {
        let mut key = HKEY::default();
        unsafe { RegOpenKeyExW(parent, name, None, KEY_READ, &mut key).ok()?; }
        Ok(Self(key))
    }
    fn bytes(&self, name: PCWSTR, flags: REG_ROUTINE_FLAGS, limit: usize) -> Option<Vec<u8>> {
        let mut size = 0;
        unsafe { RegGetValueW(self.0, PCWSTR::null(), name, flags, None, None, Some(&mut size)).ok().ok()?; }
        if size == 0 || size as usize > limit { return None; }
        let mut bytes = vec![0u8; size as usize];
        unsafe { RegGetValueW(self.0, PCWSTR::null(), name, flags, None, Some(bytes.as_mut_ptr() as _), Some(&mut size)).ok().ok()?; }
        bytes.truncate(size as usize);
        Some(bytes)
    }
    fn text(&self, name: PCWSTR) -> Option<String> {
        let bytes = self.bytes(name, RRF_RT_REG_SZ, 65536)?;
        if bytes.len() % 2 != 0 { return None; }
        let value: Vec<_> = bytes.chunks_exact(2).map(|v| u16::from_le_bytes([v[0], v[1]])).collect();
        String::from_utf16(value.strip_suffix(&[0]).unwrap_or(&value)).ok()
    }
    fn number(&self, name: PCWSTR) -> Option<u32> { Some(u32::from_le_bytes(self.bytes(name, RRF_RT_REG_DWORD, 4)?.try_into().ok()?)) }
}

fn wide(value: &str) -> Vec<u16> { value.encode_utf16().chain([0]).collect() }
fn canonical(path: &str) -> String { path.replace('/', "\\").to_lowercase() }
fn executable(path: &str) -> Option<String> {
    let mut path = path.to_owned();
    if path.starts_with('{') {
        let end = path.find('}')?;
        let guid = GUID::try_from(&path[1..end]).ok()?;
        unsafe {
            let folder = SHGetKnownFolderPath(&guid, KF_FLAG_DEFAULT, None).ok()?;
            let decoded = folder.to_string();
            CoTaskMemFree(Some(folder.0 as _));
            path = decoded.ok()? + &path[end + 1..];
        }
    }
    let source = wide(&path);
    let mut expanded = vec![0u16; 32768];
    let count = unsafe { ExpandEnvironmentStringsW(PCWSTR(source.as_ptr()), Some(&mut expanded)) } as usize;
    if count == 0 || count > expanded.len() { return None; }
    Some(canonical(&String::from_utf16_lossy(&expanded[..count - 1])))
}

#[derive(Default)]
struct Windows { by_path: HashMap<String, Vec<(isize, u32, u32)>>, paths: HashMap<u32, String>, count: usize }
impl Windows {
    fn visit(&mut self, hwnd: HWND) {
        self.count += 1;
        let mut process = 0;
        let thread = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process)) };
        let path = self.paths.entry(process).or_insert_with(|| unsafe {
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process) else { return String::new(); };
            let mut buffer = vec![0u16; 32768];
            let mut count = buffer.len() as u32;
            let result = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut count);
            let _ = CloseHandle(handle);
            if result.is_err() { return String::new(); }
            canonical(&String::from_utf16_lossy(&buffer[..count as usize]))
        });
        if !path.is_empty() && thread != 0 { self.by_path.entry(path.clone()).or_default().push((hwnd.0 as isize, process, thread)); }
    }
}
unsafe extern "system" fn visit(hwnd: HWND, context: LPARAM) -> windows::core::BOOL {
    let windows = unsafe { &mut *(context.0 as *mut Windows) };
    if windows.count >= 4096 { return false.into(); }
    windows.visit(hwnd);
    true.into()
}
pub(super) fn explorer() -> Result<u32, String> {
    let hwnd = unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }.map_err(|_| "Explorer's taskbar is unavailable")?;
    let mut process = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process)); }
    if process == 0 { return Err("Explorer's taskbar is unavailable".into()); }
    Ok(process)
}

#[derive(Default)]
struct Catalog { next: u64, entries: HashMap<String, Identity>, latest: Vec<SysValue>, refreshed: Option<Instant>, error: Option<String> }
fn catalog() -> &'static Mutex<Catalog> { static VALUE: OnceLock<Mutex<Catalog>> = OnceLock::new(); VALUE.get_or_init(Mutex::default) }
impl Catalog {
    fn replace(&mut self, entries: Vec<(Identity, String, String, String)>) {
        let previous: HashMap<_, _> = self.entries.drain().map(|(key, id)| (id, key)).collect();
        let mut seen = HashSet::new();
        self.latest = entries.into_iter().filter_map(|(id, title, name, icon)| {
            if !seen.insert(id.clone()) { return None; }
            let key = previous.get(&id).cloned().unwrap_or_else(|| { self.next += 1; format!("windows-tray:{}", self.next) });
            self.entries.insert(key.clone(), id);
            Some(SysValue::Map(vec![
                ("key".into(), SysValue::Text(key)), ("id".into(), SysValue::Text(name)),
                ("title".into(), SysValue::Text(title)), ("icon".into(), SysValue::Text(icon)),
                ("status".into(), SysValue::Text("Active".into())), ("menu".into(), SysValue::Text(String::new())),
                ("cached_metadata".into(), SysValue::Bool(true)),
            ]))
        }).collect();
        self.refreshed = Some(Instant::now());
    }
}

fn snapshot(bytes: &[u8]) -> Option<String> {
    use std::hash::{Hash, Hasher};
    // Explorer owns this cache, but validate the image before trusting a size
    // or writing it to the runtime's cache. No arbitrary registry file paths.
    let image = image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
    let (width, height) = image.into_dimensions().ok()?;
    if width == 0 || height == 0 || width > 256 || height > 256 { return None; }
    let mut hash = std::hash::DefaultHasher::new(); bytes.hash(&mut hash);
    let folder = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("pleamar/tray-icons");
    let path = folder.join(format!("{:016x}.png", hash.finish()));
    if !path.is_file() {
        std::fs::create_dir_all(folder).ok()?;
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&temp, bytes).ok()?;
        if std::fs::rename(&temp, &path).is_err() { let _ = std::fs::remove_file(temp); }
    }
    path.is_file().then(|| path.to_string_lossy().into_owned())
}

fn application_name(path: &str, fallback: &str) -> String {
    static NAMES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let names = NAMES.get_or_init(Mutex::default);
    if let Some(name) = names.lock().unwrap().get(path).cloned() { return name; }
    // InitialTooltip is historical data, sometimes an obsolete download
    // percentage or login state. Never present it as the application's state.
    // System.FileDescription identifies the actual executable instead.
    let description = unsafe {
        let source = wide(path);
        let item: windows::core::Result<IShellItem2> = SHCreateItemFromParsingName(PCWSTR(source.as_ptr()), None);
        const DESCRIPTION: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0x0cef7d53_fa64_11d1_a203_0000f81fedee), pid: 3 };
        item.and_then(|item| item.GetString(&DESCRIPTION)).ok().and_then(|raw| {
            let value = raw.to_string().ok(); CoTaskMemFree(Some(raw.0 as _)); value
        }).filter(|v| !v.trim().is_empty() && v.len() <= 512)
    }.unwrap_or_else(|| fallback.to_owned());
    let mut names = names.lock().unwrap();
    if names.len() >= 4096 { names.clear(); }
    names.insert(path.to_owned(), description.clone());
    description
}

fn scan() -> Result<Vec<(Identity, String, String, String)>, String> {
    let _apartment = super::windows_system::Apartment::new()?;
    let explorer = explorer()?;
    let root = Key::open(HKEY_CURRENT_USER, w!("Control Panel\\NotifyIconSettings")).map_err(|e| format!("Explorer's tray catalog is unavailable: {e}"))?;
    let mut windows = Windows::default();
    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(&mut windows as *mut _ as isize));
        let mut previous = None;
        for _ in 0..2048 {
            let Ok(hwnd) = FindWindowExW(Some(HWND_MESSAGE), previous, PCWSTR::null(), PCWSTR::null()) else { break; };
            windows.visit(hwnd); previous = Some(hwnd);
        }
    }
    let mut entries = Vec::new();
    for index in 0..4096 {
        let mut name = [0u16; 256];
        let mut length = name.len() as u32;
        let code = unsafe { RegEnumKeyExW(root.0, index, Some(PWSTR(name.as_mut_ptr())), &mut length, None, None, None, None) };
        if code == ERROR_NO_MORE_ITEMS { break; }
        if code != ERROR_SUCCESS { return Err(format!("could not enumerate Explorer's tray catalog: {}", code.0)); }
        let Ok(key) = Key::open(root.0, PCWSTR(name.as_ptr())) else { continue; };
        let Some(path) = key.text(w!("ExecutablePath")).and_then(|p| executable(&p)) else { continue; };
        let guid = key.text(w!("IconGuid")).and_then(|g| GUID::try_from(g.trim_matches(['{', '}'])).ok()).filter(|g| *g != GUID::zeroed());
        let identity = if let Some(guid) = guid {
            let id = Identity { guid, handle: 0, uid: 0, process: 0, thread: 0, explorer };
            id.rect().is_ok().then_some(id)
        } else if let Some(uid) = key.number(w!("UID")) {
            windows.by_path.get(&path).and_then(|list| list.iter().find_map(|&(handle, process, thread)| {
                let id = Identity { guid: GUID::zeroed(), handle, uid, process, thread, explorer };
                id.rect().is_ok().then_some(id)
            }))
        } else { None };
        let Some(identity) = identity else { continue; };
        let name = std::path::Path::new(&path).file_stem().map(|v| v.to_string_lossy().into_owned()).unwrap_or_default();
        let title = application_name(&path, &name);
        let icon = key.bytes(w!("IconSnapshot"), RRF_RT_REG_BINARY, 512 * 1024).and_then(|b| snapshot(&b)).unwrap_or_else(|| format!("windows-file:{path}"));
        entries.push((identity, title, name, icon));
    }
    entries.sort_by_cached_key(|(_, title, name, _)| (title.to_lowercase(), name.clone()));
    Ok(entries)
}

pub fn read() -> windows::core::Result<SysValue> {
    {
        let catalog = catalog().lock().unwrap();
        if catalog.refreshed.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) { return Ok(SysValue::List(catalog.latest.clone())); }
    }
    // Registry reads and shell calls must not hold the published catalog's
    // mutex. Actions and a second subscriber can use the previous snapshot.
    static SCAN: Mutex<()> = Mutex::new(());
    let Ok(_scan) = SCAN.try_lock() else { return Ok(SysValue::List(catalog().lock().unwrap().latest.clone())); };
    let result = scan();
    let mut catalog = catalog().lock().unwrap();
    match result {
        Ok(entries) => { catalog.replace(entries); catalog.error = None; },
        Err(error) => {
            // Publish the empty list too: retaining old icons after Explorer
            // stops would imply that their actions are still available.
            catalog.replace(Vec::new());
            if catalog.error.as_ref() != Some(&error) { eprintln!("windows · tray: {error}"); }
            catalog.error = Some(error);
        }
    }
    Ok(SysValue::List(catalog.latest.clone()))
}
pub fn state() -> windows::core::Result<SysValue> {
    read()?;
    let catalog = catalog().lock().unwrap();
    Ok(SysValue::Map(vec![
        ("available".into(), SysValue::Bool(catalog.error.is_none())),
        ("pending".into(), SysValue::Bool(catalog.refreshed.is_none())),
        ("error".into(), SysValue::Text(catalog.error.clone().unwrap_or_default())),
        ("cached_metadata".into(), SysValue::Bool(true)),
        ("native_menus".into(), SysValue::Bool(true)),
        ("enumeration".into(), SysValue::Text("explorer-cache".into())),
        ("complete".into(), SysValue::Bool(false)),
    ]))
}
pub(super) fn identity(args: &[SysValue]) -> Result<Identity, String> {
    let [SysValue::Text(key)] = args else { return Err("tray actions require one key from the live tray catalog".into()); };
    let id = catalog().lock().unwrap().entries.get(key).cloned().ok_or("that icon is no longer in the tray catalog")?;
    id.rect()?;
    Ok(id)
}
pub fn query(name: &str, args: &[SysValue]) -> Result<SysValue, String> {
    match name {
        "tray.list" if args.is_empty() => read().map_err(|e| e.to_string()),
        "tray.state" if args.is_empty() => state().map_err(|e| e.to_string()),
        // Windows applications own their context menus. The common scene
        // contract falls back to tray.context when no menu tree is available.
        "tray.menu" => { identity(args)?; Ok(SysValue::List(Vec::new())) },
        _ => Err(format!("Windows cannot answer '{name}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tray_keys_expire_when_the_icon_or_explorer_disappears() {
        let a = Identity { guid: GUID::zeroed(), handle: 1, uid: 1, process: 2, thread: 3, explorer: 4 };
        let row = |id: Identity| (id, "Title".into(), "app".into(), "icon".into());
        let mut catalog = Catalog::default();
        catalog.replace(vec![row(a.clone())]);
        let key = catalog.entries.keys().next().unwrap().clone();
        catalog.replace(vec![row(a.clone()), row(a.clone())]);
        assert_eq!(catalog.latest.len(), 1);
        assert!(catalog.entries.contains_key(&key));
        let mut changed = a.clone(); changed.explorer += 1;
        catalog.replace(vec![row(changed)]);
        assert!(!catalog.entries.contains_key(&key));
        catalog.replace(vec![]);
        catalog.replace(vec![row(a)]);
        assert!(!catalog.entries.contains_key(&key));
    }
    #[test]
    #[ignore = "reads the signed-in user's real Explorer catalog"]
    fn native_tray_inventory() {
        let _apartment = super::super::windows_system::Apartment::new().unwrap();
        for _ in 0..3 {
            catalog().lock().unwrap().refreshed = None;
            let started = Instant::now();
            let SysValue::List(rows) = read().unwrap() else { panic!("not a tray list") };
            assert!(catalog().lock().unwrap().error.is_none());
            println!("live icons: {}, scan: {:.2} ms", rows.len(), started.elapsed().as_secs_f64() * 1000.0);
        }
        assert!(identity(&[SysValue::Text("not-a-catalog-key".into())]).is_err());
        assert!(identity(&[SysValue::Num(1.0)]).is_err());
    }
}
