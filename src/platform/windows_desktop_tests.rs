//! Opt-in native checks use only a child fixture on a verified secondary monitor.
use super::*;
use std::{path::{Path, PathBuf}, process::{Child, Command}, os::windows::process::CommandExt};
use windows::{core::{w, HSTRING}, Win32::{System::LibraryLoader::GetModuleHandleW, UI::Input::KeyboardAndMouse::EnableWindow}};

fn field<'a>(value: &'a SysValue, name: &str) -> &'a SysValue {
    let SysValue::Map(entries) = value else { panic!("expected a map") };
    &entries.iter().find(|(key, _)| key == name).unwrap().1
}
fn text(value: &SysValue) -> &str { let SysValue::Text(value) = value else { panic!("expected text") }; value }
fn list(value: &SysValue) -> &[SysValue] { let SysValue::List(value) = value else { panic!("expected list") }; value }
fn command(name: &str, args: &[SysValue]) -> Result<(), String> {
    let mut values = vec![SysValue::Num(EPOCH.load(Ordering::Acquire) as f64)]; values.extend_from_slice(args);
    super::command(name, &values)
}
fn state(folder: &Path) -> serde_json::Value { std::fs::read(folder.join("state.json")).ok().and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or(serde_json::Value::Null) }
fn wait(mut ready: impl FnMut() -> bool) {
    let started = Instant::now();
    while !ready() { assert!(started.elapsed() < Duration::from_secs(12), "fixture timeout"); std::thread::sleep(Duration::from_millis(25)); }
}
fn request(folder: &Path, command: &str) {
    std::fs::write(folder.join("control"), command).unwrap();
    wait(|| state(folder)["command"] == command);
}
struct OwnedChild(Child);
impl Drop for OwnedChild { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }
fn catalog_id(title: &str) -> String {
    let catalog = query("desktop.windows", &[]).unwrap();
    text(field(list(field(&catalog,"windows")).iter().find(|entry| text(field(entry,"title")) == title).expect("owned fixture was not cataloged"), "id")).into()
}
fn picture(id: &str, output: &Path) -> image::RgbaImage {
    let result = query("desktop.look", &[SysValue::Text(id.into())]).unwrap();
    use base64::Engine;
    let png = base64::engine::general_purpose::STANDARD.decode(text(field(&result,"data"))).unwrap();
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(output).unwrap();
    std::io::Write::write_all(&mut file, &png).unwrap();
    image::load_from_memory(&png).unwrap().to_rgba8()
}

