//! Repeated WGC requests and retiring workers, with only an owned DISPLAY2 window.
use super::*;
use std::{os::windows::process::CommandExt, path::PathBuf, process::{Child, Command}};

struct ChildWindow(Child);
impl Drop for ChildWindow { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }

#[test]
#[ignore = "captures an owned non-activating DISPLAY2 fixture repeatedly; no physical input"]
fn repeated_capture_workers() {
    let _dpi = Dpi::physical().unwrap();
    let screen = monitors().unwrap().into_iter().find(|m| m.name == r"\\.\DISPLAY2" && !m.primary)
        .expect("active non-primary DISPLAY2 is required");
    let folder = PathBuf::from(std::env::var_os("PLEAMAR_CAPTURE_TEST_DIR").expect("new evidence directory required"));
    std::fs::create_dir(&folder).unwrap();
    let foreground = unsafe { GetForegroundWindow() };
    let mut child = ChildWindow(Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "platform::windows_desktop::native_tests::owned_fixture", "--nocapture"])
        .env("PLEAMAR_OWNED_DESKTOP_FIXTURE", &folder).creation_flags(0x08000000 | 0x00004000).spawn().unwrap());
    let start = Instant::now();
    let mut found = (child.0.id(), HWND::default());
    unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> BOOL { unsafe {
        let found = &mut *(data.0 as *mut (u32, HWND));
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == found.0 && IsWindowVisible(hwnd).as_bool() { found.1 = hwnd; return false.into(); }
        true.into()
    } }
    while found.1.is_invalid() {
        assert!(start.elapsed() < Duration::from_secs(10));
        let _ = unsafe { EnumWindows(Some(visit), LPARAM(&mut found as *mut _ as isize)) };
        std::thread::sleep(Duration::from_millis(25));
    }
    let rect = bounds(found.1).unwrap();
    assert!(rect.left >= screen.rect.left && rect.top >= screen.rect.top
        && rect.right <= screen.rect.right && rect.bottom <= screen.rect.bottom);
    unsafe { let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(found.1, false); }
    let hwnd = found.1.0 as isize;
    // Reuse one worker, then replace it as a scene reload does.
    for count in [16, 2, 2, 2] {
        std::thread::spawn(move || {
            let _dpi = Dpi::physical().unwrap();
            for _ in 0..count {
                let result = capture::window(HWND(hwnd as _), rect).unwrap();
                let SysValue::Map(fields) = result else { panic!("capture metadata") };
                let (_, SysValue::Text(data)) = fields.iter().find(|(k, _)| k == "data").unwrap() else { panic!("PNG") };
                use base64::Engine;
                let bytes = base64::engine::general_purpose::STANDARD.decode(data).unwrap();
                let image = image::load_from_memory(&bytes).unwrap().to_rgba8();
                assert!(image.pixels().any(|p| p[0] > 200 && p[1] < 60 && p[2] < 60));
            }
        }).join().unwrap();
    }
    std::fs::write(folder.join("control"), "quit").unwrap();
    let exit = child.0.wait().unwrap();
    assert!(exit.success());
    assert_eq!(unsafe { GetForegroundWindow() }, foreground);
    std::fs::write(folder.join("result.json"), serde_json::json!({"captures":22,"workers":4,
        "screen":screen.name,"physical_input":false,"foreground_unchanged":true,"exit":0}).to_string()).unwrap();
}
