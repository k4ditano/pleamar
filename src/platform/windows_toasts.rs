//! Publish scene-owned reminders through Windows, independently of listener consent.
use super::SysValue;
use std::{path::Path, sync::{Arc, Mutex}, time::{Duration, Instant}};
use windows::{core::{GUID, HSTRING, Interface, PCWSTR}, Data::Xml::Dom::XmlDocument,
    Foundation::TypedEventHandler, UI::Notifications::{NotificationSetting, ToastFailedEventArgs,
        ToastNotification, ToastNotificationManager}, Win32::{Foundation::PROPERTYKEY,
        System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER, IPersistFile, STGM_READWRITE, StructuredStorage::PROPVARIANT},
        UI::Shell::{IShellLinkW, ShellLink, PropertiesSystem::IPropertyStore,
            SHChangeNotify, SHCNE_UPDATEITEM, SHCNF_PATHW, SHCNF_FLUSH}}};

const GROUP: &str = "pleamar-scenes";
const APP_ID: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3), pid: 5 };

fn identity_for(path: &Path) -> String {
    // Stable across builds, distinct between portable installations. Never use
    // another application's AUMID merely because its notifications are enabled.
    let mut hash = 0xcbf29ce484222325u64;
    for byte in path.to_string_lossy().replace('/', "\\").to_lowercase().bytes() {
        hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
    }
    format!("org.pleamar.desktop.{hash:016x}")
}
fn app_id() -> Result<HSTRING, String> {
    std::env::current_exe().map(|path| HSTRING::from(identity_for(&path))).map_err(|e| e.to_string())
}
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain([0]).collect()
}

/// Installer-only: preserve an existing shortcut's target, arguments and icon.
pub fn register_shortcut(path: &Path) -> Result<String, String> {
    register_for(path, &app_id()?)
}
fn register_for(path: &Path, id: &HSTRING) -> Result<String, String> {
    if !path.is_file() || !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("lnk")) {
        return Err("notification registration needs an existing .lnk shortcut in the Start menu".into());
    }
    let _apartment = super::windows_system::Apartment::new()?;
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        let file: IPersistFile = link.cast().map_err(|e| e.to_string())?;
        let path = wide(path);
        file.Load(PCWSTR(path.as_ptr()), STGM_READWRITE).map_err(|e| format!("Cannot load notification shortcut: {e}"))?;
        let store: IPropertyStore = link.cast().map_err(|e| e.to_string())?;
        store.SetValue(&APP_ID, &PROPVARIANT::from(id.to_string().as_str())).map_err(|e| format!("Cannot assign notification identity: {e}"))?;
        store.Commit().map_err(|e| e.to_string())?;
        file.Save(PCWSTR(path.as_ptr()), true).map_err(|e| format!("Cannot save notification shortcut: {e}"))?;
        SHChangeNotify(SHCNE_UPDATEITEM, SHCNF_PATHW | SHCNF_FLUSH, Some(path.as_ptr().cast()), None);
    }
    Ok(id.to_string())
}

fn setting_name(value: NotificationSetting) -> &'static str {
    if value == NotificationSetting::Enabled { "enabled" }
    else if value == NotificationSetting::DisabledForApplication { "disabled-for-application" }
    else if value == NotificationSetting::DisabledForUser { "disabled-for-user" }
    else if value == NotificationSetting::DisabledByGroupPolicy { "disabled-by-policy" }
    else if value == NotificationSetting::DisabledByManifest { "disabled-by-manifest" }
    else { "unavailable" }
}
pub fn state() -> Result<SysValue, String> {
    let id = app_id()?;
    let result = ToastNotificationManager::CreateToastNotifierWithId(&id).and_then(|n| n.Setting());
    let (setting, error) = match result {
        Ok(value) => (setting_name(value), String::new()),
        Err(error) => ("unavailable", error.to_string()),
    };
    Ok(SysValue::Map(vec![
        ("app_id".into(), SysValue::Text(id.to_string())),
        ("setting".into(), SysValue::Text(setting.into())),
        ("error".into(), SysValue::Text(error)),
    ]))
}

