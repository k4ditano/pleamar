//! One frozen desktop per service worker. Selection and PNG encoding never run
//! on the render thread; a reload closes the selector and drops its pixels.
use super::SysValue;
use std::{cell::{Cell, RefCell}, path::{Path, PathBuf}, sync::{Arc, Mutex, OnceLock, atomic::{AtomicBool, AtomicU32, Ordering}}, time::{Duration, Instant}};
use windows::{core::w, Win32::{Foundation::*, Graphics::{Dwm::*, Gdi::*}, System::{Com::CoTaskMemFree, LibraryLoader::GetModuleHandleW}, UI::{HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*}}};

thread_local! {
    static FROZEN: RefCell<Option<Frame>> = const { RefCell::new(None) };
    static LIFETIME: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}
pub(crate) fn set_service_lifetime(active: Arc<AtomicBool>) { LIFETIME.with(|v| *v.borrow_mut() = Some(active)); }
pub(crate) fn service_lifetime() -> Option<Arc<AtomicBool>> { LIFETIME.with(|v| v.borrow().clone()) }
fn active() -> bool { LIFETIME.with(|v| v.borrow().as_ref().is_none_or(|v| v.load(Ordering::Acquire))) }
struct Dpi(DPI_AWARENESS_CONTEXT);
impl Dpi { fn physical() -> Result<Self, String> {
    let previous = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if previous.0.is_null() { Err(windows::core::Error::from_thread().to_string()) } else { Ok(Self(previous)) }
} }
impl Drop for Dpi { fn drop(&mut self) { if !self.0.0.is_null() { unsafe { SetThreadDpiAwarenessContext(self.0); } } } }

struct Frame { id: u32, bounds: RECT, pixels: Vec<u8>, region: bool, created: Instant }
fn size(r: RECT) -> (i32, i32) { (r.right - r.left, r.bottom - r.top) }
fn intersection(a: RECT, b: RECT) -> Option<RECT> {
    let r = RECT { left: a.left.max(b.left), top: a.top.max(b.top), right: a.right.min(b.right), bottom: a.bottom.min(b.bottom) };
    (r.right > r.left && r.bottom > r.top).then_some(r)
}
fn desktop_bounds() -> RECT { unsafe {
    let (left, top) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
    RECT { left, top, right: left + GetSystemMetrics(SM_CXVIRTUALSCREEN), bottom: top + GetSystemMetrics(SM_CYVIRTUALSCREEN) }
} }
fn freeze(scope: &str) -> Result<Frame, String> {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let _dpi = Dpi::physical()?;
    let desktop = desktop_bounds();
    let bounds = unsafe { match scope {
        "region" => desktop,
        "display" => {
            let mut cursor = POINT::default();
            GetCursorPos(&mut cursor).map_err(|e| e.to_string())?;
            let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() { return Err("could not read the selected monitor".into()); }
            info.rcMonitor
        }
        "active_window" => {
            let hwnd = super::windows::capture_window().ok_or("there is no active application window to capture")?;
            if IsIconic(hwnd).as_bool() || !IsWindowVisible(hwnd).as_bool() { return Err("the active window is not visible".into()); }
            let mut rect = RECT::default();
            DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut rect as *mut _ as _, std::mem::size_of::<RECT>() as u32).map_err(|e| e.to_string())?;
            intersection(rect, desktop).ok_or("the active window is outside the desktop")?
        }
        _ => return Err("screenshot scope must be region, display or active_window".into()),
    } };
    let (width, height) = size(bounds);
    let pixels = super::windows_backdrop::snapshot([bounds.left, bounds.top, width, height])?;
    Ok(Frame { id: NEXT.fetch_add(1, Ordering::Relaxed), bounds, pixels, region: scope == "region", created: Instant::now() })
}

