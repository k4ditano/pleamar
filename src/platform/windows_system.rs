//! Windows services whose data can be obtained without external programs.
use super::SysValue;
use std::mem::size_of;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::sync::OnceLock;
use windows::Win32::System::JobObjects::*;
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::core::PCWSTR;

pub struct Apartment;
impl Apartment {
    pub fn new() -> Result<Self, String> {
        unsafe { windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED) }
            .map(|_| Self).map_err(|e| e.to_string())
    }
}
impl Drop for Apartment {
    fn drop(&mut self) { unsafe { windows::Win32::System::WinRT::RoUninitialize(); } }
}

/// Attach the runtime itself before any logic can spawn a process. Descendants
/// inherit the job at creation, avoiding a spawn/assign race. The handle is
/// private and closes when the runtime exits, even after forced termination.
pub fn contain_children() -> Result<(), String> {
    static JOB: OnceLock<OwnedHandle> = OnceLock::new();
    if JOB.get().is_some() { return Ok(()); }
    unsafe {
        let job = CreateJobObjectW(None, PCWSTR::null()).map_err(|e| e.to_string())?;
        let owned = OwnedHandle::from_raw_handle(job.0);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        // Logic helpers stay in the job. The application launcher explicitly
        // breaks away: user applications must survive closing their launcher.
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
        SetInformationJobObject(job, JobObjectExtendedLimitInformation, &limits as *const _ as _, size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32).map_err(|e| e.to_string())?;
        AssignProcessToJobObject(job, GetCurrentProcess()).map_err(|e| e.to_string())?;
        let _ = JOB.set(owned);
    }
    Ok(())
}

pub fn service(name: &str, notify: Box<dyn Fn(SysValue) + Send>) -> bool {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    #[derive(Default)]
    struct Subscribers {
        latest: Option<SysValue>,
        callbacks: Vec<Box<dyn Fn(SysValue) + Send>>,
    }
    // A declarative service and sys.watch often request the same data. Share
    // the native worker and its snapshot instead of polling COM twice.
    static SERVICES: OnceLock<Mutex<HashMap<String, Arc<Mutex<Subscribers>>>>> = OnceLock::new();
    let mut services = SERVICES.get_or_init(Mutex::default).lock().unwrap();
    if let Some(subscribers) = services.get(name) {
        let mut subscribers = subscribers.lock().unwrap();
        if let Some(value) = &subscribers.latest { notify(value.clone()); }
        subscribers.callbacks.push(notify);
        return true;
    }
    let subscribers = Arc::new(Mutex::new(Subscribers { latest: None, callbacks: vec![notify] }));
    let forward = subscribers.clone();
    if !start_service(name, Box::new(move |value| {
        let mut listeners = forward.lock().unwrap();
        listeners.latest = Some(value.clone());
        for callback in &listeners.callbacks { callback(value.clone()); }
    })) { return false; }
    services.insert(name.into(), subscribers);
    true
}

