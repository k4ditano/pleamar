//! Core Audio session instances: one application's stream, never the master.
use super::{enumerator, identifier, watch::{Wake, APPS, LEVELS}, SysValue};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use windows::core::{implement, Interface, Result, PCWSTR, PWSTR, GUID, BOOL, Ref};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{CoTaskMemFree, CLSCTX_ALL};

fn owned_text(value: Result<PWSTR>) -> Result<String> {
    let value = value?;
    let text = unsafe { value.to_string() };
    unsafe { CoTaskMemFree(Some(value.0 as _)); }
    Ok(text?)
}

#[implement(IAudioSessionEvents)]
struct Changed(Wake);
impl IAudioSessionEvents_Impl for Changed_Impl {
    fn OnDisplayNameChanged(&self, _: &PCWSTR, _: *const GUID) -> Result<()> { self.0.signal(APPS); Ok(()) }
    fn OnIconPathChanged(&self, _: &PCWSTR, _: *const GUID) -> Result<()> { self.0.signal(APPS); Ok(()) }
    fn OnSimpleVolumeChanged(&self, _: f32, _: BOOL, _: *const GUID) -> Result<()> { self.0.signal(LEVELS); Ok(()) }
    fn OnChannelVolumeChanged(&self, _: u32, _: *const f32, _: u32, _: *const GUID) -> Result<()> { self.0.signal(LEVELS); Ok(()) }
    fn OnGroupingParamChanged(&self, _: *const GUID, _: *const GUID) -> Result<()> { self.0.signal(APPS); Ok(()) }
    fn OnStateChanged(&self, _: AudioSessionState) -> Result<()> { self.0.signal(APPS); Ok(()) }
    fn OnSessionDisconnected(&self, _: AudioSessionDisconnectReason) -> Result<()> { self.0.signal(APPS); Ok(()) }
}

type Pending = Arc<Mutex<Vec<(String, IAudioSessionControl)>>>;

#[implement(IAudioSessionNotification)]
struct Added { device: String, pending: Pending, wake: Wake }
impl IAudioSessionNotification_Impl for Added_Impl {
    fn OnSessionCreated(&self, control: Ref<IAudioSessionControl>) -> Result<()> {
        if let Some(control) = control.as_ref() {
            let mut pending = self.pending.lock().unwrap();
            if pending.len() < 256 { pending.push((self.device.clone(), control.clone())); }
        }
        self.wake.signal(APPS);
        Ok(())
    }
}

struct Manager { control: IAudioSessionManager2, callback: IAudioSessionNotification }
impl Drop for Manager {
    fn drop(&mut self) { unsafe { let _ = self.control.UnregisterSessionNotification(&self.callback); } }
}

struct Session {
    device: String,
    control: IAudioSessionControl2,
    volume: ISimpleAudioVolume,
    callback: Option<IAudioSessionEvents>,
    binary: String,
    name: String,
}
impl Session {
    fn new(device: String, control: IAudioSessionControl2, wake: Option<&Wake>) -> Result<Self> {
        let pid = unsafe { control.GetProcessId()? };
        let binary = super::super::windows_media::process_identity(pid).map(|(path, _)| path).unwrap_or_default();
        let name = std::path::Path::new(&binary).file_stem().map(|p| p.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty()).unwrap_or_else(|| format!("Application {pid}"));
        let volume = control.cast()?;
        let callback = match wake {
            Some(wake) => {
                let callback: IAudioSessionEvents = Changed(wake.clone()).into();
                unsafe { control.RegisterAudioSessionNotification(&callback)?; }
                Some(callback)
            }
            None => None,
        };
        Ok(Self { device, control, volume, callback, binary, name })
    }

    fn row(&self, id: &str) -> Result<SysValue> {
        let title = owned_text(unsafe { self.control.GetDisplayName() }).unwrap_or_default();
        // Indirect resource strings are not user-facing titles. The executable
        // name is still available when a stream supplies no display name.
        let title = if title.starts_with('@') { String::new() } else { title };
        unsafe { Ok(SysValue::Map(vec![
            ("id".into(), SysValue::Text(id.to_owned())),
            ("name".into(), SysValue::Text(self.name.clone())),
            ("binary".into(), SysValue::Text(self.binary.clone())),
            ("icon".into(), SysValue::Text(String::new())),
            ("title".into(), SysValue::Text(title)),
            ("volume".into(), SysValue::Num(self.volume.GetMasterVolume()? as f64)),
            ("muted".into(), SysValue::Bool(self.volume.GetMute()?.as_bool())),
            ("playing".into(), SysValue::Bool(self.control.GetState()? == AudioSessionStateActive)),
        ])) }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        if let Some(callback) = &self.callback { unsafe { let _ = self.control.UnregisterAudioSessionNotification(callback); } }
    }
}