fn crop(frame: &Frame, rect: RECT) -> Result<image::RgbaImage, String> {
    let (width, height) = size(frame.bounds);
    if rect.left < 0 || rect.top < 0 || rect.right > width || rect.bottom > height || rect.right - rect.left < 2 || rect.bottom - rect.top < 2 {
        return Err("the selected region must contain at least 2 by 2 pixels inside the frozen desktop".into());
    }
    let (w, h) = size(rect);
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for y in rect.top..rect.bottom {
        let row = (y as usize * width as usize + rect.left as usize) * 4;
        for bgra in frame.pixels[row..row + w as usize * 4].chunks_exact(4) { rgba.extend_from_slice(&[bgra[2], bgra[1], bgra[0], 255]); }
    }
    image::RgbaImage::from_raw(w as u32, h as u32, rgba).ok_or("invalid screenshot dimensions".into())
}
fn pictures() -> Result<PathBuf, String> { unsafe {
    let raw = SHGetKnownFolderPath(&FOLDERID_Pictures, KF_FLAG_DEFAULT, None).map_err(|e| e.to_string())?;
    let path = raw.to_string().map_err(|e| e.to_string());
    CoTaskMemFree(Some(raw.0 as _));
    Ok(PathBuf::from(path?).join("Marea"))
} }
fn write_png(directory: &Path, image: &image::RgbaImage) -> Result<PathBuf, String> {
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    // Create-new prevents a repeated shot or another instance overwriting a photo.
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f").to_string();
    for index in 0..100 {
        let path = directory.join(format!("marea-{stamp}-{index}.png"));
        let file = match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        };
        use image::ImageEncoder;
        let mut writer = std::io::BufWriter::new(file);
        let result = image::codecs::png::PngEncoder::new(&mut writer).write_image(image.as_raw(), image.width(), image.height(), image::ExtendedColorType::Rgba8)
            .map_err(|e| e.to_string()).and_then(|()| std::io::Write::flush(&mut writer).map_err(|e| e.to_string()));
        drop(writer);
        if let Err(e) = result { let _ = std::fs::remove_file(&path); return Err(e); }
        return Ok(path);
    }
    Err("could not allocate a unique screenshot filename".into())
}
fn finish(frame: Frame) -> Result<SysValue, String> {
    if frame.created.elapsed() > Duration::from_secs(60) { return Err("the frozen screenshot has expired".into()); }
    let _dpi = Dpi::physical()?;
    let (width, height) = size(frame.bounds);
    let selected = if frame.region { select_region(&frame)? } else { Some(RECT { left: 0, top: 0, right: width, bottom: height }) };
    let Some(selected) = selected else { return Ok(SysValue::Map(vec![("cancelled".into(), SysValue::Bool(true))])); };
    if !active() { return Err("screenshot cancelled by reload".into()); }
    let image = crop(&frame, selected)?;
    drop(frame);
    let path = write_png(&pictures()?, &image)?;
    let clipboard = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_image(arboard::ImageData {
        width: image.width() as usize, height: image.height() as usize, bytes: std::borrow::Cow::Borrowed(image.as_raw()),
    }));
    Ok(SysValue::Map(vec![
        ("path".into(), SysValue::Text(path.to_string_lossy().into_owned())),
        ("width".into(), SysValue::Num(image.width() as f64)), ("height".into(), SysValue::Num(image.height() as f64)),
        ("clipboard".into(), SysValue::Bool(clipboard.is_ok())),
        ("clipboard_error".into(), SysValue::Text(clipboard.err().map(|e| e.to_string()).unwrap_or_default())),
    ]))
}
pub fn query(name: &str, args: &[SysValue]) -> Result<SysValue, String> {
    match (name, args) {
        ("screenshot.freeze", [SysValue::Text(scope)]) => {
            // Drop the old buffer before allocating another large desktop.
            FROZEN.with(|v| v.borrow_mut().take());
            let frame = freeze(scope)?;
            let value = SysValue::Num(frame.id as f64);
            FROZEN.with(|v| *v.borrow_mut() = Some(frame));
            Ok(value)
        }
        ("screenshot.finish" | "screenshot.cancel", [SysValue::Num(id)]) => {
            let frame = FROZEN.with(|v| {
                let mut v = v.borrow_mut();
                if v.as_ref().is_some_and(|f| f.id as f64 == *id) { v.take() } else { None }
            }).ok_or("this service worker has no frozen screenshot with that identifier")?;
            if name == "screenshot.cancel" { Ok(SysValue::Null) } else { finish(frame) }
        }
        _ => Err("invalid screenshot query or arguments".into()),
    }
}

