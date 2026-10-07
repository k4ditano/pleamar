//! Real OLE transfer to an owned scene on a disposable CI desktop only.
use super::*;
use std::{mem::size_of, time::{Duration, Instant}};
use windows::{core::w, Win32::{Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW, UI::HiDpi::*}};

unsafe extern "system" fn source_window(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, message, w, l) }
}
fn button(flags: MOUSE_EVENT_FLAGS) -> bool {
    let event = INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dwFlags: flags, ..Default::default() } } };
    unsafe { SendInput(&[event], size_of::<INPUT>() as i32) == 1 }
}
struct Dpi(DPI_AWARENESS_CONTEXT);
impl Drop for Dpi { fn drop(&mut self) { unsafe { let _ = SetThreadDpiAwarenessContext(self.0); } } }
struct Cleanup { hwnd: HWND, cursor: POINT }
impl Drop for Cleanup {
    fn drop(&mut self) {
        button(MOUSEEVENTF_LEFTUP);
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            let _ = SetCursorPos(self.cursor.x, self.cursor.y);
        }
    }
}
fn target(point: POINT, expected: u32) -> bool {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(WindowFromPoint(point), Some(&mut pid)); }
    pid == expected
}

#[test]
#[ignore = "moves the OS pointer only for an owned OLE drag on a disposable GitHub-hosted desktop"]
fn native_ole_file_source() -> Result<(), Box<dyn std::error::Error>> {
    for (key, value) in [("GITHUB_ACTIONS", "true"), ("RUNNER_ENVIRONMENT", "github-hosted"), ("PLEAMAR_WM_CI_DOCK_DROP", "1")] {
        if std::env::var(key).as_deref() != Ok(value) { return Err("requires the explicit disposable CI OLE step".into()); }
    }
    let request: serde_json::Value = serde_json::from_str(&std::env::var("PLEAMAR_DOCK_OLE_REQUEST")?)?;
    let point = POINT { x: i32::try_from(request["x"].as_i64().ok_or("missing target x")?)?, y: i32::try_from(request["y"].as_i64().ok_or("missing target y")?)? };
    let expected = u32::try_from(request["pid"].as_u64().ok_or("missing scene PID")?)?;
    let paths: Vec<String> = serde_json::from_value(request["files"].clone())?;
    if paths.is_empty() || paths.len() > 16 || paths.iter().any(|p| !std::path::Path::new(p).is_file()) {
        return Err("OLE fixture needs its existing owned test files".into());
    }
    let _ole = Apartment::new()?;
    let _dpi = Dpi(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) });
    if _dpi.0.0.is_null() { return Err("DPI context unavailable".into()); }
    if !target(point, expected) { return Err("the OLE destination is not the expected scene".into()); }
    if unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } < 0 { return Err("refusing an already pressed mouse button".into()); }
    let mut cursor = POINT::default(); unsafe { GetCursorPos(&mut cursor)?; }
    let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONULL) };
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() { return Err("destination monitor disappeared".into()); }
    let x = info.rcWork.right - 130; let y = info.rcWork.top + 8;
    if point.x >= x && point.x < x + 120 && point.y >= y && point.y < y + 80 { return Err("source would cover the drop target".into()); }
    let module = unsafe { GetModuleHandleW(None)? };
    let class = WNDCLASSW { lpfnWndProc: Some(source_window), hInstance: module.into(), lpszClassName: w!("pleamar-owned-ole-source"),
        hbrBackground: unsafe { HBRUSH(GetStockObject(WHITE_BRUSH).0) }, ..Default::default() };
    if unsafe { RegisterClassW(&class) } == 0 { return Err(windows_core::Error::from_thread().into()); }
    let hwnd = unsafe { CreateWindowExW(WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST, class.lpszClassName,
        w!("Owned OLE source"), WS_POPUP | WS_VISIBLE, x, y, 120, 80, None, None, Some(module.into()), None)? };
    let _cleanup = Cleanup { hwnd, cursor };
    unsafe { SetCursorPos(x + 20, y + 20)?; }
    if !target(POINT { x: x + 20, y: y + 20 }, std::process::id()) { return Err("source is not the owned window".into()); }
    if !button(MOUSEEVENTF_LEFTDOWN) { return Err("mouse press was rejected".into()); }
    let until = Instant::now() + Duration::from_secs(1);
    loop {
        let mut message = MSG::default();
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            unsafe { let _ = TranslateMessage(&message); DispatchMessageW(&message); }
        }
        // DoDragDrop observes this thread's queued key state. The asynchronous
        // state changes before WM_LBUTTONDOWN has necessarily been removed.
        if unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 && GetKeyState(VK_LBUTTON.0 as i32) < 0 } { break; }
        if Instant::now() >= until { return Err("mouse press did not enter the input queue".into()); }
        std::thread::sleep(Duration::from_millis(5));
    }
    let data = source_data(&paths.join("\n"))?;
    let source: IDropSource = Source { hwnd: hwnd.0 as isize }.into();
    let mover = std::thread::spawn(move || {
        let _dpi = Dpi(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) });
        std::thread::sleep(Duration::from_millis(150));
        let moved = target(point, expected) && unsafe { SetCursorPos(point.x, point.y) }.is_ok();
        std::thread::sleep(Duration::from_millis(350));
        let released = button(MOUSEEVENTF_LEFTUP);
        moved && released
    });
    let mut effect = DROPEFFECT_NONE;
    let started = Instant::now();
    let result = unsafe { DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect) };
    println!("OLE completed in {} ms: {result:?}, effect {effect:?}", started.elapsed().as_millis());
    let moved = mover.join().map_err(|_| "OLE pointer helper panicked")?;
    assert!(moved, "refused a stale destination or failed to release the pointer");
    assert_eq!(result, DRAGDROP_S_DROP, "OS drag did not finish as a drop");
    assert_eq!(effect, DROPEFFECT_COPY, "the scene did not accept the file copy");
    println!("{}", serde_json::json!({"passed":true,"os_pointer_moved":true,"ole_drop":true,"destination_pid":expected,"files":paths}));
    Ok(())
}
