//! Windows' notification center remains the owner. We mirror its live toasts,
//! and never claim to expose their private activation arguments or retention.
use super::SysValue;
use std::{collections::HashMap, sync::{Mutex, OnceLock, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use windows::{ApplicationModel::Package, UI::Notifications::{KnownNotificationBindings, NotificationKinds, UserNotification,
    Management::{UserNotificationListener, UserNotificationListenerAccessStatus as Access}}};

const MAX_NOTICES: usize = 256;
const MAX_TEXT: usize = 8192;
const WINDOWS_EPOCH: i64 = 116_444_736_000_000_000;
static REQUESTING: AtomicBool = AtomicBool::new(false);
static ACCESS_ERROR: Mutex<Option<String>> = Mutex::new(None);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Identity { native: u32, created: i64, app: String }
#[derive(Default)]
struct Catalog { next: u64, entries: HashMap<u64, Identity> }
impl Catalog {
    fn reconcile(&mut self, keys: &[Identity]) -> Vec<u64> {
        // Windows can resolve a classic toast's AppInfo a little after its
        // visual arrives. That metadata refresh must not become a new toast.
        let old: HashMap<_, _> = self.entries.drain().map(|(id, key)| ((key.native, key.created), id)).collect();
        keys.iter().map(|key| {
            let id = old.get(&(key.native, key.created)).copied().unwrap_or_else(|| { self.next += 1; self.next });
            self.entries.insert(id, key.clone());
            id
        }).collect()
    }
}
fn catalog() -> &'static Mutex<Catalog> {
    static CATALOG: OnceLock<Mutex<Catalog>> = OnceLock::new();
    CATALOG.get_or_init(Mutex::default)
}
fn identity(n: &UserNotification) -> windows::core::Result<Identity> {
    // Classic desktop toasts may return E_NOTIMPL for AppInfo. Their native
    // ID/timestamp and visual still work; sender metadata is optional.
    let app = n.AppInfo().and_then(|app| app.AppUserModelId()).map(|id| id.to_string()).unwrap_or_default();
    Ok(Identity { native: n.Id()?, created: n.CreationTime()?.UniversalTime, app })
}
fn same_notification(previous: &Identity, current: &Identity) -> bool {
    previous.native == current.native && previous.created == current.created
        && (previous.app.is_empty() || current.app.is_empty() || previous.app == current.app)
}
fn map(fields: Vec<(&str, SysValue)>) -> SysValue {
    SysValue::Map(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
fn text(s: impl Into<String>) -> SysValue { SysValue::Text(s.into()) }
fn access_name(access: Access) -> &'static str {
    if access == Access::Allowed { "allowed" } else if access == Access::Denied { "denied" } else { "unspecified" }
}
pub fn state() -> windows::core::Result<SysValue> {
    let package = Package::Current().and_then(|p| p.Id()?.Name()).ok();
    let status = UserNotificationListener::Current().and_then(|l| l.GetAccessStatus());
    let (access, error) = match status {
        Ok(access) => (access_name(access), ACCESS_ERROR.lock().unwrap().clone().unwrap_or_default()),
        Err(error) => ("unavailable", error.to_string()),
    };
    Ok(map(vec![
        ("access", text(access)), ("requesting", SysValue::Bool(REQUESTING.load(Ordering::Acquire))),
        ("packaged", SysValue::Bool(package.is_some())), ("error", text(error)),
        ("retention", SysValue::Bool(false)), ("original_actions", SysValue::Bool(false)),
        ("limit", SysValue::Num(MAX_NOTICES as f64)),
    ]))
}
fn allowed() -> Result<UserNotificationListener, String> {
    let listener = UserNotificationListener::Current().map_err(|e| e.to_string())?;
    let access = listener.GetAccessStatus().map_err(|e| e.to_string())?;
    if access != Access::Allowed { return Err(format!("Notification access is {}", access_name(access))); }
    Ok(listener)
}

fn bounded(s: String) -> String { s.chars().take(MAX_TEXT).collect() }
fn row(n: &UserNotification, key: &Identity, id: u64) -> windows::core::Result<SysValue> {
    let app = n.AppInfo().and_then(|app| app.DisplayInfo()).and_then(|info| info.DisplayName())
        .map(|name| name.to_string()).unwrap_or_default();
    let mut lines = Vec::new();
    // Some app-specific templates have no generic visual. Keep their identity
    // and app name; do not substitute invented notification content.
    if let Ok(binding) = n.Notification()?.Visual()?.GetBinding(&KnownNotificationBindings::ToastGeneric()?) {
        for line in binding.GetTextElements()?.into_iter().take(32) {
            lines.push(bounded(line.Text()?.to_string()));
        }
    }
    let title = if lines.is_empty() { String::new() } else { lines.remove(0) };
    let actions = if key.app.is_empty() { vec![] } else {
        vec![map(vec![("key", text("open-app")), ("label", text("Open app"))])]
    };
    Ok(map(vec![
        ("id", SysValue::Num(id as f64)), ("app", text(app)), ("title", text(title)),
        ("body", text(bounded(lines.join("\n")))), ("icon", text(if key.app.is_empty() { String::new() } else { format!("windows-app:{}", key.app) })),
        ("image", text("")), ("time", SysValue::Num((key.created - WINDOWS_EPOCH) as f64 / 10_000_000.0)),
        ("actions", SysValue::List(actions)),
    ]))
}

pub fn read() -> Result<SysValue, String> {
    static READING: Mutex<()> = Mutex::new(());
    let _reading = READING.lock().unwrap();
    let listener = allowed()?;
    let pending = listener.GetNotificationsAsync(NotificationKinds::Toast).map_err(|e| e.to_string())?;
    // A stuck shell must not occupy the scene's service queue indefinitely.
    let deadline = Instant::now() + Duration::from_secs(5);
    while pending.Status().map_err(|e| e.to_string())?.0 == 0 {
        if Instant::now() >= deadline { let _ = pending.Cancel(); return Err("Notification enumeration timed out".into()); }
        std::thread::sleep(Duration::from_millis(10));
    }
    let notifications = pending.GetResults().map_err(|e| e.to_string())?;
    if listener.GetAccessStatus().map_err(|e| e.to_string())? != Access::Allowed {
        return Err("Notification access was revoked".into());
    }
    let mut notices = Vec::new();
    for n in notifications {
        let key = identity(&n).map_err(|e| e.to_string())?;
        notices.push((key, n));
    }
    notices.sort_by(|a, b| b.0.created.cmp(&a.0.created).then_with(|| b.0.native.cmp(&a.0.native)));
    notices.truncate(MAX_NOTICES);
    let mut catalog = catalog().lock().unwrap();
    let ids = catalog.reconcile(&notices.iter().map(|(key, _)| key.clone()).collect::<Vec<_>>());
    notices.iter().zip(ids).map(|((key, n), id)| row(n, key, id).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>().map(SysValue::List)
}

pub fn service(notify: Box<dyn Fn(SysValue) + Send>) -> bool {
    std::thread::Builder::new().name("notifications".into()).spawn(move || {
        let _apartment = match super::windows_system::Apartment::new() {
            Ok(value) => value, Err(error) => { *ACCESS_ERROR.lock().unwrap() = Some(error); return; }
        };
        let mut previous = None;
        loop {
            let value = match read() {
                Ok(value) => { *ACCESS_ERROR.lock().unwrap() = None; value }
                Err(error) => {
                    // Empty live state on revocation/failure, accompanied by a
                    // separate status channel. Never retain stale actionable IDs.
                    catalog().lock().unwrap().entries.clear();
                    *ACCESS_ERROR.lock().unwrap() = Some(error);
                    SysValue::List(vec![])
                }
            };
            if previous.as_ref() != Some(&value) { notify(value.clone()); previous = Some(value); }
            std::thread::sleep(Duration::from_secs(1));
        }
    }).is_ok()
}

pub fn query(name: &str, args: &[SysValue]) -> Result<SysValue, String> {
    if !args.is_empty() { return Err(format!("{name} takes no arguments")); }
    let _apartment = super::windows_system::Apartment::new()?;
    match name {
        "notifications.publisher" => super::windows_toasts::state(),
        "notifications.state" => state().map_err(|e| e.to_string()),
        "notifications.list" => read(),
        _ => Err(format!("Unknown notification query: {name}")),
    }
}

fn parse_id(value: &SysValue) -> Result<u64, String> {
    if let SysValue::Num(id) = value {
        if id.is_finite() && *id >= 1.0 && *id <= 9_007_199_254_740_991.0 && id.fract() == 0.0 { return Ok(*id as u64); }
    }
    Err("notifications requires an ID from the current notification list".into())
}
pub fn command(name: &str, args: &[SysValue]) -> Result<(), String> {
    if name == "notifications.publish" { return super::windows_toasts::publish(args); }
    if name == "notifications.request_access" {
        if !args.is_empty() { return Err(format!("{name} takes no arguments")); }
        if REQUESTING.swap(true, Ordering::AcqRel) { return Err("Notification access request is already pending".into()); }
        if let Err(error) = super::windows::on_ui_thread(Box::new(request_access)) {
            REQUESTING.store(false, Ordering::Release);
            return Err(error);
        }
        return Ok(());
    }
    let (id, open) = match (name, args) {
        ("notifications.dismiss", [id]) => (parse_id(id)?, false),
        ("notifications.invoke", [id, SysValue::Text(action)]) if action == "open-app" => (parse_id(id)?, true),
        ("notifications.keep", _) => return Err("Windows owns toast expiration; retention cannot be changed by a listener".into()),
        _ => return Err(format!("Unsupported notification command or arguments: {name}")),
    };
    let key = catalog().lock().unwrap().entries.get(&id).cloned()
        .ok_or("This notification has expired or is no longer in the live list")?;
    let listener = allowed()?;
    let current = listener.GetNotification(key.native).map_err(|e| e.to_string())?;
    if !same_notification(&key, &identity(&current).map_err(|e| e.to_string())?) {
        return Err("Windows replaced this notification; the old action was cancelled".into());
    }
    if open { super::windows_shell::launch(&key.app) }
    else { listener.RemoveNotification(key.native).map_err(|e| e.to_string()) }
}
fn request_access() {
    let result = UserNotificationListener::Current().and_then(|listener| listener.RequestAccessAsync())
        .and_then(|operation| operation.when(|result| {
            *ACCESS_ERROR.lock().unwrap() = result.err().map(|e| e.to_string());
            REQUESTING.store(false, Ordering::Release);
        }));
    if let Err(error) = result {
        *ACCESS_ERROR.lock().unwrap() = Some(error.to_string());
        REQUESTING.store(false, Ordering::Release);
    }
}

#[cfg(test)]
#[path = "windows_notifications_test.rs"]
mod live_tests;

#[cfg(test)]
mod tests {
    use super::*;
    fn key(native: u32, created: i64, app: &str) -> Identity { Identity { native, created, app: app.into() } }
    #[test]
    fn reused_os_ids_cannot_address_replacement_notifications() {
        let mut catalog = Catalog::default();
        let a = key(7, 100, "app.one");
        assert_eq!(catalog.reconcile(&[a.clone()]), [1]);
        assert_eq!(catalog.reconcile(&[key(8, 101, "app.two"), a.clone()]), [2, 1]);
        assert_eq!(catalog.reconcile(&[key(7, 102, "app.one")]), [3]);
        assert!(!catalog.entries.contains_key(&1));
        assert!(catalog.reconcile(&[]).is_empty());
        assert_eq!(catalog.reconcile(&[a]), [4]);
    }
    #[test]
    fn late_sender_metadata_preserves_the_live_id() {
        let mut catalog = Catalog::default();
        let pending = key(7, 100, "");
        let resolved = key(7, 100, "app.one");
        assert_eq!(catalog.reconcile(&[pending.clone()]), [1]);
        assert_eq!(catalog.reconcile(&[resolved.clone()]), [1]);
        assert!(same_notification(&pending, &resolved));
        assert!(same_notification(&resolved, &pending));
        assert!(!same_notification(&resolved, &key(7, 100, "app.other")));
        assert!(!same_notification(&pending, &key(7, 101, "app.one")));
    }
    #[test]
    fn rejects_untrusted_notification_ids() {
        for id in [f64::NAN, f64::INFINITY, -1.0, 0.0, 1.5, 9_007_199_254_740_992.0] {
            assert!(parse_id(&SysValue::Num(id)).is_err());
        }
        assert!(parse_id(&text("1")).is_err());
        assert_eq!(parse_id(&SysValue::Num(123.0)).unwrap(), 123);
    }
    #[test]
    #[ignore = "read-only local notification access; prints counts, never content"]
    fn live_notification_inventory() {
        let _apartment = super::super::windows_system::Apartment::new().unwrap();
        println!("status: {:?}", state().unwrap());
        let SysValue::List(list) = read().unwrap() else { panic!("expected list") };
        println!("native notification count: {}", list.len());
        assert!(list.len() <= MAX_NOTICES);
    }
}
