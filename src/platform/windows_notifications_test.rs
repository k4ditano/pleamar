//! Opt-in native round trip; creates/removes only its own shortcut and toast.
#[test]
#[ignore = "posts and removes a dedicated local validation toast"]
fn native_toast_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
    use crate::platform::SysValue;
    use windows::{core::{w, Interface, HSTRING, PCWSTR, GUID}, Data::Xml::Dom::XmlDocument,
        UI::Notifications::{ToastNotification, ToastNotificationManager}, Win32::{Foundation::PROPERTYKEY,
        System::{Com::{CoCreateInstance, CLSCTX_INPROC_SERVER, IPersistFile, StructuredStorage::PROPVARIANT}, WinRT::*},
        UI::Shell::{ShellLink, IShellLinkW, PropertiesSystem::IPropertyStore}}};
    use std::{path::PathBuf, os::windows::ffi::OsStrExt, time::{Duration, Instant}};
    fn wide(path: &std::path::Path) -> Vec<u16> { path.as_os_str().encode_wide().chain([0]).collect() }
    fn field<'a>(v: &'a SysValue, name: &str) -> Option<&'a SysValue> {
        if let SysValue::Map(row) = v { row.iter().find(|(key, _)| key == name).map(|(_, v)| v) } else { None }
    }
    unsafe { RoInitialize(RO_INIT_MULTITHREADED)?; }
    struct Cleanup { shortcut: PathBuf, app: HSTRING }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Ok(history) = ToastNotificationManager::History() {
                let _ = history.RemoveGroupedTagWithId(&HSTRING::from("owned"), &HSTRING::from("validation"), &self.app);
            }
            let _ = std::fs::remove_file(&self.shortcut);
        }
    }
    let app = HSTRING::from(format!("Pleamar.Validation.{}", std::process::id()));
    let shortcut = PathBuf::from(std::env::var_os("APPDATA").ok_or("APPDATA is missing")?)
        .join("Microsoft/Windows/Start Menu/Programs").join(format!("Pleamar validation {}.lnk", std::process::id()));
    std::fs::OpenOptions::new().create_new(true).write(true).open(&shortcut)?;
    let cleanup = Cleanup { shortcut, app };
    println!("stage: register owned shortcut");
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(PCWSTR(wide(&std::env::current_exe()?).as_ptr()))?;
        link.SetArguments(w!("--list"))?;
        let store: IPropertyStore = link.cast()?;
        let key = PROPERTYKEY { fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3), pid: 5 };
        store.SetValue(&key, &PROPVARIANT::from(cleanup.app.to_string().as_str()))?;
        store.Commit()?;
        link.cast::<IPersistFile>()?.Save(PCWSTR(wide(&cleanup.shortcut).as_ptr()), true)?;
    }
    println!("stage: create owned toast");
    let title = format!("Pleamar notification validation {}", std::process::id());
    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(format!("<toast><visual><binding template='ToastGeneric'><text>{title}</text><text>España · 海 · 🚀 — native Windows round trip</text></binding></visual><audio silent='true'/></toast>")))?;
    let toast = ToastNotification::CreateToastNotification(&document)?;
    toast.SetTag(&HSTRING::from("owned"))?;
    toast.SetGroup(&HSTRING::from("validation"))?;
    ToastNotificationManager::CreateToastNotifierWithId(&cleanup.app)?.Show(&toast)?;
    println!("stage: read through native notification service");
    let deadline = Instant::now() + Duration::from_secs(15);
    let id = loop {
        let SysValue::List(list) = super::query("notifications.list", &[])? else { return Err("expected notification list".into()); };
        if let Some(row) = list.iter().find(|row| field(row, "title") == Some(&SysValue::Text(title.clone()))) {
            assert_eq!(field(row, "title"), Some(&SysValue::Text(title.clone())));
            assert_eq!(field(row, "body"), Some(&SysValue::Text("España · 海 · 🚀 — native Windows round trip".into())));
            // Windows may omit AppInfo for this classic desktop toast. There
            // must not be an invented sender or an unusable opening action.
            if field(row, "app") == Some(&SysValue::Text(String::new())) {
                assert_eq!(field(row, "actions"), Some(&SysValue::List(vec![])));
                println!("PASS: classic toast without AppInfo retains text and omits unsupported app actions");
            }
            break field(row, "id").ok_or("missing ID")?.clone();
        }
        if Instant::now() >= deadline { return Err("own toast did not arrive in Windows notification center".into()); }
        std::thread::sleep(Duration::from_millis(200));
    };
    println!("PASS: native toast arrived with Unicode title/body; no other notification content printed");
    if std::env::var_os("PLEAMAR_NOTIFICATION_HOLD").is_some() {
        println!("OWN_APP={}", cleanup.app);
        println!("OWN_TITLE={title}");
        std::thread::sleep(Duration::from_secs(120));
    }
    println!("stage: dismiss through native notification service");
    super::command("notifications.dismiss", &[id.clone()])?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let SysValue::List(list) = super::query("notifications.list", &[])? else { return Err("expected list".into()); };
        if !list.iter().any(|row| field(row, "id") == Some(&id)) { break; }
        if Instant::now() >= deadline { return Err("own notification was not dismissed".into()); }
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("PASS: native dismissal removed only the owned toast; shortcut/history cleanup follows");
    Ok(())
}