fn start_service(name: &str, notify: Box<dyn Fn(SysValue) + Send>) -> bool {
    if name == "audio" { return super::windows_audio::service(notify); }
    if name == "notifications" { return super::windows_notifications::service(notify); }
    if name == "hotkeys" { return super::windows_hotkeys::service(notify); }
    let native: Option<fn() -> windows::core::Result<SysValue>> = match name {
        "notifications.state" => Some(super::windows_notifications::state),
        "media" => Some(super::windows_media::media),
        "network" => Some(super::windows_media::network),
        "network.wifi" => Some(super::windows_wifi::read),
        "bluetooth" => Some(super::windows_bluetooth::read),
        "brightness" => Some(super::windows_brightness::read),
        "apps" => Some(super::windows_shell::apps),
        "tray" => Some(super::windows_tray::read),
        "tray.state" => Some(super::windows_tray::state),
        _ => None,
    };
    if let Some(read) = native {
        let interval = match name { "network" | "network.wifi" => 3000, "brightness" => 1500, "bluetooth" | "tray" | "tray.state" => 2000, "apps" => 30000, _ => 1000 };
        let name = name.to_owned();
        return std::thread::Builder::new().name(name.clone()).spawn(move || {
            let _apartment = match Apartment::new() {
                Ok(apartment) => apartment,
                Err(error) => { eprintln!("windows · {name} unavailable: {error}"); return; }
            };
            // The first catalog/DDC/COM read can be just as slow as any later
            // read. Never perform it inside sys.watch on the Lua thread.
            let mut last = None;
            let mut warned = false;
            loop {
                match read() {
                    Ok(value) => { warned = false; if last.as_ref() != Some(&value) { notify(value.clone()); last = Some(value); } }
                    Err(e) => {
                        // A lost player must not leave stale, clickable controls.
                        if name == "media" {
                            let value = super::windows_media::unavailable(&e.to_string());
                            if last.as_ref() != Some(&value) { notify(value.clone()); last = Some(value); }
                        }
                        if !warned { eprintln!("windows · {name}: {e}"); warned = true; }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(interval));
            }
        }).is_ok();
    }
    let read: fn() -> Option<SysValue> = match name {
        "battery" => battery,
        "window" => active_window,
        _ => return false,
    };
    std::thread::Builder::new().name(name.into()).spawn(move || {
        let mut last = None;
        loop {
            if let Some(value) = read() {
                if last.as_ref() != Some(&value) { notify(value.clone()); last = Some(value); }
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }).is_ok()
}

pub fn query(name: &str, args: &[SysValue]) -> Result<SysValue, String> {
    if name.starts_with("tray.") { return super::windows_tray::query(name, args); }
    if name.starts_with("notifications.") { return super::windows_notifications::query(name, args); }
    if name.starts_with("recording.") { return super::windows_recording::query(name, args); }
    if name.starts_with("screenshot.") { return super::windows_capture::query(name, args); }
    if name == "search.files" { return super::windows_search::query(args); }
    if name.starts_with("wallpaper.") {
        let _apartment = Apartment::new()?;
        return super::windows_wallpaper::query(name, args);
    }
    if !args.is_empty() { return Err(format!("{name} takes no arguments")); }
    let _apartment = Apartment::new()?;
    match name {
        "audio.state" => super::windows_audio::read().map_err(|e| e.to_string()),
        "media.state" => super::windows_media::media().map_err(|e| e.to_string()),
        "network.state" => super::windows_media::network().map_err(|e| e.to_string()),
        "network.wifi" => super::windows_wifi::read().map_err(|e| e.to_string()),
        "bluetooth.state" => super::windows_bluetooth::read().map_err(|e| e.to_string()),
        "bluetooth.discover" => super::windows_bluetooth::discover(),
        "brightness.state" => super::windows_brightness::read().map_err(|e| e.to_string()),
        "apps.list" => super::windows_shell::apps().map_err(|e| e.to_string()),
        "window.state" => super::windows_windows::read(),
        _ => Err(format!("Windows cannot answer '{name}' yet")),
    }
}

pub fn command(name: &str, args: &[SysValue]) -> Result<(), String> {
    if name.starts_with("tray.") { return super::windows_tray_actions::command(name, args); }
    if name == "window.restore" { return super::windows_windows::restore(args); }
    if name.starts_with("hotkeys.") { return super::windows_hotkeys::command(name, args); }
    if name == "shell.open" {
        let [SysValue::Text(path)] = args else { return Err("shell.open takes a path or URL".into()); };
        if path.contains('\0') { return Err("paths cannot contain NUL".into()); }
        return super::windows_shell::open(path);
    }
    let settings = match name {
        "audio.settings" => Some("ms-settings:sound"),
        "network.settings" => Some("ms-settings:network-status"),
        "bluetooth.settings" => Some("ms-settings:bluetooth"),
        "brightness.settings" => Some("ms-settings:display"),
        "session.settings" => Some("ms-settings:powersleep"),
        _ => None,
    };
    if let Some(uri) = settings {
        if !args.is_empty() { return Err(format!("{name} takes no arguments")); }
        return super::windows_shell::open(uri);
    }
    let _apartment = Apartment::new()?;
    if name == "apps.launch" {
        let [SysValue::Text(id)] = args else { return Err("apps.launch takes a catalog identifier".into()); };
        return super::windows_shell::launch(id);
    }
    if name.starts_with("notifications.") { return super::windows_notifications::command(name, args); }
    if name.starts_with("wallpaper.") { return super::windows_wallpaper::command(name, args); }
    if name.starts_with("session.") { return session(name, args); }
    if name.starts_with("audio.") { return super::windows_audio::command(name, args); }
    if name.starts_with("network.") { return super::windows_wifi::command(name, args); }
    if name.starts_with("bluetooth.") { return super::windows_bluetooth::command(name, args); }
    if name.starts_with("brightness.") { return super::windows_brightness::command(name, args); }
    if name.starts_with("media.") { return super::windows_media::command(name, args); }
    Err(format!("Windows cannot do '{name}' yet"))
}

fn session(name: &str, args: &[SysValue]) -> Result<(), String> {
    use windows::Win32::{Foundation::*, Security::*, System::{Threading::OpenProcessToken, Shutdown::*}};
    if !args.is_empty() { return Err(format!("{name} takes no arguments")); }
    if !matches!(name, "session.lock" | "session.logout" | "session.suspend" | "session.reboot" | "session.poweroff") {
        return Err(format!("unknown session action: {name}"));
    }
    unsafe {
        if name == "session.lock" { return LockWorkStation().map_err(|e| e.to_string()); }
        // Logoff needs no shutdown privilege. Never force applications closed.
        if name == "session.logout" { return ExitWindowsEx(EWX_LOGOFF, SHTDN_REASON_FLAG_PLANNED).map_err(|e| e.to_string()); }
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token).map_err(|e| e.to_string())?;
        let owned = OwnedHandle::from_raw_handle(token.0);
        let mut id = LUID::default();
        LookupPrivilegeValueW(PCWSTR::null(), SE_SHUTDOWN_NAME, &mut id).map_err(|e| e.to_string())?;
        let requested = TOKEN_PRIVILEGES { PrivilegeCount: 1, Privileges: [LUID_AND_ATTRIBUTES { Luid: id, Attributes: SE_PRIVILEGE_ENABLED }] };
        let mut previous = TOKEN_PRIVILEGES::default();
        let mut length = 0;
        AdjustTokenPrivileges(token, false, Some(&requested), size_of::<TOKEN_PRIVILEGES>() as u32, Some(&mut previous), Some(&mut length)).map_err(|e| e.to_string())?;
        let access = GetLastError();
        struct Restore(HANDLE, TOKEN_PRIVILEGES, OwnedHandle);
        impl Drop for Restore { fn drop(&mut self) { unsafe { let _ = AdjustTokenPrivileges(self.0, false, Some(&self.1), 0, None, None); } let _ = &self.2; } }
        let _restore = Restore(token, previous, owned);
        if access == ERROR_NOT_ALL_ASSIGNED { return Err("Windows policy does not grant this account shutdown permission".into()); }
        if name == "session.suspend" {
            return if windows::Win32::System::Power::SetSuspendState(false, false, false) { Ok(()) } else { Err(windows::core::Error::from_thread().to_string()) };
        }
        ExitWindowsEx(if name == "session.reboot" { EWX_REBOOT } else { EWX_POWEROFF }, SHTDN_REASON_FLAG_PLANNED).map_err(|e| e.to_string())
    }
}

fn battery() -> Option<SysValue> {
    let mut status = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut status).ok()?; }
    let mut fields = Vec::new();
    if status.BatteryFlag != 255 {
        fields.push(("present".into(), SysValue::Bool(status.BatteryFlag & 128 == 0)));
    }
    if status.BatteryLifePercent <= 100 { fields.push(("percent".into(), SysValue::Num(status.BatteryLifePercent as f64))); }
    if status.BatteryFlag != 255 { fields.push(("charging".into(), SysValue::Bool(status.BatteryFlag & 8 != 0))); }
    Some(SysValue::Map(fields))
}