#[test]
#[ignore = "opens only owned fixture windows on active non-primary DISPLAY2; no physical input"]
fn native_catalog_capture_and_lifetimes() {
    let _dpi = prepare().unwrap();
    let screens = monitors().unwrap();
    let monitor_index = screens.iter().position(|m| m.name == r"\\.\DISPLAY2" && !m.primary).expect("DISPLAY2 must be active and non-primary");
    let folder = std::env::var_os("PLEAMAR_DESKTOP_TEST_DIR").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join(format!("pleamar desktop ñ {}", chrono::Utc::now().timestamp_millis())));
    std::fs::create_dir(&folder).expect("test output must be a new directory");
    let foreground = unsafe { GetForegroundWindow() };
    let mut cursor = POINT::default(); unsafe { GetCursorPos(&mut cursor).unwrap(); }
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "platform::windows_desktop::native_tests::owned_fixture", "--nocapture"])
        .env("PLEAMAR_OWNED_DESKTOP_FIXTURE", &folder).creation_flags(0x08000000 | 0x00004000).spawn().unwrap();
    let mut child = OwnedChild(child);
    wait(|| state(&folder)["ready"] == true);
    let name = format!("Pleamar desktop fixture {} — Español 日本語", child.0.id());
    let id = catalog_id(&name);
    let first = picture(&id, &folder.join("window.png"));
    assert!(first.pixels().any(|p| p.0[0] > 200 && p.0[1] < 60 && p.0[2] < 60), "owned red patch was not captured");
    assert!(first.pixels().any(|p| p.0[2] > 200 && p.0[0] < 60), "owned blue background was not captured");
    // These invalid actions must fail before inserting any keyboard/mouse event.
    assert!(command("desktop.click", &[SysValue::Text(id.clone()), SysValue::Num(-1.0), SysValue::Num(20.0), SysValue::Text("left".into()), SysValue::Num(1.0)]).unwrap_err().contains("outside"));
    assert!(command("desktop.type", &[SysValue::Text(id.clone()), SysValue::Text("must not be sent".into())]).unwrap_err().contains("foreground"));
    request(&folder, "resize");
    assert!(command("desktop.type", &[SysValue::Text(id.clone()), SysValue::Text("stale".into())]).unwrap_err().contains("changed"));
    let resized = picture(&id, &folder.join("resized.png"));
    assert!(resized.width() > first.width());
    assert_eq!(catalog_id(&name), id, "live window id changed on refresh");
    command("desktop.send", &[SysValue::Text(id.clone()), SysValue::Num(monitor_index as f64)]).unwrap();
    let moved = target(&id).unwrap();
    let monitor = &screens[monitor_index];
    assert!(moved.rect.left >= monitor.rect.left && moved.rect.right <= monitor.rect.right);
    request(&folder, "dialog");
    let _ = catalog_id(&name);
    let dialog = picture(&id, &folder.join("dialog.png"));
    assert!(dialog.width() < resized.width(), "owner did not route to its modal dialog");
    request(&folder, "close-dialog");
    pump();
    assert!(command("desktop.type", &[SysValue::Text(id.clone()), SysValue::Text("old dialog".into())]).is_err());
    request(&folder, "close");
    pump();
    assert!(query("desktop.look", &[SysValue::Text(id.clone())]).is_err());
    request(&folder, "reopen");
    let reopened = catalog_id(&name);
    assert_ne!(reopened, id, "closed window's catalog id was reused");
    let _ = picture(&reopened, &folder.join("reopened.png"));
    let stale = EPOCH.load(Ordering::Acquire);
    super::command("desktop.cancel", &[]).unwrap();
    assert!(super::command("desktop.type", &[SysValue::Num(stale as f64), SysValue::Text(reopened.clone()), SysValue::Text("cancelled queued input".into())]).unwrap_err().contains("cancelled"));
    assert_eq!(state(&folder)["input_events"], 0);
    request(&folder,"quit");
    wait(|| child.0.try_wait().unwrap().is_some());
    assert_eq!(unsafe { GetForegroundWindow() }, foreground, "fixture changed foreground focus");
    let mut after = POINT::default(); unsafe { GetCursorPos(&mut after).unwrap(); }
    // The fixture itself never moves the pointer; the human may move it during
    // the test, so record rather than asserting that the global pointer froze.
    let report = serde_json::json!({"monitor":monitor.name,"native_capture":true,"window_size":[first.width(),first.height()],
        "resized_size":[resized.width(),resized.height()],"dialog_size":[dialog.width(),dialog.height()],"stable_live_id":true,
        "expired_closed_id":true,"invalid_input_rejected":true,"queued_cancel_rejected":true,"foreground_preserved":true,"physical_input_sent":false,
        "cursor_before":[cursor.x,cursor.y],"cursor_after":[after.x,after.y]});
    std::fs::write(folder.join("result.json"), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("native desktop evidence: {}",folder.display());
}

unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT { unsafe {
    match msg {
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default(); let dc = BeginPaint(hwnd,&mut paint);
            let mut rect = RECT::default(); let _ = GetClientRect(hwnd,&mut rect);
            let blue = CreateSolidBrush(COLORREF(0x00ff0000)); FillRect(dc,&rect,blue); let _ = DeleteObject(blue.into());
            let red = CreateSolidBrush(COLORREF(0x000000ff));
            FillRect(dc,&RECT{left:25,top:25,right:150,bottom:120},red); let _ = DeleteObject(red.into());
            SetBkMode(dc, TRANSPARENT); SetTextColor(dc,COLORREF(0x00ffffff));
            let letters:Vec<_>="Marea · owned native capture · Español 日本語".encode_utf16().collect();
            let _ = TextOutW(dc,25,145,&letters); let _ = EndPaint(hwnd,&paint); LRESULT(0)
        }
        WM_KEYDOWN | WM_CHAR | WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MOUSEWHEEL => {
            SetWindowLongPtrW(hwnd,GWLP_USERDATA,GetWindowLongPtrW(hwnd,GWLP_USERDATA)+1); LRESULT(0)
        }
        _ => DefWindowProcW(hwnd,msg,w,l),
    }
} }
#[test]
#[ignore = "owned child fixture, launched only by native_catalog_capture_and_lifetimes"]
fn owned_fixture() {
    let Some(folder) = std::env::var_os("PLEAMAR_OWNED_DESKTOP_FIXTURE") else { return; };
    let folder = PathBuf::from(folder);
    let _dpi = Dpi::physical().unwrap();
    let screen = monitors().unwrap().into_iter().find(|m|m.name==r"\\.\DISPLAY2" && !m.primary).expect("secondary display disappeared");
    unsafe {
        let instance = GetModuleHandleW(None).unwrap();
        let class = w!("PleamarDesktopOwnedTest");
        let wc = WNDCLASSW { lpfnWndProc:Some(procedure), hInstance:instance.into(), lpszClassName:class, ..Default::default() };
        assert_ne!(RegisterClassW(&wc),0);
        let title=HSTRING::from(format!("Pleamar desktop fixture {} — Español 日本語",std::process::id()));
        let make = |owner:Option<HWND>,width:i32,height:i32| CreateWindowExW(WINDOW_EX_STYLE(0),class,&title,WS_OVERLAPPEDWINDOW,
            screen.work.left+80,screen.work.top+90,width,height,owner,None,Some(instance.into()),None).unwrap();
        let mut hwnd=make(None,420,300);
        let mut dialog=None;
        let _ = ShowWindow(hwnd,SW_SHOWNOACTIVATE);
        let mut msg=MSG::default(); let started=Instant::now(); let mut previous=String::new();
        loop {
            while PeekMessageW(&mut msg,None,0,0,PM_REMOVE).as_bool() { let _=TranslateMessage(&msg); DispatchMessageW(&msg); }
            let command=std::fs::read_to_string(folder.join("control")).unwrap_or_default();
            let changed=command!=previous;
            if changed { match command.as_str() {
                "resize"=>{ SetWindowPos(hwnd,None,0,0,560,360,SWP_NOMOVE|SWP_NOZORDER|SWP_NOACTIVATE).unwrap(); }
                "dialog"=>{ let d=make(Some(hwnd),300,240);let _=EnableWindow(hwnd,false);let _=ShowWindow(d,SW_SHOWNOACTIVATE);dialog=Some(d); }
                "close-dialog"=>{ if let Some(d)=dialog.take(){ DestroyWindow(d).unwrap(); } let _=EnableWindow(hwnd,true); }
                "close"=>{ DestroyWindow(hwnd).unwrap(); hwnd=HWND::default(); }
                "reopen"=>{ hwnd=make(None,420,300);let _=ShowWindow(hwnd,SW_SHOWNOACTIVATE); }
                "quit"=>{}, _=>panic!("unknown owned command")
            } }
            if changed || !folder.join("state.json").exists() {
                let events=if hwnd.is_invalid(){0}else{GetWindowLongPtrW(hwnd,GWLP_USERDATA)};
                let report=serde_json::json!({"ready":true,"command":command,"input_events":events});
                std::fs::write(folder.join("state.json"),report.to_string()).unwrap(); previous=command.clone();
            }
            if command=="quit" || started.elapsed()>Duration::from_secs(60) { break; }
            std::thread::sleep(Duration::from_millis(10));
        }
        if let Some(d)=dialog { let _=DestroyWindow(d); }
        if !hwnd.is_invalid(){let _=DestroyWindow(hwnd);}
    }
}
