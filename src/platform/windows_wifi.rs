//! Native WLAN. Discovery is opt-in because it can require location consent.
use super::SysValue;
use std::{ffi::c_void, mem::size_of, sync::atomic::{AtomicBool, Ordering}, time::{Duration, Instant}};
use windows::core::{GUID, HRESULT, PCWSTR};
use windows::Win32::{Foundation::{HANDLE, ERROR_INVALID_STATE}, NetworkManagement::WiFi::*};

static DISCOVERY: AtomicBool = AtomicBool::new(false);
#[path = "windows_wifi_share.rs"]
mod sharing;
pub(super) fn has_thread_state() -> bool { sharing::has_thread_state() }
struct Client(HANDLE);
impl Drop for Client { fn drop(&mut self) { unsafe { WlanCloseHandle(self.0, None); } } }
struct Allocation(*mut c_void);
impl Drop for Allocation { fn drop(&mut self) { if !self.0.is_null() { unsafe { WlanFreeMemory(self.0); } } } }

fn checked(code: u32) -> Result<(), String> {
    match code {
        0 => Ok(()),
        5 => Err("Windows denied Wi-Fi access; check location consent and WLAN policy".into()),
        1062 => Err("The Windows WLAN AutoConfig service is not running".into()),
        _ => Err(windows::core::Error::from_hresult(HRESULT::from_win32(code)).to_string()),
    }
}
fn wide_text(v: &[u16]) -> String { String::from_utf16_lossy(&v[..v.iter().position(|c| *c == 0).unwrap_or(v.len())]) }
fn valid_profile(profile: &str) -> bool {
    !profile.is_empty() && profile.encode_utf16().count() < 256 && !profile.contains('\0')
}
fn hex_ssid(ssid: &DOT11_SSID) -> String {
    ssid.ucSSID[..(ssid.uSSIDLength as usize).min(32)].iter().map(|v| format!("{v:02X}")).collect()
}

