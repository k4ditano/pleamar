//! Radio, classic/LE discovery and pairing, and audio connection controls.
use super::SysValue;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::{core::Result, Devices::Radios::{Radio, RadioKind, RadioState, RadioAccessStatus}};
use windows::Win32::{Devices::Bluetooth::*, Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, WIN32_ERROR}};

static DISCOVERED: AtomicBool = AtomicBool::new(false);
static DISCOVERING: AtomicBool = AtomicBool::new(false);
#[path = "windows_bluetooth_le.rs"]
mod le;

fn radios() -> Result<Vec<Radio>> {
    let list = Radio::GetRadiosAsync()?.join()?;
    let mut radios = Vec::new();
    for i in 0..list.Size()? {
        let radio = list.GetAt(i)?;
        if radio.Kind()? == RadioKind::Bluetooth { radios.push(radio); }
    }
    Ok(radios)
}
struct Search(HBLUETOOTH_DEVICE_FIND);
impl Drop for Search { fn drop(&mut self) { unsafe { let _ = BluetoothFindDeviceClose(self.0); } } }
fn search_parameters(inquiry: bool) -> BLUETOOTH_DEVICE_SEARCH_PARAMS {
    BLUETOOTH_DEVICE_SEARCH_PARAMS {
        dwSize: size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32,
        fReturnAuthenticated: true.into(), fReturnRemembered: true.into(), fReturnConnected: true.into(),
        fReturnUnknown: (inquiry || DISCOVERED.load(Ordering::Relaxed)).into(),
        fIssueInquiry: inquiry.into(), cTimeoutMultiplier: if inquiry { 4 } else { 0 },
        ..Default::default()
    }
}
fn devices(inquiry: bool) -> Result<Vec<SysValue>> {
    let parameters = search_parameters(inquiry);
    let mut device = BLUETOOTH_DEVICE_INFO { dwSize: size_of::<BLUETOOTH_DEVICE_INFO>() as u32, ..Default::default() };
    let search = match unsafe { BluetoothFindFirstDevice(&parameters, &mut device) } {
        Ok(search) => Search(search),
        Err(e) if e.code() == ERROR_NO_MORE_ITEMS.to_hresult() => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut devices = Vec::new();
    loop {
        let end = device.szName.iter().position(|c| *c == 0).unwrap_or(device.szName.len());
        let id = format!("{:012X}", unsafe { device.Address.Anonymous.ullLong });
        let name = String::from_utf16_lossy(&device.szName[..end]);
        devices.push(SysValue::Map(vec![
            ("name".into(), SysValue::Text(if name.trim().is_empty() { format!("Bluetooth ({id})") } else { name })),
            ("id".into(), SysValue::Text(id)),
            ("current".into(), SysValue::Bool(device.fConnected.as_bool())),
            ("paired".into(), SysValue::Bool(device.fAuthenticated.as_bool())),
            ("pairable".into(), SysValue::Bool(!device.fAuthenticated.as_bool())),
            ("controllable".into(), SysValue::Bool(false)),
        ]));
        match unsafe { BluetoothFindNextDevice(search.0, &mut device) } {
            Ok(()) => {},
            Err(e) if e.code() == ERROR_NO_MORE_ITEMS.to_hresult() => break,
            Err(e) => return Err(e),
        }
    }
    Ok(devices)
}
fn snapshot(inquiry: bool) -> Result<SysValue> {
    let radios = radios()?;
    let mut enabled = false;
    for radio in &radios { enabled |= radio.State()? == RadioState::On; }
    let mut warnings = Vec::new();
    let mut audio = super::windows_audio::bluetooth_devices().unwrap_or_else(|error| {
        warnings.push(format!("Bluetooth audio: {error}"));
        Vec::new()
    });
    let classic = if radios.is_empty() { Vec::new() } else {
        devices(inquiry).unwrap_or_else(|error| {
            warnings.push(format!("Bluetooth classic: {error}"));
            Vec::new()
        })
    };
    // Audio containers cover both playback and microphone profiles in one row.
    // Classic addresses and audio container IDs are different identities.
    // Names (including empty or duplicate ones) cannot prove they are the same
    // device; keep pairing state instead of silently dropping it by substring.
    audio.extend(classic);
    let (low_energy, le_ready, le_error) = le::read(enabled);
    audio.extend(low_energy);
    if !le_error.is_empty() { warnings.push(format!("Bluetooth LE: {le_error}")); }
    Ok(SysValue::Map(vec![
        ("present".into(), SysValue::Bool(!radios.is_empty())),
        ("enabled".into(), SysValue::Bool(enabled)),
        ("devices".into(), SysValue::List(audio)),
        ("error".into(), SysValue::Text(String::new())),
        ("le_ready".into(), SysValue::Bool(le_ready)),
        ("warning".into(), SysValue::Text(warnings.join("; "))),
    ]))
}
pub fn read() -> Result<SysValue> {
    Ok(snapshot(false).unwrap_or_else(|error| SysValue::Map(vec![
        ("available".into(), SysValue::Bool(false)), ("error".into(), SysValue::Text(error.to_string())),
    ])))
}
pub fn discover() -> std::result::Result<SysValue, String> {
    if DISCOVERING.swap(true, Ordering::AcqRel) { return Err("Bluetooth discovery is already running".into()); }
    struct Scanning;
    impl Drop for Scanning { fn drop(&mut self) { DISCOVERING.store(false, Ordering::Release); } }
    let _scanning = Scanning;
    if !radios().map_err(|e| e.to_string())?.iter().any(|radio| radio.State().ok() == Some(RadioState::On)) {
        return Err("Turn on a Bluetooth radio before discovery".into());
    }
    // Both inquiries happen only on an explicit user action. The LE watcher
    // receives events while classic inquiry runs on this worker.
    let le_scan = le::Scan::start();
    let classic = devices(true).map_err(|e| e.to_string());
    if classic.is_ok() { DISCOVERED.store(true, Ordering::Relaxed); }
    let low_energy = le_scan.and_then(|scan| scan.finish());
    let mut warnings = Vec::new();
    if let Err(error) = classic { warnings.push(format!("Bluetooth classic: {error}")); }
    if let Err(error) = low_energy { warnings.push(format!("Bluetooth LE: {error}")); }
    let mut value = snapshot(false).map_err(|e| e.to_string())?;
    if !warnings.is_empty() {
        if let SysValue::Map(fields) = &mut value {
            if let Some((_, SysValue::Text(warning))) = fields.iter_mut().find(|(key, _)| key == "warning") {
                if !warning.is_empty() { warnings.push(warning.clone()); }
                *warning = warnings.join("; ");
            }
        }
    }
    Ok(value)
}
fn address(id: &str) -> std::result::Result<u64, String> {
    if id.len() != 12 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("pairing requires a 12-digit classic Bluetooth address from discovery".into());
    }
    let value = u64::from_str_radix(id, 16).map_err(|e| e.to_string())?;
    if value == 0 || value == 0xFFFF_FFFF_FFFF { return Err("invalid Bluetooth address".into()); }
    Ok(value)
}
fn pair(id: &str) -> std::result::Result<(), String> {
    if let Some(id) = id.strip_prefix("ble:") { return le::pair(id); }
    let address = address(id)?;
    let mut device = BLUETOOTH_DEVICE_INFO { dwSize: size_of::<BLUETOOTH_DEVICE_INFO>() as u32, ..Default::default() };
    device.Address.Anonymous.ullLong = address;
    // Require a device known to the stack, not a name or fabricated row. The
    // Windows pairing wizard owns PIN/confirmation UI; no PIN is auto-accepted.
    let status = unsafe { BluetoothGetDeviceInfo(None, &mut device) };
    if status != ERROR_SUCCESS.0 { return Err(windows::core::Error::from(WIN32_ERROR(status)).to_string()); }
    if device.fAuthenticated.as_bool() { return Ok(()); }
    let status = unsafe { BluetoothAuthenticateDeviceEx(None, None, &mut device, None, MITMProtectionNotRequiredBonding) };
    if status != ERROR_SUCCESS.0 && status != ERROR_NO_MORE_ITEMS.0 {
        return Err(windows::core::Error::from(WIN32_ERROR(status)).to_string());
    }
    // Authentication success is still confirmed by the stack, not the button.
    let status = unsafe { BluetoothGetDeviceInfo(None, &mut device) };
    if status != ERROR_SUCCESS.0 { return Err(windows::core::Error::from(WIN32_ERROR(status)).to_string()); }
    if !device.fAuthenticated.as_bool() { return Err("Windows has not confirmed Bluetooth pairing".into()); }
    Ok(())
}
fn forget(id: &str) -> std::result::Result<(), String> {
    if let Some(id) = id.strip_prefix("ble:") { return le::forget(id); }
    let address = address(id)?;
    let mut device = BLUETOOTH_DEVICE_INFO { dwSize: size_of::<BLUETOOTH_DEVICE_INFO>() as u32, ..Default::default() };
    device.Address.Anonymous.ullLong = address;
    let status = unsafe { BluetoothGetDeviceInfo(None, &mut device) };
    if status != ERROR_SUCCESS.0 { return Err(windows::core::Error::from(WIN32_ERROR(status)).to_string()); }
    // An audio container GUID identifies profiles, not a physical pairing.
    // Only a stack-validated classic address or LE association may be removed.
    let status = unsafe { BluetoothRemoveDevice(&device.Address) };
    if status != ERROR_SUCCESS.0 { return Err(windows::core::Error::from(WIN32_ERROR(status)).to_string()); }
    let status = unsafe { BluetoothGetDeviceInfo(None, &mut device) };
    if status == windows::Win32::Foundation::ERROR_NOT_FOUND.0 { return Ok(()); }
    if status != ERROR_SUCCESS.0 { return Err(windows::core::Error::from(WIN32_ERROR(status)).to_string()); }
    if device.fAuthenticated.as_bool() || device.fRemembered.as_bool() {
        return Err("Windows has not confirmed removal of this Bluetooth pairing".into());
    }
    Ok(())
}
pub fn command(name: &str, args: &[SysValue]) -> std::result::Result<(), String> {
    if let ("bluetooth.pair", [SysValue::Text(id)]) = (name, args) { return pair(id); }
    if let ("bluetooth.forget", [SysValue::Text(id)]) = (name, args) { return forget(id); }
    if let ("bluetooth.connect", [SysValue::Text(id), SysValue::Bool(connect)]) = (name, args) {
        return super::windows_audio::bluetooth_connect(id, *connect);
    }
    let ("bluetooth.radio", [SysValue::Bool(enabled)]) = (name, args) else { return Err("bluetooth.radio takes a boolean".into()); };
    let radios = radios().map_err(|e| e.to_string())?;
    if radios.is_empty() { return Err("No Bluetooth radio is present".into()); }
    let access = Radio::RequestAccessAsync().and_then(|op| op.join()).map_err(|e| e.to_string())?;
    if access != RadioAccessStatus::Allowed { return Err(format!("Windows denied Bluetooth radio access: {access:?}")); }
    for radio in radios {
        let status = radio.SetStateAsync(if *enabled { RadioState::On } else { RadioState::Off })
            .and_then(|op| op.join()).map_err(|e| e.to_string())?;
        if status != RadioAccessStatus::Allowed { return Err(format!("Windows rejected Bluetooth radio change: {status:?}")); }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pairing_rejects_names_guids_and_invalid_addresses_before_touching_a_radio() {
        assert_eq!(address("001122AaBbCc").unwrap(), 0x0011_22AA_BBCC);
        for id in ["Headphones", "{00112233-4455-6677-8899-aabbccddeeff}", "00:11:22:33:44:55", "000000000000", "FFFFFFFFFFFF", "00112233445G"] {
            assert!(command("bluetooth.pair", &[SysValue::Text(id.into())]).is_err());
            assert!(command("bluetooth.forget", &[SysValue::Text(id.into())]).is_err());
        }
    }
    #[test]
    fn polling_never_starts_discovery() {
        assert!(!search_parameters(false).fIssueInquiry.as_bool());
        let inquiry = search_parameters(true);
        assert!(inquiry.fIssueInquiry.as_bool() && inquiry.fReturnUnknown.as_bool());
        assert_eq!(inquiry.cTimeoutMultiplier, 4);
    }
}