fn active_window() -> Option<SysValue> { super::windows_windows::read().ok() }

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::process::{Command, Stdio};
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};

    #[test]
    #[ignore = "read-only per-service heap diagnostic; requires PLEAMAR_PROFILE_SERVICE"]
    fn native_service_heap_profile() {
        use std::time::{Duration, Instant};
        use windows::{core::{s, w, BOOL}, Win32::{Foundation::HANDLE,
            System::{LibraryLoader::{GetModuleHandleW, GetProcAddress}, Memory::*}}};
        let name = std::env::var("PLEAMAR_PROFILE_SERVICE").expect("select a read-only native query");
        let interval = match name.as_str() {
            "audio.state" => 250, "media.state" => 1000, "network.state" | "network.wifi" => 3000,
            "bluetooth.state" | "tray.list" => 2000, "brightness.state" => 1500,
            "window.state" => 500, "apps.list" => 30000,
            _ => panic!("only read-only service queries may be profiled"),
        };
        let seconds: u64 = std::env::var("PLEAMAR_PROFILE_SECONDS").unwrap_or("30".into()).parse().unwrap();
        assert!((1..=600).contains(&seconds));
        let _apartment = Apartment::new().unwrap();
        type Summary = unsafe extern "system" fn(HANDLE, u32, *mut HEAP_SUMMARY) -> BOOL;
        let summary: Summary = unsafe {
            let module = GetModuleHandleW(w!("kernel32.dll")).unwrap();
            let Some(function) = GetProcAddress(module, s!("HeapSummary")) else {
                println!("NOT RUN: HeapSummary requires Windows build 20348 or newer"); return;
            };
            std::mem::transmute(function)
        };
        let heap = unsafe { GetProcessHeap() }.unwrap();
        let sample = |calls: u64, errors: u64, elapsed: f64| {
            let mut info = HEAP_SUMMARY { cb: size_of::<HEAP_SUMMARY>() as u32, ..Default::default() };
            assert!(unsafe { summary(heap, 0, &mut info) }.as_bool());
            println!("{}", serde_json::json!({"service": name, "calls": calls, "errors": errors,
                "seconds": elapsed, "allocated": info.cbAllocated, "committed": info.cbCommitted,
                "reserved": info.cbReserved}));
        };
        // Warm the apartment/query and stdout before comparing repeated reads.
        for _ in 0..3 { drop(query(&name, &[])); }
        sample(0, 0, 0.0);
        let started = Instant::now();
        let (mut calls, mut errors, mut last_sample) = (0, 0, Instant::now());
        while started.elapsed() < Duration::from_secs(seconds) {
            let read = Instant::now();
            if query(&name, &[]).is_err() { errors += 1; }
            calls += 1;
            if last_sample.elapsed() >= Duration::from_secs(5) {
                sample(calls, errors, started.elapsed().as_secs_f64());
                last_sample = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(interval).saturating_sub(read.elapsed()));
        }
        sample(calls, errors, started.elapsed().as_secs_f64());
        // No plateau assertion: native caches and the desktop state vary. These
        // measurements attribute work; successful reads alone are not a leak test.
    }

    #[test]
    #[ignore = "read-only inventory of the local desktop hardware"]
    fn live_native_control_inventory() {
        for name in ["network.wifi", "bluetooth.state", "brightness.state"] {
            let value = query(name, &[]).unwrap();
            println!("{name}: {value:?}");
            assert!(matches!(value, SysValue::Map(_)));
        }
    }

    #[test]
    fn forced_exit_terminates_job_descendants() {
        let mut parent = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "platform::windows_system::tests::job_probe", "--nocapture"])
            .stdout(Stdio::piped()).spawn().unwrap();
        let mut child_id = None;
        for line in std::io::BufReader::new(parent.stdout.take().unwrap()).lines() {
            if let Some(id) = line.unwrap().strip_prefix("CHILD=") { child_id = Some(id.parse::<u32>().unwrap()); break; }
        }
        let id = child_id.expect("job probe must report its child");
        unsafe {
            let child = OpenProcess(PROCESS_SYNCHRONIZE, false, id).unwrap();
            parent.kill().unwrap();
            parent.wait().unwrap();
            let stopped = WaitForSingleObject(child, 5000);
            let _ = CloseHandle(child);
            assert_eq!(stopped, WAIT_OBJECT_0, "child survived forced parent exit");
        }
    }

    #[test]
    #[ignore = "subprocess helper for forced_exit_terminates_job_descendants"]
    fn job_probe() {
        contain_children().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "platform::windows_system::tests::job_leaf"])
            .stdout(Stdio::null()).spawn().unwrap();
        println!("CHILD={}", child.id());
        let _ = child.wait();
    }

    #[test]
    #[ignore = "subprocess helper for forced_exit_terminates_job_descendants"]
    fn job_leaf() { std::thread::sleep(std::time::Duration::from_secs(30)); }
}
