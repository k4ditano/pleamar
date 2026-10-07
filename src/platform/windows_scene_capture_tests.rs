//! Capture only the non-activating scene process this fixture starts on DISPLAY2.
use super::*;
use base64::Engine;
use std::{os::windows::process::CommandExt, path::PathBuf, process::{Child, Command, Stdio}};

struct OwnedScene(Child);
impl Drop for OwnedScene { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }

fn scene_window(pid: u32) -> Option<HWND> {
    struct Find { pid: u32, window: Option<HWND> }
    unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> BOOL { unsafe {
        let find = &mut *(data.0 as *mut Find);
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == find.pid && IsWindowVisible(hwnd).as_bool()
            && caption(hwnd) == r"pleamar surface 0 · \\.\DISPLAY2" {
            find.window = Some(hwnd);
        }
        true.into()
    } }
    let mut find = Find { pid, window: None };
    unsafe { EnumWindows(Some(visit), LPARAM(&mut find as *mut _ as isize)) }.unwrap();
    find.window
}

#[test]
#[ignore = "starts and captures only an owned non-activating DISPLAY2 scene; no physical input"]
fn native_scene_capture() {
    let _dpi = Dpi::physical().unwrap();
    let screen = monitors().unwrap().into_iter().find(|m| m.name == r"\\.\DISPLAY2" && !m.primary)
        .expect("active non-primary DISPLAY2 is required");
    let path = |key| PathBuf::from(std::env::var_os(key).expect(key)).canonicalize().unwrap();
    let binary = path("PLEAMAR_SCENE_TEST_BINARY");
    let scene = path("PLEAMAR_SCENE_TEST_FILE");
    let out = path("PLEAMAR_SCENE_TEST_OUTPUT");
    // These fixtures may render UI but must never reserve the desktop, lock it
    // or take keyboard focus. The driver owns the scene and its reloads.
    let (parsed, _) = crate::language::read_file(scene.to_str().unwrap()).unwrap();
    assert!(parsed.surfaces.iter().all(|s| s.window.is_none() && !s.lock_screen
        && s.keyboard == crate::scene::Keyboard::Never && s.exclusive_zone == 0
        && s.reserve_while.is_none() && !s.hidden_from_captures));
    drop(parsed);
    let foreground = unsafe { GetForegroundWindow() };
    let log = std::fs::File::create(out.join("scene.log")).unwrap();
    let process = Command::new(binary).args(["--scene", scene.to_str().unwrap(), "--screen", r"\\.\DISPLAY2",
        "--no-hud", "--stall", "0", "--seconds", "300"])
        .env("PLEAMAR_TEST_WINDOWS", "1").env("PLEAMAR_NO_RELAUNCH", "1")
        .stdout(log.try_clone().unwrap()).stderr(Stdio::from(log))
        .creation_flags(0x08000000 | 0x00004000).spawn().unwrap();
    let mut process = OwnedScene(process);
    let started = Instant::now();
    let mut request = String::new();
    let mut count = 0;
    std::fs::write(out.join("ready.json"), format!("{{\"pid\":{}}}", process.0.id())).unwrap();
    while started.elapsed() < Duration::from_secs(300) {
        assert!(process.0.try_wait().unwrap().is_none(), "owned scene exited before the capture driver");
        let next = std::fs::read_to_string(out.join("request")).unwrap_or_default();
        if next == "quit" { break; }
        if next != request && !next.is_empty() {
            assert!(next.len() < 60 && next.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'));
            count += 1;
            assert!(count <= 20, "capture fixture request limit");
            let hwnd = scene_window(process.0.id()).expect("the owned main scene canvas is not visible");
            let rect = bounds(hwnd).unwrap();
            let current = monitors().unwrap().into_iter().find(|m| m.name == screen.name && !m.primary).unwrap();
            assert!(rect.left >= current.rect.left && rect.top >= current.rect.top
                && rect.right <= current.rect.right && rect.bottom <= current.rect.bottom, "owned scene left DISPLAY2");
            let image = capture::window(hwnd, rect).unwrap();
            let SysValue::Map(fields) = image else { panic!("capture metadata") };
            let data = fields.iter().find(|(k,_)| k == "data").unwrap();
            let SysValue::Text(data) = &data.1 else { panic!("PNG data") };
            let png = base64::engine::general_purpose::STANDARD.decode(data).unwrap();
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(out.join(format!("{next}.png"))).unwrap();
            std::io::Write::write_all(&mut file, &png).unwrap();
            std::fs::write(out.join("captured"), &next).unwrap();
            request = next;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(started.elapsed() < Duration::from_secs(300), "capture driver timed out");
    // Close only our child's main window, with no global input or focus change.
    if let Some(hwnd) = scene_window(process.0.id()) {
        unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }.unwrap();
    }
    let closing = Instant::now();
    while process.0.try_wait().unwrap().is_none() && closing.elapsed() < Duration::from_secs(12) {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(process.0.try_wait().unwrap().and_then(|s| s.code()), Some(0));
    std::fs::write(out.join("capture-result.json"), serde_json::json!({"captures":count,
        "physical_input_sent":false,"foreground_unchanged":unsafe { GetForegroundWindow() } == foreground,
        "scene_pid":process.0.id(),"exit":0,"screen":screen.name}).to_string()).unwrap();
}