fn personal_profile(ssid: &str, password: &str, secure: bool) -> Result<(String, String), String> {
    if ssid.is_empty() || ssid.len() > 64 || ssid.len() % 2 != 0 || !ssid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid Wi-Fi network identifier".into());
    }
    let profile = format!("Pleamar-{}", ssid.to_ascii_uppercase());
    let (authentication, encryption, key) = if secure {
        let kind = if password.len() == 64 && password.bytes().all(|b| b.is_ascii_hexdigit()) { "networkKey" }
            else if (8..=63).contains(&password.len()) && password.bytes().all(|b| (32..=126).contains(&b)) { "passPhrase" }
            else { return Err("WPA2 requires 8–63 ASCII characters or a 64-digit hexadecimal key".into()); };
        let escaped = password.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;");
        ("WPA2PSK", "AES", format!("<sharedKey><keyType>{kind}</keyType><protected>false</protected><keyMaterial>{escaped}</keyMaterial></sharedKey>"))
    } else {
        if !password.is_empty() { return Err("An open network does not use a password".into()); }
        ("open", "none", String::new())
    };
    // The key is passed directly to WLAN, which encrypts saved profile material.
    // Neither the XML nor the password is written to a log or temporary file.
    Ok((profile.clone(), format!("<WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\"><name>{profile}</name><SSIDConfig><SSID><hex>{ssid}</hex></SSID></SSIDConfig><connectionType>ESS</connectionType><connectionMode>manual</connectionMode><MSM><security><authEncryption><authentication>{authentication}</authentication><encryption>{encryption}</encryption><useOneX>false</useOneX></authEncryption>{key}</security></MSM></WLANProfile>")))
}
impl Client {
    fn open() -> Result<Self, String> {
        let mut handle = HANDLE::default();
        let mut version = 0;
        checked(unsafe { WlanOpenHandle(2, None, &mut version, &mut handle) })?;
        Ok(Self(handle))
    }
    fn interfaces(&self) -> Result<Vec<WLAN_INTERFACE_INFO>, String> {
        let mut list = std::ptr::null_mut();
        checked(unsafe { WlanEnumInterfaces(self.0, None, &mut list) })?;
        let _allocation = Allocation(list.cast());
        if list.is_null() { return Err("WLAN returned no interface list".into()); }
        unsafe {
            let count = (*list).dwNumberOfItems as usize;
            if count > 1024 { return Err("WLAN interface count is invalid".into()); }
            Ok(std::slice::from_raw_parts((*list).InterfaceInfo.as_ptr(), count).to_vec())
        }
    }
    fn radio(&self, id: &GUID) -> Result<WLAN_RADIO_STATE, String> {
        let mut data = std::ptr::null_mut();
        let mut size = 0;
        checked(unsafe { WlanQueryInterface(self.0, id, wlan_intf_opcode_radio_state, None, &mut size, &mut data, None) })?;
        let _allocation = Allocation(data);
        if data.is_null() || (size as usize) < size_of::<WLAN_RADIO_STATE>() { return Err("WLAN radio state is incomplete".into()); }
        let value = unsafe { *data.cast::<WLAN_RADIO_STATE>() };
        if value.dwNumberOfPhys > 64 { return Err("WLAN radio count is invalid".into()); }
        Ok(value)
    }
    fn networks(&self, id: &GUID) -> Result<Vec<WLAN_AVAILABLE_NETWORK>, String> {
        let mut list = std::ptr::null_mut();
        checked(unsafe { WlanGetAvailableNetworkList(self.0, id, 0, None, &mut list) })?;
        let _allocation = Allocation(list.cast());
        if list.is_null() { return Err("WLAN returned no network list".into()); }
        unsafe {
            let count = (*list).dwNumberOfItems as usize;
            if count > 65536 { return Err("WLAN network count is invalid".into()); }
            Ok(std::slice::from_raw_parts((*list).Network.as_ptr(), count).to_vec())
        }
    }
    fn connection(&self, id: &GUID) -> Result<Option<WLAN_CONNECTION_ATTRIBUTES>, String> {
        let mut data = std::ptr::null_mut();
        let mut size = 0;
        let status = unsafe { WlanQueryInterface(self.0, id, wlan_intf_opcode_current_connection, None, &mut size, &mut data, None) };
        let _allocation = Allocation(data);
        if status == ERROR_INVALID_STATE.0 { return Ok(None); }
        checked(status)?;
        if data.is_null() || (size as usize) < size_of::<WLAN_CONNECTION_ATTRIBUTES>() {
            return Err("WLAN connection state is incomplete".into());
        }
        Ok(Some(unsafe { *data.cast::<WLAN_CONNECTION_ATTRIBUTES>() }))
    }
}

fn authenticated_temporary(connection: &WLAN_CONNECTION_ATTRIBUTES, ssid: &str, profile: &str) -> bool {
    connection.isState == wlan_interface_state_connected
        && connection.wlanConnectionMode == wlan_connection_mode_temporary_profile
        && hex_ssid(&connection.wlanAssociationAttributes.dot11Ssid).eq_ignore_ascii_case(ssid)
        && wide_text(&connection.strProfileName) == profile
}

