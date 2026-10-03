//! Core Audio callbacks only wake the worker; COM reads and unregistration
//! stay off the callback, Lua and render threads.
use super::*;
use std::sync::{Arc, atomic::{AtomicU8, Ordering}, mpsc::{self, Receiver, SyncSender}};
use std::time::{Duration, Instant};
use windows::core::implement;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{IMMNotificationClient, IMMNotificationClient_Impl, DEVICE_STATE, AUDIO_VOLUME_NOTIFICATION_DATA};
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl};

const LEVELS: u8 = 1;
const DEVICES: u8 = 2;
const RECOVERY: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct Wake {
    pending: Arc<AtomicU8>,
    sender: SyncSender<()>,
}

impl Wake {
    fn new() -> (Self, Receiver<()>) {
        let (sender, receiver) = mpsc::sync_channel(1);
        (Self { pending: Arc::new(AtomicU8::new(0)), sender }, receiver)
    }
    fn signal(&self, kind: u8) {
        self.pending.fetch_or(kind, Ordering::Release);
        // A full queue already promises a wakeup; the bits retain every kind
        // of change without a queue growing for every slider sample.
        let _ = self.sender.try_send(());
    }
    fn take(&self) -> u8 { self.pending.swap(0, Ordering::AcqRel) }
    fn wait(&self, receiver: &Receiver<()>, recovery: Instant) -> Option<u8> {
        loop {
            if Instant::now() >= recovery { return Some(self.take() | DEVICES); }
            match receiver.recv_timeout(recovery.saturating_duration_since(Instant::now())) {
                Ok(()) => {
                    let pending = self.take();
                    if pending != 0 { return Some(pending); }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => return Some(self.take() | DEVICES),
                Err(mpsc::RecvTimeoutError::Disconnected) => return None,
            }
        }
    }
}

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeNotice(Wake);
impl IAudioEndpointVolumeCallback_Impl for VolumeNotice_Impl {
    fn OnNotify(&self, _: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
        self.0.signal(LEVELS);
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceNotice(Wake);
impl IMMNotificationClient_Impl for DeviceNotice_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> Result<()> { self.0.signal(DEVICES); Ok(()) }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> Result<()> { self.0.signal(DEVICES); Ok(()) }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> Result<()> { self.0.signal(DEVICES); Ok(()) }
    fn OnDefaultDeviceChanged(&self, _: EDataFlow, _: ERole, _: &PCWSTR) -> Result<()> { self.0.signal(DEVICES); Ok(()) }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> Result<()> { self.0.signal(DEVICES); Ok(()) }
}

struct Endpoint {
    id: String,
    volume: IAudioEndpointVolume,
    callback: IAudioEndpointVolumeCallback,
}
impl Endpoint {
    fn new(device: IMMDevice, id: String, wake: &Wake) -> Result<Self> {
        let volume: IAudioEndpointVolume = unsafe { device.Activate(CLSCTX_ALL, None)? };
        let callback: IAudioEndpointVolumeCallback = VolumeNotice(wake.clone()).into();
        unsafe { volume.RegisterControlChangeNotify(&callback)?; }
        Ok(Self { id, volume, callback })
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) { unsafe { let _ = self.volume.UnregisterControlChangeNotify(&self.callback); } }
}

struct Watcher {
    enumerator: IMMDeviceEnumerator,
    callback: IMMNotificationClient,
    wake: Wake,
    outputs: SysValue,
    inputs: SysValue,
    endpoints: [Option<Endpoint>; 2],
}
impl Watcher {
    fn new(wake: Wake) -> Result<Self> {
        let enumerator = enumerator()?;
        let callback: IMMNotificationClient = DeviceNotice(wake.clone()).into();
        // Subscribe before taking the snapshot so a concurrent device change
        // queues another refresh instead of being lost between read and subscribe.
        unsafe { enumerator.RegisterEndpointNotificationCallback(&callback)?; }
        Ok(Self { enumerator, callback, wake, outputs: SysValue::List(Vec::new()),
            inputs: SysValue::List(Vec::new()), endpoints: [None, None] })
    }
    fn refresh(&mut self) -> Result<()> {
        let outputs = devices(&self.enumerator, eRender)?;
        let inputs = devices(&self.enumerator, eCapture)?;
        for (index, flow) in [eRender, eCapture].into_iter().enumerate() {
            let device = match unsafe { self.enumerator.GetDefaultAudioEndpoint(flow, eConsole) } {
                Ok(device) => device,
                // No endpoint is normal on a machine without a microphone or
                // while the last device is unplugged; other COM errors need recovery.
                Err(error) if error.code() == HRESULT::from_win32(1168) => {
                    self.endpoints[index] = None;
                    continue;
                }
                Err(error) => return Err(error),
            };
            let id = identifier(&device)?;
            if self.endpoints[index].as_ref().is_none_or(|endpoint| endpoint.id != id) {
                self.endpoints[index] = Some(Endpoint::new(device, id, &self.wake)?);
            }
        }
        self.outputs = outputs;
        self.inputs = inputs;
        Ok(())
    }
    fn snapshot(&self) -> Result<SysValue> {
        let mut values = vec![("outputs".into(), self.outputs.clone()), ("inputs".into(), self.inputs.clone())];
        for (index, level, mute) in [(0, "volume", "muted"), (1, "input", "input_muted")] {
            if let Some(endpoint) = &self.endpoints[index] {
                unsafe {
                    values.push((level.into(), SysValue::Num(endpoint.volume.GetMasterVolumeLevelScalar()? as f64)));
                    values.push((mute.into(), SysValue::Bool(endpoint.volume.GetMute()?.as_bool())));
                }
            }
        }
        Ok(SysValue::Map(values))
    }
}
impl Drop for Watcher {
    fn drop(&mut self) { unsafe { let _ = self.enumerator.UnregisterEndpointNotificationCallback(&self.callback); } }
}

pub fn service(notify: Box<dyn Fn(SysValue) + Send>) -> bool {
    std::thread::Builder::new().name("audio".into()).spawn(move || {
        let _apartment = match super::super::windows_system::Apartment::new() {
            Ok(apartment) => apartment,
            Err(error) => { eprintln!("windows · audio unavailable: {error}"); return; }
        };
        let (wake, receiver) = Wake::new();
        let (mut watcher, mut last, mut warned) = (None, None, false);
        let mut changes = DEVICES;
        let mut recovery = Instant::now();
        loop {
            let result = (|| -> Result<SysValue> {
                if watcher.is_none() { watcher = Some(Watcher::new(wake.clone())?); changes |= DEVICES; }
                let watcher = watcher.as_mut().unwrap();
                if changes & DEVICES != 0 {
                    recovery = Instant::now() + RECOVERY;
                    watcher.refresh()?;
                }
                watcher.snapshot()
            })();
            let value = match result {
                Ok(value) => { warned = false; value }
                Err(error) => {
                    // A disconnected audio service invalidates COM interfaces.
                    // Recreate the subscriptions on retry, and keep the existing
                    // query path as a fallback when event registration fails.
                    watcher = None;
                    recovery = Instant::now() + RECOVERY;
                    if !warned { eprintln!("windows · audio notifications: {error}; retrying"); warned = true; }
                    read().unwrap_or_else(|error| SysValue::Map(vec![
                        ("available".into(), SysValue::Bool(false)),
                        ("error".into(), SysValue::Text(error.to_string())),
                        ("outputs".into(), SysValue::List(Vec::new())),
                        ("inputs".into(), SysValue::List(Vec::new())),
                    ]))
                }
            };
            if last.as_ref() != Some(&value) { notify(value.clone()); last = Some(value); }
            // A bounded recovery read handles missed callbacks/service restarts.
            // Volume events must not postpone that deadline indefinitely.
            let Some(pending) = wake.wait(&receiver, recovery) else { return };
            changes = pending;
        }
    }).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_bursts_keep_device_and_volume_changes_without_growing_the_queue() {
        let (wake, receiver) = Wake::new();
        let volume: IAudioEndpointVolumeCallback = VolumeNotice(wake.clone()).into();
        let devices: IMMNotificationClient = DeviceNotice(wake.clone()).into();
        let mut data = AUDIO_VOLUME_NOTIFICATION_DATA::default();
        for _ in 0..1000 {
            unsafe { volume.OnNotify(&mut data).unwrap(); }
        }
        unsafe { devices.OnDefaultDeviceChanged(eRender, eConsole, PCWSTR::null()).unwrap(); }
        receiver.try_recv().unwrap();
        assert_eq!(wake.take(), LEVELS | DEVICES);
        assert!(receiver.try_recv().is_err());
        unsafe { volume.OnNotify(&mut data).unwrap(); }
        receiver.try_recv().unwrap();
        assert_eq!(wake.take(), LEVELS);
    }

    #[test]
    fn every_device_callback_invalidates_the_catalog() {
        let (wake, receiver) = Wake::new();
        let devices: IMMNotificationClient = DeviceNotice(wake.clone()).into();
        for kind in 0..5 {
            unsafe {
                match kind {
                    0 => devices.OnDeviceAdded(PCWSTR::null()),
                    1 => devices.OnDeviceRemoved(PCWSTR::null()),
                    2 => devices.OnDeviceStateChanged(PCWSTR::null(), DEVICE_STATE_ACTIVE),
                    3 => devices.OnDefaultDeviceChanged(eCapture, eConsole, PCWSTR::null()),
                    _ => devices.OnPropertyValueChanged(PCWSTR::null(), PROPERTYKEY::default()),
                }.unwrap();
            }
            receiver.try_recv().unwrap();
            assert_eq!(wake.take(), DEVICES);
        }
    }

    #[test]
    fn volume_changes_do_not_postpone_device_recovery() {
        let (wake, receiver) = Wake::new();
        let recovery = Instant::now() + Duration::from_millis(10);
        loop {
            wake.signal(LEVELS);
            let changes = wake.wait(&receiver, recovery).unwrap();
            if changes & DEVICES != 0 { break; }
            assert_eq!(changes, LEVELS);
            assert!(Instant::now() < recovery + Duration::from_secs(1));
        }
        assert!(Instant::now() >= recovery);
    }

    #[test]
    #[ignore = "read-only Core Audio registration and snapshot on actual Windows hardware"]
    fn live_audio_subscription() {
        let _apartment = super::super::super::windows_system::Apartment::new().unwrap();
        let (wake, _) = Wake::new();
        for _ in 0..12 {
            let mut watcher = Watcher::new(wake.clone()).unwrap();
            watcher.refresh().unwrap();
            let SysValue::Map(values) = watcher.snapshot().unwrap() else { panic!("missing snapshot") };
            assert!(values.iter().any(|(name, value)| name == "outputs" && matches!(value, SysValue::List(_))));
            assert!(values.iter().any(|(name, value)| name == "inputs" && matches!(value, SysValue::List(_))));
        }
        println!("PASS: twelve actual Core Audio subscription/snapshot/drop cycles; no settings changed");
    }
}