// Win32 can reenter the procedure during SetCapture/ReleaseCapture. Shared
// references with Cells avoid overlapping mutable references to this state.
struct Selection<'a> { frame: &'a Frame, start: Cell<Option<POINT>>, end: Cell<POINT>, result: Cell<Option<RECT>>, done: Cell<bool>, started: Instant, hint: POINT }
fn selection_rect(start: POINT, end: POINT, width: i32, height: i32) -> RECT {
    RECT { left: start.x.min(end.x).clamp(0, width), top: start.y.min(end.y).clamp(0, height), right: start.x.max(end.x).clamp(0, width), bottom: start.y.max(end.y).clamp(0, height) }
}
unsafe extern "system" fn selector_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT { unsafe {
    if msg == WM_NCCREATE {
        let create = &*(l.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Selection<'_>;
    if raw.is_null() { return DefWindowProcW(hwnd, msg, w, l); }
    let s = &*raw;
    let (width, height) = size(s.frame.bounds);
    match msg {
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut paint);
            let mut info = BITMAPINFO::default();
            info.bmiHeader = BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: width, biHeight: -height, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() };
            StretchDIBits(dc, 0, 0, width, height, 0, 0, width, height, Some(s.frame.pixels.as_ptr().cast()), &info, DIB_RGB_COLORS, SRCCOPY);
            if let Some(start) = s.start.get() { let _ = DrawFocusRect(dc, &selection_rect(start, s.end.get(), width, height)); }
            SetBkColor(dc, COLORREF(0x202020)); SetTextColor(dc, COLORREF(0xffffff));
            let hint: Vec<u16> = "  Drag to capture | Esc / right click: cancel  ".encode_utf16().collect();
            let _ = TextOutW(dc, s.hint.x, s.hint.y, &hint);
            let _ = EndPaint(hwnd, &paint);
            return LRESULT(0);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_LBUTTONDOWN | WM_MOUSEMOVE | WM_LBUTTONUP => {
            // XOR only the old/new selection outline. Repainting a multi-4K
            // frozen desktop for every mouse move wastes memory bandwidth.
            let dc = GetDC(Some(hwnd));
            if let Some(start) = s.start.get() { let _ = DrawFocusRect(dc, &selection_rect(start, s.end.get(), width, height)); }
            let p = POINT { x: l.0 as i16 as i32, y: (l.0 >> 16) as i16 as i32 };
            if msg == WM_LBUTTONDOWN { s.start.set(Some(p)); SetCapture(hwnd); }
            s.end.set(p);
            if msg == WM_LBUTTONUP && s.start.get().is_some() {
                let rect = selection_rect(s.start.get().unwrap(), p, width, height);
                if rect.right - rect.left >= 2 && rect.bottom - rect.top >= 2 { s.result.set(Some(rect)); s.done.set(true); }
                else { s.start.set(None); }
                let _ = ReleaseCapture();
            }
            if let Some(start) = s.start.get() { let _ = DrawFocusRect(dc, &selection_rect(start, s.end.get(), width, height)); }
            ReleaseDC(Some(hwnd), dc);
            return LRESULT(0);
        }
        WM_KEYDOWN if w.0 == VK_ESCAPE.0 as usize => s.done.set(true),
        WM_RBUTTONDOWN | WM_CLOSE | WM_CANCELMODE | WM_DISPLAYCHANGE => s.done.set(true),
        WM_ACTIVATE if w.0 as u16 == WA_INACTIVE as u16 => s.done.set(true),
        WM_TIMER if !active() || s.started.elapsed() > Duration::from_secs(120) => s.done.set(true),
        WM_NCDESTROY => { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0); }
        _ => return DefWindowProcW(hwnd, msg, w, l),
    }
    LRESULT(0)
} }
fn select_region(frame: &Frame) -> Result<Option<RECT>, String> {
    static SELECTING: Mutex<()> = Mutex::new(());
    let _selection = SELECTING.try_lock().map_err(|_| "another screenshot selection is already open")?;
    static CLASS: OnceLock<Result<(), String>> = OnceLock::new();
    let instance = unsafe { GetModuleHandleW(None).map_err(|e| e.to_string())? };
    CLASS.get_or_init(|| unsafe {
        let class = WNDCLASSW { lpfnWndProc: Some(selector_proc), hInstance: instance.into(), hCursor: LoadCursorW(None, IDC_CROSS).map_err(|e| e.to_string())?, lpszClassName: w!("pleamar-screenshot"), ..Default::default() };
        if RegisterClassW(&class) == 0 { Err(windows::core::Error::from_thread().to_string()) } else { Ok(()) }
    }).clone()?;
    let (width, height) = size(frame.bounds);
    let previous = unsafe { GetForegroundWindow() };
    let mut cursor = POINT::default();
    unsafe { let _ = GetCursorPos(&mut cursor); }
    let mut monitor = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe { let _ = GetMonitorInfoW(MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST), &mut monitor); }
    let hint = POINT { x: monitor.rcMonitor.left - frame.bounds.left + 20, y: monitor.rcMonitor.top - frame.bounds.top + 20 };
    let state = Selection { frame, start: Cell::new(None), end: Cell::new(POINT::default()), result: Cell::new(None), done: Cell::new(false), started: Instant::now(), hint };
    unsafe {
        let discoverable = if std::env::var_os("PLEAMAR_TEST_WINDOWS").is_some() { WS_EX_APPWINDOW } else { WS_EX_TOOLWINDOW };
        let hwnd = CreateWindowExW(WS_EX_TOPMOST | discoverable, w!("pleamar-screenshot"), w!("Marea screenshot selection"), WS_POPUP,
            frame.bounds.left, frame.bounds.top, width, height, None, None, Some(instance.into()), Some(&state as *const _ as _)).map_err(|e| e.to_string())?;
        // Only this worker owns the selector. Always release the HWND before the
        // borrowed frame/state disappear, including failures in the message loop.
        struct Window(HWND);
        impl Drop for Window { fn drop(&mut self) { unsafe { let _ = DestroyWindow(self.0); } } }
        let window = Window(hwnd);
        if SetTimer(Some(hwnd), 1, 100, None) == 0 { return Err("could not start the screenshot cancellation timer".into()); }
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        let mut message = MSG::default();
        while !state.done.get() {
            let result = GetMessageW(&mut message, None, 0, 0).0;
            if result == -1 { return Err(windows::core::Error::from_thread().to_string()); }
            if result == 0 { break; }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        let restore = GetForegroundWindow() == hwnd;
        drop(window);
        if restore && IsWindow(Some(previous)).as_bool() { let _ = SetForegroundWindow(previous); }
    }
    Ok(state.result.get())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reverse_selection_and_negative_monitor_origin_crop_frozen_pixels() {
        let rect = selection_rect(POINT { x: 3, y: 2 }, POINT { x: -8, y: 0 }, 4, 2);
        assert_eq!(size(rect), (3, 2));
        let frame = Frame { id: 1, bounds: RECT { left: -4, top: -2, right: 0, bottom: 0 }, pixels: (0..8).flat_map(|i| [i, 20, 30, 0]).collect(), region: true, created: Instant::now() };
        let crop = crop(&frame, rect).unwrap();
        assert_eq!(crop.dimensions(), (3, 2));
        assert_eq!(crop.get_pixel(0, 1).0, [30, 20, 4, 255]);
        assert!(super::crop(&frame, RECT { left: 0, top: 0, right: 5, bottom: 2 }).is_err());
        assert!(intersection(frame.bounds, RECT { left: 0, top: 0, right: 4, bottom: 2 }).is_none());
    }
    #[test]
    fn unicode_png_roundtrip_does_not_overwrite_existing_photos() {
        let directory = std::env::temp_dir().join(format!("pleamar capture ñ 空 {}", std::process::id()));
        let image = image::RgbaImage::from_pixel(2, 3, image::Rgba([20, 40, 80, 255]));
        let first = write_png(&directory, &image).unwrap();
        let second = write_png(&directory, &image).unwrap();
        assert_ne!(first, second);
        assert_eq!(image::open(&first).unwrap().into_rgba8(), image);
        std::fs::remove_file(first).unwrap(); std::fs::remove_file(second).unwrap(); std::fs::remove_dir(directory).unwrap();
    }
    #[test]
    #[ignore = "read-only verification of the image clipboard after a manual screenshot"]
    fn live_clipboard_matches_png() {
        let path = std::env::var_os("PLEAMAR_TEST_SCREENSHOT").expect("provide the PNG from this test capture");
        let expected = image::open(path).unwrap().into_rgba8();
        let actual = arboard::Clipboard::new().unwrap().get_image().unwrap();
        assert_eq!((actual.width, actual.height), (expected.width() as usize, expected.height() as usize));
        assert_eq!(actual.bytes.as_ref(), expected.as_raw());
        println!("clipboard matches the saved PNG pixel for pixel ({} x {})", actual.width, actual.height);
    }
}