#[derive(Default)]
pub(super) struct Catalog {
    sessions: BTreeMap<String, Session>,
    managers: BTreeMap<String, Manager>,
    pending: Pending,
}
impl Catalog {
    fn add(&mut self, device: String, control: IAudioSessionControl, wake: &Wake) -> Result<()> {
        let control: IAudioSessionControl2 = control.cast()?;
        if unsafe { control.GetState()? } == AudioSessionStateExpired || unsafe { control.GetProcessId()? } == 0 {
            return Ok(());
        }
        let id = owned_text(unsafe { control.GetSessionInstanceIdentifier() })?;
        if !self.sessions.contains_key(&id) && self.sessions.len() < 256 {
            self.sessions.insert(id, Session::new(device, control, Some(wake))?);
        }
        Ok(())
    }

    pub fn refresh(&mut self, enumerator: &IMMDeviceEnumerator, wake: &Wake) -> Result<()> {
        let devices = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)? };
        let mut present = HashSet::new();
        let mut found = Vec::new();
        for i in 0..unsafe { devices.GetCount()? } {
            let device = unsafe { devices.Item(i)? };
            let id = identifier(&device)?;
            present.insert(id.clone());
            if !self.managers.contains_key(&id) {
                let control: IAudioSessionManager2 = unsafe { device.Activate(CLSCTX_ALL, None)? };
                let callback: IAudioSessionNotification = Added { device: id.clone(), pending: self.pending.clone(), wake: wake.clone() }.into();
                unsafe { control.RegisterSessionNotification(&callback)?; }
                self.managers.insert(id.clone(), Manager { control, callback });
            }
            let sessions = unsafe { self.managers[&id].control.GetSessionEnumerator()? };
            // GetCount enables delivery of new-session notifications.
            for n in 0..unsafe { sessions.GetCount()? } {
                if let Ok(control) = unsafe { sessions.GetSession(n) } { found.push((id.clone(), control)); }
            }
        }
        self.managers.retain(|id, _| present.contains(id));
        self.sessions.retain(|_, s| present.contains(&s.device)
            && unsafe { s.control.GetState() }.is_ok_and(|state| state != AudioSessionStateExpired));
        found.extend(std::mem::take(&mut *self.pending.lock().unwrap()));
        for (device, control) in found {
            if present.contains(&device) { let _ = self.add(device, control, wake); }
        }
        Ok(())
    }

    pub fn snapshot(&self) -> SysValue {
        SysValue::List(self.sessions.iter().filter_map(|(id, session)| session.row(id).ok()).collect())
    }
}

pub(super) fn read() -> Result<SysValue> {
    // Explicit queries do not create subscriptions; the service keeps its
    // catalog alive and reads fresh levels directly on native callbacks.
    let devices = unsafe { enumerator()?.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)? };
    let mut rows = BTreeMap::new();
    for d in 0..unsafe { devices.GetCount()? } {
        let device = unsafe { devices.Item(d)? };
        let manager: IAudioSessionManager2 = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let sessions = unsafe { manager.GetSessionEnumerator()? };
        for i in 0..unsafe { sessions.GetCount()? } {
            if rows.len() >= 256 { break; }
            let row = (|| -> Result<_> {
                let control: IAudioSessionControl2 = unsafe { sessions.GetSession(i)? }.cast()?;
                if unsafe { control.GetState()? } == AudioSessionStateExpired || unsafe { control.GetProcessId()? } == 0 {
                    return Ok(None);
                }
                let id = owned_text(unsafe { control.GetSessionInstanceIdentifier() })?;
                let row = Session::new(String::new(), control, None)?.row(&id)?;
                Ok(Some((id, row)))
            })();
            if let Ok(Some((id, row))) = row { rows.insert(id, row); }
        }
    }
    Ok(SysValue::List(rows.into_values().collect()))
}

