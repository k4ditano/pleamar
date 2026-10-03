//! Eight owned native windows for shelf/restore validation, without a GPU.
//! Commands in --control affect only these windows; --state reports real HWNDs.
#[cfg(not(target_os = "windows"))]
fn main() { eprintln!("window-fixture requires Windows"); }
#[cfg(target_os = "windows")]
fn main() { if let Err(error) = fixture::run() { eprintln!("{error}"); std::process::exit(1); } }

#[cfg(target_os = "windows")]
mod fixture {
    use std::{path::PathBuf, time::{Duration, Instant}};
    use windows::{core::{w, HSTRING}, Win32::{Foundation::*, Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW, UI::{HiDpi::*, WindowsAndMessaging::*}}};

    unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT { unsafe {
        match message {
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut paint);
                let mut area = RECT { left: 20, top: 18, right: 480, bottom: 150 };
                let mut text: Vec<u16> = "Owned native window for Marea's minimized shelf.\nEight windows exceed the six visible stones.\nUse Marea to bring them back. No user files are opened.".encode_utf16().collect();
                DrawTextW(dc, &mut text, &mut area, DT_LEFT | DT_WORDBREAK);
                let _ = EndPaint(hwnd, &paint);
                LRESULT(0)
            }
            WM_CLOSE => { let _ = DestroyWindow(hwnd); LRESULT(0) }
            _ => DefWindowProcW(hwnd, message, wp, lp),
        }
    } }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let args: Vec<_> = std::env::args().skip(1).collect();
        let [control_flag, control, state_flag, state] = args.as_slice() else {
            return Err("usage: window-fixture --control COMMAND_FILE --state STATE_FILE".into());
        };
        if control_flag != "--control" || state_flag != "--state" { return Err("invalid arguments".into()); }
        let (control, state) = (PathBuf::from(control), PathBuf::from(state));
        if state.exists() { return Err("state file already exists".into()); }
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let instance = GetModuleHandleW(None)?;
            let class = w!("PleamarWindowFixture");
            let wc = WNDCLASSW { hInstance: instance.into(), lpszClassName: class,
                lpfnWndProc: Some(procedure), hCursor: LoadCursorW(None, IDC_ARROW)?,
                hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0), ..Default::default() };
            if RegisterClassW(&wc) == 0 { return Err(windows::core::Error::from_thread().into()); }
            let mut windows = Vec::new();
            for index in 1..=8 {
                let title = HSTRING::from(format!("Pleamar shelf fixture {index} — Español 日本語"));
                let hwnd = CreateWindowExW(WS_EX_APPWINDOW, class, &title, WS_OVERLAPPEDWINDOW,
                    850 + index * 22, 70 + index * 34, 550, 230, None, None, Some(instance.into()), None)?;
                let _ = ShowWindow(hwnd, SW_SHOWMINNOACTIVE);
                windows.push(hwnd);
            }
            let timer = SetTimer(None, 0, 200, None);
            if timer == 0 { return Err(windows::core::Error::from_thread().into()); }
            let start = Instant::now();
            let mut previous_command = String::new();
            let mut previous_state = String::new();
            let mut message = MSG::default();
            while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
                if message.message != WM_TIMER { continue; }
                windows.retain(|hwnd| IsWindow(Some(*hwnd)).as_bool());
                let command = std::fs::read_to_string(&control).unwrap_or_default();
                let mut done = start.elapsed() > Duration::from_secs(15 * 60);
                if command != previous_command {
                    previous_command = command.clone();
                    let mut words = command.split_whitespace();
                    let action = words.next();
                    let index = words.next().and_then(|v| v.parse::<usize>().ok());
                    match action {
                        Some("minimize") => for hwnd in &windows { let _ = ShowWindow(*hwnd, SW_SHOWMINNOACTIVE); },
                        Some("close") => { let hwnd = index.and_then(|i| windows.get(i)).ok_or("missing owned window index")?; DestroyWindow(*hwnd)?; },
                        Some("rename") => { let hwnd = index.and_then(|i| windows.get(i)).ok_or("missing owned window index")?; SetWindowTextW(*hwnd, w!("Pleamar shelf fixture renamed — 世界 🚀"))?; },
                        Some("quit") => done = true,
                        Some(value) => return Err(format!("unknown fixture command: {value}").into()),
                        None => {},
                    }
                }
                windows.retain(|hwnd| IsWindow(Some(*hwnd)).as_bool());
                let rows: Vec<_> = windows.iter().map(|hwnd| {
                    let mut title = [0u16; 256];
                    let len = GetWindowTextW(*hwnd, &mut title).max(0) as usize;
                    serde_json::json!({"hwnd": hwnd.0 as usize, "title": String::from_utf16_lossy(&title[..len]),
                        "minimized": IsIconic(*hwnd).as_bool(), "foreground": GetForegroundWindow() == *hwnd})
                }).collect();
                let report = serde_json::json!({"pid": std::process::id(), "windows": rows, "done": done}).to_string();
                if report != previous_state {
                    std::fs::write(&state, &report)?;
                    println!("{report}");
                    previous_state = report;
                }
                if done { break; }
            }
            let _ = KillTimer(None, timer);
            for hwnd in windows { let _ = DestroyWindow(hwnd); }
        }
        Ok(())
    }
}
