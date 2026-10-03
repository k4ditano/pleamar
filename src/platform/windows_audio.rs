//! Core Audio endpoints. Commands resolve the current default at execution time.
#[path = "windows_audio_watch.rs"]
mod watch;
pub use watch::service;
use super::SysValue;
use windows::core::{Result, Interface, GUID, HRESULT, PCWSTR, IUnknown, IUnknown_Vtbl};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{IMMDevice, IMMDeviceEnumerator, IMMEndpoint, MMDeviceEnumerator, EDataFlow, ERole, eRender, eCapture, eConsole, eMultimedia, eCommunications, DEVICE_STATE_ACTIVE};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, STGM_READ};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::Media::KernelStreaming::*;

fn enumerator() -> Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
}

fn endpoint(flow: EDataFlow) -> Result<IAudioEndpointVolume> {
    unsafe { enumerator()?.GetDefaultAudioEndpoint(flow, eConsole)?.Activate(CLSCTX_ALL, None) }
}

// Windows exposes no documented desktop setter for the system default endpoint.
// This isolated COM ABI is also used by EarTrumpet (Interop/MMDeviceAPI):
// https://github.com/File-New-Project/EarTrumpet/blob/master/EarTrumpet/Interop/MMDeviceAPI/IPolicyConfig.cs
// QueryInterface failure is reported; never replace this operation with Settings.
#[repr(transparent)]
#[derive(Clone)]
struct PolicyConfig(IUnknown);
unsafe impl Interface for PolicyConfig {
    type Vtable = PolicyConfigVtbl;
    const IID: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);
}
#[repr(C)]
struct PolicyConfigVtbl {
    base: IUnknown_Vtbl,
    unused: [usize; 10],
    set_default: unsafe extern "system" fn(*mut std::ffi::c_void, PCWSTR, ERole) -> HRESULT,
}

fn policy() -> Result<PolicyConfig> {
    unsafe { CoCreateInstance(&GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9), None::<&IUnknown>, CLSCTX_ALL) }
}

fn set_role(policy: &PolicyConfig, id: &str, role: ERole) -> Result<()> {
    let wide: Vec<u16> = id.encode_utf16().chain([0]).collect();
    unsafe { (policy.vtable().set_default)(policy.as_raw(), PCWSTR(wide.as_ptr()), role).ok() }
}

fn select_default(id: &str) -> std::result::Result<(), String> {
    if id.is_empty() || id.contains('\0') { return Err("invalid audio endpoint identifier".into()); }
    let e = enumerator().map_err(|e| e.to_string())?;
    let wide: Vec<u16> = id.encode_utf16().chain([0]).collect();
    let device = unsafe { e.GetDevice(PCWSTR(wide.as_ptr())) }.map_err(|e| e.to_string())?;
    if unsafe { device.GetState() }.map_err(|e| e.to_string())? != DEVICE_STATE_ACTIVE {
        return Err("the audio endpoint is no longer active".into());
    }
    let flow = unsafe { device.cast::<IMMEndpoint>().and_then(|v| v.GetDataFlow()) }.map_err(|e| e.to_string())?;
    let policy = policy().map_err(|e| format!("Windows audio policy is unavailable: {e}"))?;
    let roles = [eConsole, eMultimedia, eCommunications];
    let previous: Vec<_> = roles.iter().map(|&r| unsafe { e.GetDefaultAudioEndpoint(flow, r) }.and_then(|d| identifier(&d)).ok()).collect();
    for (i, role) in roles.iter().enumerate() {
        let result = set_role(&policy, id, *role).and_then(|_| {
            let actual = unsafe { e.GetDefaultAudioEndpoint(flow, *role) }.and_then(|d| identifier(&d))?;
            if actual == id { Ok(()) } else { Err(windows::core::Error::from_hresult(HRESULT(0x80004005u32 as i32))) }
        });
        if let Err(error) = result {
            let mut restored = true;
            for j in 0..=i {
                restored &= previous[j].as_ref().is_some_and(|old| set_role(&policy, old, roles[j]).is_ok());
            }
            return Err(format!("could not select audio endpoint: {error}; previous roles restored: {restored}"));
        }
    }
    Ok(())
}