fn find(id: &str) -> Result<Option<ISimpleAudioVolume>> {
    let devices = unsafe { enumerator()?.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)? };
    for d in 0..unsafe { devices.GetCount()? } {
        let found = (|| -> Result<Option<ISimpleAudioVolume>> {
            let manager: IAudioSessionManager2 = unsafe { devices.Item(d)?.Activate(CLSCTX_ALL, None)? };
            let sessions = unsafe { manager.GetSessionEnumerator()? };
            for i in 0..unsafe { sessions.GetCount()? } {
                let control: IAudioSessionControl2 = unsafe { sessions.GetSession(i)? }.cast()?;
                if unsafe { control.GetState()? } == AudioSessionStateExpired || unsafe { control.GetProcessId()? } == 0 { continue; }
                if owned_text(unsafe { control.GetSessionInstanceIdentifier() })? == id { return control.cast().map(Some); }
            }
            Ok(None)
        })();
        // One output can disappear while another still has the target session.
        if let Ok(Some(volume)) = found { return Ok(Some(volume)); }
    }
    Ok(None)
}

pub(super) fn command(name: &str, args: &[SysValue]) -> std::result::Result<(), String> {
    let (id, level, mute) = match (name, args) {
        ("audio.app_volume", [SysValue::Text(id), SysValue::Num(level)])
            if level.is_finite() && (0.0..=1.0).contains(level) => (id, Some(*level as f32), None),
        ("audio.app_mute", [SysValue::Text(id)]) => (id, None, None),
        ("audio.app_mute", [SysValue::Text(id), SysValue::Bool(mute)]) => (id, None, Some(*mute)),
        _ => return Err("Use audio.app_volume(id, 0..1) or audio.app_mute(id[, true|false]) with an audio apps instance id".into()),
    };
    if id.is_empty() || id.len() > 32768 || id.contains('\0') { return Err("invalid audio session instance id".into()); }
    let volume = find(id).map_err(|e| e.to_string())?.ok_or("The application's audio session has ended")?;
    unsafe {
        if let Some(level) = level {
            volume.SetMasterVolume(level, std::ptr::null())
        } else {
            let mute = match mute { Some(mute) => mute, None => !volume.GetMute().map_err(|e| e.to_string())?.as_bool() };
            volume.SetMute(mute, std::ptr::null())
        }
    }.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_app_controls_fail_before_enumerating_or_changing_audio() {
        for level in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
            assert!(command("audio.app_volume", &[SysValue::Text("unused".into()), SysValue::Num(level)]).is_err());
        }
        for args in [vec![SysValue::Text(String::new())], vec![SysValue::Num(1.0)], vec![SysValue::Text("bad\0id".into())]] {
            assert!(command("audio.app_mute", &args).is_err());
        }
    }

    struct SilentSession { client: IAudioClient, control: IAudioSessionControl2, volume: ISimpleAudioVolume }
    impl SilentSession {
        fn new(name: &str, level: f32) -> Result<Self> {
            unsafe {
                let device = enumerator()?.GetDefaultAudioEndpoint(eRender, eConsole)?;
                let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
                let format = client.GetMixFormat()?;
                let guid = windows::Win32::System::Com::CoCreateGuid()?;
                let initialized = client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 1_000_000, 0, format, Some(&guid));
                CoTaskMemFree(Some(format as _));
                initialized?;
                let control: IAudioSessionControl = client.GetService()?;
                let control: IAudioSessionControl2 = control.cast()?;
                control.SetDisplayName(&windows::core::HSTRING::from(name), std::ptr::null())?;
                let volume: ISimpleAudioVolume = client.GetService()?;
                volume.SetMasterVolume(level, std::ptr::null())?;
                let render: IAudioRenderClient = client.GetService()?;
                let count = client.GetBufferSize()?;
                let _ = render.GetBuffer(count)?;
                render.ReleaseBuffer(count, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)?;
                client.Start()?;
                Ok(Self { client, control, volume })
            }
        }
        fn id(&self) -> Result<String> { owned_text(unsafe { self.control.GetSessionInstanceIdentifier() }) }
    }
    impl Drop for SilentSession { fn drop(&mut self) { unsafe { let _ = self.client.Stop(); } } }

    #[test]
    #[ignore = "controls two unique silent WASAPI test sessions only; no windows or physical input"]
    fn native_app_mixer_isolates_session_volume_mute_and_callbacks() {
        let _apartment = super::super::super::windows_system::Apartment::new().unwrap();
        let master = super::super::endpoint(eRender).unwrap();
        let before = unsafe { (master.GetMasterVolumeLevelScalar().unwrap(), master.GetMute().unwrap()) };
        let first = SilentSession::new("Owned mixer test — Español", 0.21).unwrap();
        let second = SilentSession::new("Owned mixer test — 日本語", 0.72).unwrap();
        let first_id = first.id().unwrap();
        let second_id = second.id().unwrap();
        assert_ne!(first_id, second_id, "two sessions in one PID must remain independent");
        let (wake, receiver) = Wake::new();
        let mut catalog = Catalog::default();
        catalog.refresh(&enumerator().unwrap(), &wake).unwrap();
        assert!(catalog.sessions.contains_key(&first_id));
        assert!(catalog.sessions.contains_key(&second_id));
        while receiver.try_recv().is_ok() {}
        wake.take();
        command("audio.app_volume", &[SysValue::Text(first_id.clone()), SysValue::Num(0.43)]).unwrap();
        receiver.recv_timeout(std::time::Duration::from_secs(3)).unwrap();
        assert_ne!(wake.take() & LEVELS, 0, "real session volume callback was lost");
        unsafe {
            assert!((first.volume.GetMasterVolume().unwrap() - 0.43).abs() < 0.001);
            assert!((second.volume.GetMasterVolume().unwrap() - 0.72).abs() < 0.001);
        }
        command("audio.app_mute", &[SysValue::Text(first_id.clone()), SysValue::Bool(true)]).unwrap();
        command("audio.app_volume", &[SysValue::Text(first_id.clone()), SysValue::Num(0.31)]).unwrap();
        unsafe {
            assert!(first.volume.GetMute().unwrap().as_bool(), "moving the level must preserve mute");
            assert!(!second.volume.GetMute().unwrap().as_bool());
        }
        command("audio.app_mute", &[SysValue::Text(first_id.clone())]).unwrap();
        assert!(!unsafe { first.volume.GetMute().unwrap() }.as_bool());
        assert!(command("audio.app_volume", &[SysValue::Text("missing-instance".into()), SysValue::Num(0.13)]).is_err());
        while receiver.try_recv().is_ok() {}
        wake.take();
        let third = SilentSession::new("Owned late mixer session", 0.62).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            receiver.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())).unwrap();
            if wake.take() & APPS != 0 { break; }
        }
        assert!(catalog.pending.lock().unwrap().iter().any(|(_, control)| {
            control.cast::<IAudioSessionControl2>().is_ok_and(|control| {
                owned_text(unsafe { control.GetSessionInstanceIdentifier() }).ok() == third.id().ok()
            })
        }), "late-created session must arrive through the native session callback");
        catalog.refresh(&enumerator().unwrap(), &wake).unwrap();
        assert!(catalog.sessions.contains_key(&third.id().unwrap()), "late-created session was not discovered");
        let SysValue::List(queried) = read().unwrap() else { panic!("queried apps") };
        for expected in [&first_id, &second_id, &third.id().unwrap()] {
            assert!(queried.iter().any(|row| matches!(row, SysValue::Map(values) if values.iter().any(
                |(key, value)| key == "id" && matches!(value, SysValue::Text(id) if id == expected)
            ))), "explicit query lost an active session");
        }
        let SysValue::Map(row) = catalog.sessions[&first_id].row(&first_id).unwrap() else { panic!("session row") };
        assert!(row.iter().any(|(key, value)| key == "title" && matches!(value, SysValue::Text(s) if s.contains("Español"))));
        unsafe { assert_eq!((master.GetMasterVolumeLevelScalar().unwrap(), master.GetMute().unwrap()), before); }
        drop(catalog);
        drop(first);
        drop(second);
        drop(third);
        println!("PASS: three silent WASAPI sessions; exact instance control, independent levels/mute, Unicode metadata, native callbacks and unchanged master");
    }
}
