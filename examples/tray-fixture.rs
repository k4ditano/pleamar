//! An owned notification icon for interactive Windows integration tests.
//! Writes only its own activation/menu events to stdout and exits after 15 min.
#[cfg(not(target_os = "windows"))]
fn main() { eprintln!("tray-fixture requires Windows"); }
#[cfg(target_os = "windows")]
fn main() { if let Err(e) = fixture::run() { eprintln!("{e}"); std::process::exit(1); } }
#[cfg(target_os = "windows")]
mod fixture {
    use windows::{core::{w, GUID}, Win32::{Foundation::*, System::LibraryLoader::GetModuleHandleW, UI::{Shell::*, WindowsAndMessaging::*}}};
    const CALLBACK: u32 = WM_APP + 1;
    const GUID: GUID = GUID::from_u128(0x907dfb2c_5280_4f65_8eef_436d6c3d81bf);
    static LEGACY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    fn legacy() -> bool { *LEGACY.get_or_init(|| std::env::args().any(|arg| arg == "--legacy")) }
    fn icon(hwnd: HWND) -> NOTIFYICONDATAW { NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 41, guidItem: if legacy() { GUID::zeroed() } else { GUID }, uFlags: if legacy() { NOTIFY_ICON_DATA_FLAGS(0) } else { NIF_GUID }, ..Default::default() } }
    unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT { unsafe {
        match message {
            CALLBACK => {
                let event = lp.0 as u32 & 0xffff;
                if if legacy() { event == WM_LBUTTONUP } else { event == NIN_SELECT || event == (NIN_SELECT | 1) } {
                    println!("fixture: activated");
                    let _ = SetWindowTextW(hwnd, w!("Pleamar tray fixture — activated"));
                } else if if legacy() { event == WM_RBUTTONUP } else { event == WM_CONTEXTMENU } {
                    println!("fixture: context");
                    let _ = SetWindowTextW(hwnd, w!("Pleamar tray fixture — context open"));
                    let menu = CreatePopupMenu().unwrap();
                    let _ = AppendMenuW(menu, MF_STRING, 1, w!("Validate Unicode — Español 日本語"));
                    let _ = AppendMenuW(menu, MF_STRING, 2, w!("Close this test fixture"));
                    let _ = SetForegroundWindow(hwnd);
                    let mut point = POINT::default(); let _ = GetCursorPos(&mut point);
                    let selected = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY, point.x, point.y, None, hwnd, None).0;
                    let _ = DestroyMenu(menu);
                    if selected == 1 { println!("fixture: menu selected"); let _ = SetWindowTextW(hwnd, w!("Pleamar tray fixture — menu selected")); }
                    if selected == 2 { let _ = DestroyWindow(hwnd); }
                    if selected == 0 { println!("fixture: menu dismissed"); }
                    let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
                }
                LRESULT(0)
            }
            WM_TIMER | WM_CLOSE => { let _ = DestroyWindow(hwnd); LRESULT(0) }
            WM_DESTROY => { let _ = Shell_NotifyIconW(NIM_DELETE, &icon(hwnd)); PostQuitMessage(0); LRESULT(0) }
            _ => DefWindowProcW(hwnd, message, wp, lp),
        }
    } }
    pub fn run() -> windows::core::Result<()> { unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(None)?;
        let class = w!("PleamarTrayFixture");
        let wc = WNDCLASSW { hInstance: instance.into(), lpszClassName: class, lpfnWndProc: Some(procedure), hCursor: LoadCursorW(None, IDC_ARROW)?, ..Default::default() };
        if RegisterClassW(&wc) == 0 { return Err(windows::core::Error::from_thread()); }
        let hwnd = CreateWindowExW(WS_EX_APPWINDOW, class, w!("Pleamar tray fixture"), WS_OVERLAPPEDWINDOW, 200, 220, 560, 200, None, None, Some(instance.into()), None)?;
        let mut data = icon(hwnd);
        data.uFlags |= NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        data.uCallbackMessage = CALLBACK;
        data.hIcon = LoadIconW(None, IDI_INFORMATION)?;
        for (dst, value) in data.szTip.iter_mut().zip("Pleamar tray validation — Español 日本語".encode_utf16()) { *dst = value; }
        if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() { return Err(windows::core::Error::from_thread()); }
        if !legacy() {
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
        }
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(Some(hwnd), 1, 15 * 60 * 1000, None);
        println!("fixture: ready");
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() { let _ = TranslateMessage(&message); DispatchMessageW(&message); }
        Ok(())
    } }
}