fn identifier(device: &IMMDevice) -> Result<String> {
    unsafe {
        let raw = device.GetId()?;
        let value = raw.to_string();
        CoTaskMemFree(Some(raw.0 as _));
        Ok(value?)
    }
}

fn label(device: &IMMDevice) -> Result<String> {
    unsafe {
        let mut value = device.OpenPropertyStore(STGM_READ)?.GetValue(&PKEY_Device_FriendlyName)?;
        let raw = PropVariantToStringAlloc(&value);
        let _ = PropVariantClear(&mut value);
        let raw = raw?;
        let name = raw.to_string();
        CoTaskMemFree(Some(raw.0 as _));
        Ok(name?)
    }
}

fn devices(e: &IMMDeviceEnumerator, flow: EDataFlow) -> Result<SysValue> {
    let mut list = Vec::new();
    unsafe {
        let default = e.GetDefaultAudioEndpoint(flow, eConsole).and_then(|d| identifier(&d)).ok();
        let devices = e.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
        for k in 0..devices.GetCount()? {
            let device = devices.Item(k)?;
            let id = identifier(&device)?;
            list.push(SysValue::Map(vec![
                ("name".into(), SysValue::Text(label(&device).unwrap_or_else(|_| id.clone()))),
                ("default".into(), SysValue::Bool(default.as_ref() == Some(&id))),
                ("id".into(), SysValue::Text(id)),
            ]));
        }
    }
    Ok(SysValue::List(list))
}

struct BluetoothAudio {
    id: String,
    name: String,
    connected: bool,
    controls: Vec<IKsControl>,
}

fn bluetooth_property(id: KSPROPERTY_BTAUDIO, flags: u32) -> KSIDENTIFIER {
    KSIDENTIFIER { Anonymous: KSIDENTIFIER_0 { Anonymous: KSIDENTIFIER_0_0 { Set: KSPROPSETID_BtAudio, Id: id.0 as u32, Flags: flags } } }
}

fn bluetooth_catalog() -> Result<Vec<BluetoothAudio>> {
    use windows::Win32::{Devices::FunctionDiscovery::PKEY_Device_ContainerId, Media::Audio::{IDeviceTopology, eAll, DEVICE_STATEMASK_ALL}};
    use windows::Win32::System::Com::StructuredStorage::PropVariantToGUID;
    let e = enumerator()?;
    let mut groups: Vec<BluetoothAudio> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    unsafe {
        let endpoints = e.EnumAudioEndpoints(eAll, windows::Win32::Media::Audio::DEVICE_STATE(DEVICE_STATEMASK_ALL))?;
        for i in 0..endpoints.GetCount()? {
            let endpoint = endpoints.Item(i)?;
            let Ok(topology) = endpoint.Activate::<IDeviceTopology>(CLSCTX_ALL, None) else { continue; };
            let Ok(connector) = topology.GetConnector(0) else { continue; };
            let Ok(raw) = connector.GetDeviceIdConnectedTo() else { continue; };
            let adapter = e.GetDevice(PCWSTR(raw.0));
            let adapter_id = raw.to_string();
            CoTaskMemFree(Some(raw.0 as _));
            let (Ok(adapter), Ok(adapter_id)) = (adapter, adapter_id) else { continue; };
            let Ok(control) = adapter.Activate::<IKsControl>(CLSCTX_ALL, None) else { continue; };
            let property = bluetooth_property(KSPROPERTY_ONESHOT_RECONNECT, KSPROPERTY_TYPE_BASICSUPPORT);
            let (mut support, mut returned) = (0u32, 0u32);
            if control.KsProperty(&property, std::mem::size_of::<KSIDENTIFIER>() as u32,
                &mut support as *mut _ as _, 4, &mut returned).is_err() || support & KSPROPERTY_TYPE_GET == 0 { continue; }
            let Ok(store) = endpoint.OpenPropertyStore(STGM_READ) else { continue; };
            let Ok(mut variant) = store.GetValue(&PKEY_Device_ContainerId) else { continue; };
            let container = PropVariantToGUID(&variant);
            let _ = PropVariantClear(&mut variant);
            let Ok(container) = container else { continue; };
            let id = format!("{container:?}");
            let connected = endpoint.GetState()? == DEVICE_STATE_ACTIVE;
            let index = groups.iter().position(|g| g.id == id).unwrap_or_else(|| {
                let name = label(&adapter).ok().filter(|s| !s.is_empty())
                    .or_else(|| label(&endpoint).ok().filter(|s| !s.is_empty())).unwrap_or_else(|| id.clone());
                groups.push(BluetoothAudio { id: id.clone(), name, connected: false, controls: Vec::new() });
                groups.len() - 1
            });
            groups[index].connected |= connected;
            if seen.insert(adapter_id) { groups[index].controls.push(control); }
        }
    }
    Ok(groups)
}