fn valid_text(value: &str, limit: usize) -> bool {
    value.chars().count() <= limit && value.chars().all(|c| matches!(c, '\t' | '\n' | '\r') || c >= ' ' && c != '\u{fffe}' && c != '\u{ffff}')
}
fn document(title: &str, body: &str) -> windows::core::Result<XmlDocument> {
    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from("<toast><visual><binding template='ToastGeneric'><text/><text/></binding></visual><audio silent='true'/></toast>"))?;
    let fields = doc.GetElementsByTagName(&HSTRING::from("text"))?;
    // Text nodes keep user content literal; no XML, command or URI interpolation.
    for (index, value) in [title, body].iter().enumerate() {
        fields.Item(index as u32)?.AppendChild(&doc.CreateTextNode(&HSTRING::from(*value))?)?;
    }
    Ok(doc)
}
fn in_history(id: &HSTRING, tag: &str, xml: &HSTRING) -> windows::core::Result<bool> {
    let history = match ToastNotificationManager::History()?.GetHistoryWithId(id) {
        Ok(history) => history,
        Err(error) if error.code().0 == 0x80070490u32 as i32 => return Ok(false),
        Err(error) => return Err(error),
    };
    for toast in history {
        if toast.Tag()? == tag && toast.Group()? == GROUP && toast.Content()?.GetXml()? == *xml { return Ok(true); }
    }
    Ok(false)
}
pub fn publish(args: &[SysValue]) -> Result<(), String> {
    publish_for(args, &app_id()?)
}
fn publish_for(args: &[SysValue], id: &HSTRING) -> Result<(), String> {
    let [SysValue::Text(title), SysValue::Text(body), SysValue::Text(tag)] = args else {
        return Err("notifications.publish takes title, body and a stable 1–16 character ASCII tag".into());
    };
    if title.trim().is_empty() || !valid_text(title, 256) || !valid_text(body, 2048)
        || tag.is_empty() || tag.len() > 16 || !tag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err("invalid notification text/tag (title: 256 characters; body: 2048; tag: 1–16 ASCII letters/digits/_/-)".into());
    }
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(id).map_err(|e| e.to_string())?;
    match notifier.Setting() {
        Ok(setting) if setting != NotificationSetting::Enabled => return Err(format!("Windows notification publishing is {}", setting_name(setting))),
        // Windows creates a new classic publisher's settings on its first Show.
        // This is unknown, not enabled: delivery still needs history confirmation.
        Err(error) if error.code().0 != 0x80070490u32 as i32 => return Err(error.to_string()),
        _ => {}
    }
    let doc = document(title, body).map_err(|e| e.to_string())?;
    let xml = doc.GetXml().map_err(|e| e.to_string())?;
    if in_history(id, tag, &xml).map_err(|e| e.to_string())? { return Ok(()); }
    let toast = ToastNotification::CreateToastNotification(&doc).map_err(|e| e.to_string())?;
    toast.SetTag(&HSTRING::from(tag)).map_err(|e| e.to_string())?;
    toast.SetGroup(&HSTRING::from(GROUP)).map_err(|e| e.to_string())?;
    let failure = Arc::new(Mutex::new(None));
    let observed = failure.clone();
    let token = toast.Failed(&TypedEventHandler::<ToastNotification, ToastFailedEventArgs>::new(move |_, args| {
        *observed.lock().unwrap() = Some(args.as_ref().map(|a| a.ErrorCode().map(|code| code.to_string()).unwrap_or_else(|e| e.to_string()))
            .unwrap_or_else(|| "Windows rejected the notification".into()));
        Ok(())
    })).map_err(|e| e.to_string())?;
    let result = (|| {
        notifier.Show(&toast).map_err(|e| e.to_string())?;
        // Show() only queues delivery. Confirm our own history entry; no access
        // to other applications' notifications or listener consent is required.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(error) = failure.lock().unwrap().clone() { return Err(error); }
            if in_history(id, tag, &xml).map_err(|e| e.to_string())? { return Ok(()); }
            if Instant::now() >= deadline { return Err("Windows did not confirm the reminder in notification history".into()); }
            std::thread::sleep(Duration::from_millis(50));
        }
    })();
    let _ = toast.RemoveFailed(token);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publisher_identity_and_literal_xml() {
        assert_eq!(identity_for(Path::new("C:/Marea ñ/bin/pleamar.exe")), identity_for(Path::new("c:\\marea ñ\\bin\\PLEAMAR.EXE")));
        assert_ne!(identity_for(Path::new("C:/one/pleamar.exe")), identity_for(Path::new("C:/two/pleamar.exe")));
        assert!(!valid_text("bad\0text", 256));
        assert!(!valid_text(&"x".repeat(257), 256));
        assert!(valid_text("España · 海 · 🚀\n& < >", 256));
        let _apartment = super::super::windows_system::Apartment::new().unwrap();
        let doc = document("<hello> & \"España\"", "海 🚀\n<script/>").unwrap();
        let nodes = doc.GetElementsByTagName(&HSTRING::from("text")).unwrap();
        assert_eq!(nodes.Item(0).unwrap().InnerText().unwrap(), "<hello> & \"España\"");
        assert_eq!(nodes.Item(1).unwrap().InnerText().unwrap(), "海 🚀\n<script/>");
        assert!(publish(&[]).is_err());
        assert!(publish(&[SysValue::Text("title".into()), SysValue::Text("body".into()), SysValue::Text("../invalid".into())]).is_err());
    }

    #[test]
    #[ignore = "registers an owned shortcut and publishes/removes a native validation toast"]
    fn native_publisher_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
        use windows::core::w;
        let _apartment = super::super::windows_system::Apartment::new()?;
        let shortcut = std::path::PathBuf::from(std::env::var_os("APPDATA").ok_or("APPDATA is missing")?)
            .join("Microsoft/Windows/Start Menu/Programs")
            .join(format!("Pleamar publisher validation {}.lnk", std::process::id()));
        std::fs::OpenOptions::new().write(true).create_new(true).open(&shortcut)?;
        let id = HSTRING::from(format!("{}.Test{:x}", app_id()?, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));
        let tag = format!("test{:08x}", std::process::id());
        struct Cleanup { path: std::path::PathBuf, id: HSTRING, tag: String }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                if let Ok(history) = ToastNotificationManager::History() {
                    let _ = history.RemoveGroupedTagWithId(&HSTRING::from(&self.tag), &HSTRING::from(GROUP), &self.id);
                }
                let _ = std::fs::remove_file(&self.path);
            }
        }
        let cleanup = Cleanup { path: shortcut, id, tag };
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(PCWSTR(wide(&std::env::current_exe()?).as_ptr()))?;
            link.SetArguments(w!("--list"))?;
            link.cast::<IPersistFile>()?.Save(PCWSTR(wide(&cleanup.path).as_ptr()), true)?;
        }
        assert_eq!(register_for(&cleanup.path, &cleanup.id)?, cleanup.id.to_string());
        let args = [SysValue::Text("Pleamar calendar validation · España & <海>".into()),
            SysValue::Text("Native publishing, not a simulated reminder. 🚀".into()), SysValue::Text(cleanup.tag.clone())];
        println!("initial publisher setting: {:?}", ToastNotificationManager::CreateToastNotifierWithId(&cleanup.id)?.Setting());
        publish_for(&args, &cleanup.id)?;
        publish_for(&args, &cleanup.id)?;
        let entries = ToastNotificationManager::History()?.GetHistoryWithId(&cleanup.id)?.into_iter()
            .filter(|n| n.Tag().ok().is_some_and(|tag| tag == cleanup.tag)).count();
        assert_eq!(entries, 1, "retry duplicated the owned reminder");
        println!("PASS: actual Windows history confirms escaped Unicode text; repeat tag has one entry");
        if std::env::var_os("PLEAMAR_NOTIFICATION_HOLD").is_some() {
            println!("Owned publisher validation toast is available for visual inspection");
            std::thread::sleep(Duration::from_secs(120));
        }
        Ok(())
    }
}