fn join(client: &Client, id: &GUID, ssid: &str, profile: &str, xml: &str) -> Result<(), String> {
    let wide: Vec<u16> = xml.encode_utf16().chain([0]).collect();
    let parameters = WLAN_CONNECTION_PARAMETERS {
        wlanConnectionMode: wlan_connection_mode_temporary_profile,
        strProfile: PCWSTR(wide.as_ptr()), dot11BssType: dot11_BSS_type_infrastructure,
        ..Default::default()
    };
    checked(unsafe { WlanConnect(client.0, id, &parameters, None) })?;
    // Do not persist an untested password. Otherwise the failed profile takes
    // over the next selection, preventing the user from entering a new key.
    let started = Instant::now();
    loop {
        if client.connection(id)?.as_ref().is_some_and(|connection| authenticated_temporary(connection, ssid, profile)) { break; }
        if started.elapsed() >= Duration::from_secs(30) {
            return Err("Windows has not confirmed the Wi-Fi connection; the password was not saved. Check the key and try again.".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let name: Vec<u16> = profile.encode_utf16().chain([0]).collect();
    // A profile created elsewhere is never overwritten by this operation.
    checked(unsafe { WlanSaveTemporaryProfile(client.0, id, PCWSTR(name.as_ptr()), PCWSTR::null(), WLAN_PROFILE_USER, false, None) })
        .map_err(|error| format!("Wi-Fi connected, but the profile could not be saved: {error}"))
}

fn snapshot() -> Result<SysValue, String> {
    let client = Client::open()?;
    let interfaces = client.interfaces()?;
    let mut enabled = false;
    let mut networks = Vec::new();
    let mut adapters = Vec::new();
    let mut error = String::new();
    for interface in &interfaces {
        adapters.push(SysValue::Map(vec![
            ("id".into(), SysValue::Text(format!("{:?}", interface.InterfaceGuid))),
            ("disconnected".into(), SysValue::Bool(interface.isState == wlan_interface_state_disconnected)),
        ]));
        match client.radio(&interface.InterfaceGuid) {
            Ok(state) => enabled |= state.PhyRadioState[..state.dwNumberOfPhys as usize].iter().any(|r|
                r.dot11SoftwareRadioState == dot11_radio_state_on && r.dot11HardwareRadioState == dot11_radio_state_on),
            Err(e) => error = e,
        }
        if !DISCOVERY.load(Ordering::Relaxed) { continue; }
        match client.networks(&interface.InterfaceGuid) {
            Ok(list) => for network in list {
                let length = (network.dot11Ssid.uSSIDLength as usize).min(32);
                if length == 0 { continue; }
                let profile = wide_text(&network.strProfileName);
                networks.push(SysValue::Map(vec![
                    ("name".into(), SysValue::Text(String::from_utf8_lossy(&network.dot11Ssid.ucSSID[..length]).into_owned())),
                    ("profile".into(), SysValue::Text(profile)),
                    ("ssid".into(), SysValue::Text(hex_ssid(&network.dot11Ssid))),
                    ("joinable".into(), SysValue::Bool(!network.bSecurityEnabled.as_bool() ||
                        (network.dot11DefaultAuthAlgorithm == DOT11_AUTH_ALGO_RSNA_PSK && network.dot11DefaultCipherAlgorithm == DOT11_CIPHER_ALGO_CCMP))),
                    ("interface".into(), SysValue::Text(format!("{:?}", interface.InterfaceGuid))),
                    ("signal".into(), SysValue::Num(network.wlanSignalQuality.min(100) as f64 / 100.0)),
                    ("current".into(), SysValue::Bool(network.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED != 0)),
                    ("connectable".into(), SysValue::Bool(network.bNetworkConnectable.as_bool())),
                    ("secure".into(), SysValue::Bool(network.bSecurityEnabled.as_bool())),
                ]));
            },
            Err(e) => error = e,
        }
    }
    Ok(SysValue::Map(vec![
        ("available".into(), SysValue::Bool(true)),
        ("present".into(), SysValue::Bool(!interfaces.is_empty())),
        ("enabled".into(), SysValue::Bool(enabled)),
        ("networks".into(), SysValue::List(networks)),
        ("adapters".into(), SysValue::List(adapters)),
        ("error".into(), SysValue::Text(error)),
    ]))
}

pub fn read() -> windows::core::Result<SysValue> {
    // Failure is data too: subscribers must see permission/driver changes.
    Ok(snapshot().unwrap_or_else(|error| SysValue::Map(vec![
        ("available".into(), SysValue::Bool(false)),
        ("error".into(), SysValue::Text(error)),
    ])))
}

pub fn command(name: &str, args: &[SysValue]) -> Result<(), String> {
    if name == "network.unshare" && args.is_empty() { sharing::clear(); return Ok(()); }
    match (name, args) {
        ("network.scan", []) | ("network.radio", [SysValue::Bool(_)]) => {},
        ("network.connect" | "network.forget", [SysValue::Text(interface), SysValue::Text(profile)]) if !interface.is_empty() && valid_profile(profile) => {},
        ("network.join", [SysValue::Text(_), SysValue::Text(ssid), SysValue::Text(password)]) => {
            // Reject malformed identifiers before opening WLAN. Security mode
            // itself is determined from the adapter's current scan below.
            personal_profile(ssid, password, !password.is_empty())?;
        },
        ("network.disconnect", [SysValue::Text(_)]) => {},
        _ => return Err("invalid native Wi-Fi command or arguments".into()),
    }
    let client = Client::open()?;
    let interfaces = client.interfaces()?;
    if interfaces.is_empty() { return Err("No Wi-Fi adapter is present".into()); }
    if name == "network.scan" {
        // Do not scan at startup: recent Windows versions can ask for consent.
        for interface in &interfaces { checked(unsafe { WlanScan(client.0, &interface.InterfaceGuid, None, None, None) })?; }
        DISCOVERY.store(true, Ordering::Relaxed);
        return Ok(());
    }
    if let ("network.radio", [SysValue::Bool(enabled)]) = (name, args) {
        for interface in &interfaces {
            let state = client.radio(&interface.InterfaceGuid)?;
            for previous in &state.PhyRadioState[..state.dwNumberOfPhys as usize] {
                if *enabled && previous.dot11HardwareRadioState == dot11_radio_state_off {
                    return Err("The physical Wi-Fi switch is off".into());
                }
                let value = WLAN_PHY_RADIO_STATE { dot11SoftwareRadioState: if *enabled { dot11_radio_state_on } else { dot11_radio_state_off }, ..*previous };
                checked(unsafe { WlanSetInterface(client.0, &interface.InterfaceGuid, wlan_intf_opcode_radio_state,
                    size_of::<WLAN_PHY_RADIO_STATE>() as u32, &value as *const _ as _, None) })?;
            }
        }
        return Ok(());
    }
    let SysValue::Text(id) = &args[0] else { unreachable!() };
    let interface = interfaces.iter().find(|i| format!("{:?}", i.InterfaceGuid) == *id).ok_or("The Wi-Fi adapter was removed")?;
    if name == "network.forget" {
        let SysValue::Text(profile) = &args[1] else { unreachable!() };
        let wide: Vec<u16> = profile.encode_utf16().chain([0]).collect();
        checked(unsafe { WlanDeleteProfile(client.0, &interface.InterfaceGuid, PCWSTR(wide.as_ptr()), None) })?;
        // Deletion is synchronous; re-read the exact profile before reporting
        // success. Never disconnect a different interface or delete by SSID.
        let mut xml = windows::core::PWSTR::null();
        let status = unsafe { WlanGetProfile(client.0, &interface.InterfaceGuid, PCWSTR(wide.as_ptr()), None, &mut xml, None, None) };
        let _allocation = Allocation(xml.0.cast());
        return if status == windows::Win32::Foundation::ERROR_NOT_FOUND.0 { Ok(()) }
            else if status == 0 { Err("Windows still reports the saved Wi-Fi profile".into()) }
            else { checked(status) };
    }
    if name == "network.disconnect" { return checked(unsafe { WlanDisconnect(client.0, &interface.InterfaceGuid, None) }); }
    if name == "network.join" {
        let (SysValue::Text(ssid), SysValue::Text(password)) = (&args[1], &args[2]) else { unreachable!() };
        let network = client.networks(&interface.InterfaceGuid)?.into_iter().find(|n| hex_ssid(&n.dot11Ssid).eq_ignore_ascii_case(ssid))
            .ok_or("The network is no longer visible; scan again")?;
        if !network.bNetworkConnectable.as_bool() { return Err("Windows says this network is not connectable".into()); }
        let secure = network.bSecurityEnabled.as_bool();
        if secure && !(network.dot11DefaultAuthAlgorithm == DOT11_AUTH_ALGO_RSNA_PSK && network.dot11DefaultCipherAlgorithm == DOT11_CIPHER_ALGO_CCMP) {
            return Err("New profiles currently support open networks and WPA2-Personal/AES; use an existing profile for enterprise or WPA3".into());
        }
        let (profile, xml) = personal_profile(ssid, password, secure)?;
        return join(&client, &interface.InterfaceGuid, ssid, &profile, &xml);
    }
    let SysValue::Text(profile) = &args[1] else { unreachable!() };
    // Select by the OS profile, never interpolate the SSID into a shell command.
    let wide: Vec<u16> = profile.encode_utf16().chain([0]).collect();
    let parameters = WLAN_CONNECTION_PARAMETERS {
        wlanConnectionMode: wlan_connection_mode_profile,
        strProfile: PCWSTR(wide.as_ptr()), dot11BssType: dot11_BSS_type_any,
        ..Default::default()
    };
    checked(unsafe { WlanConnect(client.0, &interface.InterfaceGuid, &parameters, None) })
    // Acceptance is not connection success. The next snapshot owns that state.
}

pub fn share(args: &[SysValue]) -> Result<SysValue, String> { sharing::read(args) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_authenticated_temporary_profile_is_persistable() {
        let mut state = WLAN_CONNECTION_ATTRIBUTES {
            isState: wlan_interface_state_connected,
            wlanConnectionMode: wlan_connection_mode_temporary_profile,
            ..Default::default()
        };
        state.wlanAssociationAttributes.dot11Ssid = DOT11_SSID { uSSIDLength: 2, ucSSID: [0xAB; 32] };
        let name: Vec<u16> = "Pleamar-ABAB".encode_utf16().collect();
        state.strProfileName[..name.len()].copy_from_slice(&name);
        assert!(authenticated_temporary(&state, "abab", "Pleamar-ABAB"));
        assert!(!authenticated_temporary(&state, "ABAC", "Pleamar-ABAB"));
        assert!(!authenticated_temporary(&state, "ABAB", "another profile"));
        state.isState = wlan_interface_state_authenticating;
        assert!(!authenticated_temporary(&state, "ABAB", "Pleamar-ABAB"));
        state.isState = wlan_interface_state_connected;
        state.wlanConnectionMode = wlan_connection_mode_profile;
        assert!(!authenticated_temporary(&state, "ABAB", "Pleamar-ABAB"));
    }
    #[test]
    fn invalid_wifi_commands_do_not_touch_a_radio() {
        assert!(command("network.connect", &[SysValue::Text("x".into()), SysValue::Text("bad\0profile".into())]).is_err());
        for profile in [String::new(), "bad\0profile".into(), "x".repeat(256), "海".repeat(256)] {
            assert!(command("network.forget", &[SysValue::Text("x".into()), SysValue::Text(profile)]).is_err());
        }
        assert!(command("network.forget", &[SysValue::Text("network name".into())]).is_err());
        assert!(command("network.radio", &[]).is_err());
        assert!(command("network.scan", &[SysValue::Bool(true)]).is_err());
    }
    #[test]
    fn wlan_profiles_escape_keys_and_encode_ssids_without_xml_injection() {
        let (name, xml) = personal_profile("45535041C39141", "<&>\"'abc", true).unwrap();
        assert_eq!(name, "Pleamar-45535041C39141");
        assert!(xml.contains("<keyMaterial>&lt;&amp;&gt;&quot;&apos;abc</keyMaterial>"));
        assert!(personal_profile("</hex>", "", false).is_err());
        assert!(personal_profile("01", "short", true).is_err());
        assert!(personal_profile("01", "bad\npass", true).is_err());
        assert!(!personal_profile("00FF", "", false).unwrap().1.contains("sharedKey"));
    }
}