pub fn bluetooth_devices() -> Result<Vec<SysValue>> {
    // Probe capabilities only; enumerating the page never initiates a connection.
    Ok(bluetooth_catalog()?.into_iter().map(|device| SysValue::Map(vec![
        ("id".into(), SysValue::Text(device.id)), ("name".into(), SysValue::Text(device.name)),
        ("current".into(), SysValue::Bool(device.connected)), ("controllable".into(), SysValue::Bool(true)),
    ])).collect())
}

pub fn bluetooth_connect(id: &str, connect: bool) -> std::result::Result<(), String> {
    let device = bluetooth_catalog().map_err(|e| e.to_string())?.into_iter().find(|device| device.id == id)
        .ok_or("This device has no Bluetooth audio driver with connection control")?;
    // All audio profiles in this physical container must be disconnected.
    // A successful request is only an attempt; report actual state via polling.
    let property = bluetooth_property(if connect { KSPROPERTY_ONESHOT_RECONNECT } else { KSPROPERTY_ONESHOT_DISCONNECT }, KSPROPERTY_TYPE_GET);
    let mut errors = Vec::new();
    for control in device.controls {
        let mut returned = 0;
        if let Err(e) = unsafe { control.KsProperty(&property, std::mem::size_of::<KSIDENTIFIER>() as u32, std::ptr::null_mut(), 0, &mut returned) } {
            errors.push(e.to_string());
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(format!("Bluetooth audio request failed: {}", errors.join("; "))) }
}

pub fn read() -> Result<SysValue> {
    let e = enumerator()?;
    // Explicit queries can arrive in bursts. Cache their expensive labels for
    // one second; the event watcher maintains its own invalidated catalog.
    thread_local! { static CATALOG: std::cell::RefCell<Option<(std::time::Instant, SysValue, SysValue)>> = const { std::cell::RefCell::new(None) }; }
    let (outputs, inputs) = CATALOG.with(|cache| -> Result<_> {
        let mut cache = cache.borrow_mut();
        if cache.as_ref().is_none_or(|(at, _, _)| at.elapsed() >= std::time::Duration::from_secs(1)) {
            *cache = Some((std::time::Instant::now(), devices(&e, eRender)?, devices(&e, eCapture)?));
        }
        let (_, outputs, inputs) = cache.as_ref().unwrap();
        Ok((outputs.clone(), inputs.clone()))
    })?;
    let mut result = vec![("outputs".into(), outputs), ("inputs".into(), inputs)];
    for (flow, level, mute) in [(eRender, "volume", "muted"), (eCapture, "input", "input_muted")] {
        if let Ok(device) = endpoint(flow) {
            unsafe {
                result.push((level.into(), SysValue::Num(device.GetMasterVolumeLevelScalar()? as f64)));
                result.push((mute.into(), SysValue::Bool(device.GetMute()?.as_bool())));
            }
        }
    }
    Ok(SysValue::Map(result))
}

pub fn command(name: &str, args: &[SysValue]) -> std::result::Result<(), String> {
    let flow = if name.starts_with("audio.input") { eCapture } else { eRender };
    match name {
        "audio.volume" | "audio.input" | "audio.input_volume" | "audio.step" => {
            let [SysValue::Num(value)] = args else { return Err("audio volume takes a number from 0 to 1".into()); };
            if !value.is_finite() { return Err("audio volume must be finite".into()); }
            unsafe { endpoint(flow).and_then(|e| {
                let level = if name == "audio.step" { e.GetMasterVolumeLevelScalar()? as f64 + value } else { *value };
                e.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0) as f32, std::ptr::null())
            }) }.map_err(|e| e.to_string())
        }
        "audio.mute" | "audio.input_mute" => {
            unsafe {
                let e = endpoint(flow).map_err(|e| e.to_string())?;
                let muted = match args {
                    [] => !e.GetMute().map_err(|e| e.to_string())?.as_bool(),
                    [SysValue::Bool(value)] => *value,
                    _ => return Err("audio mute takes no arguments (toggle), or a boolean".into()),
                };
                e.SetMute(muted, std::ptr::null()).map_err(|e| e.to_string())
            }
        }
        "audio.default" => {
            let [SysValue::Text(id)] = args else { return Err("audio.default takes an endpoint identifier".into()); };
            select_default(id)
        }
        _ => Err(format!("unsupported audio command: {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_volume_is_rejected_before_accessing_hardware() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(command("audio.volume", &[SysValue::Num(value)]).is_err());
        }
        assert!(command("audio.volume", &[]).is_err());
        assert!(command("audio.default", &[]).is_err());
        assert!(command("audio.default", &[SysValue::Text("bad\0id".into())]).is_err());
    }

    #[test]
    #[ignore = "requires two active endpoints per flow; changes and restores default playback and capture devices"]
    fn live_default_endpoint_roundtrip_restores_roles() {
        let _apartment = super::super::windows_system::Apartment::new().unwrap();
        let e = enumerator().unwrap();
        let roles = [eConsole, eMultimedia, eCommunications];
        for flow in [eRender, eCapture] {
        let previous: Vec<_> = roles.iter().map(|&r| identifier(&unsafe { e.GetDefaultAudioEndpoint(flow, r) }.unwrap()).unwrap()).collect();
        struct Restore(PolicyConfig, Vec<String>);
        impl Drop for Restore {
            fn drop(&mut self) {
                for (id, role) in self.1.iter().zip([eConsole, eMultimedia, eCommunications]) {
                    set_role(&self.0, id, role).expect("restore default audio role");
                }
            }
        }
        let restore = Restore(policy().unwrap(), previous.clone());
        let list = unsafe { e.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE) }.unwrap();
        let target = (0..unsafe { list.GetCount() }.unwrap()).map(|i| identifier(&unsafe { list.Item(i) }.unwrap()).unwrap())
            .find(|id| id != &previous[0]).expect("need a second active output for this test");
        command("audio.default", &[SysValue::Text(target.clone())]).unwrap();
        for role in roles { assert_eq!(identifier(&unsafe { e.GetDefaultAudioEndpoint(flow, role) }.unwrap()).unwrap(), target); }
        drop(restore);
        for (role, id) in roles.into_iter().zip(previous) { assert_eq!(identifier(&unsafe { e.GetDefaultAudioEndpoint(flow, role) }.unwrap()).unwrap(), id); }
        }
    }

    #[test]
    #[ignore = "requires a live audio endpoint; briefly changes and restores the system volume"]
    fn live_volume_roundtrip_restores_original() {
        let _apartment = super::super::windows_system::Apartment::new().unwrap();
        let device = endpoint(eRender).unwrap();
        struct Restore(IAudioEndpointVolume, f32);
        impl Drop for Restore {
            fn drop(&mut self) { unsafe { self.0.SetMasterVolumeLevelScalar(self.1, std::ptr::null()).expect("restore system volume"); } }
        }
        let original = unsafe { device.GetMasterVolumeLevelScalar().unwrap() };
        let restore = Restore(device, original);
        let value = if original > 0.05 { original - 0.01 } else { original + 0.01 };
        command("audio.volume", &[SysValue::Num(value as f64)]).unwrap();
        let actual = unsafe { restore.0.GetMasterVolumeLevelScalar().unwrap() };
        assert!((actual - value).abs() < 0.002, "the real endpoint did not change");
        drop(restore);
        assert!((unsafe { endpoint(eRender).unwrap().GetMasterVolumeLevelScalar().unwrap() } - original).abs() < 0.002);
    }
}
