//! Internal-panel brightness through the Windows WMI provider, with no shell.
use windows::core::{BSTR, PCWSTR, w};
use windows::Win32::{Graphics::Gdi::*, System::{Com::*, Ole::*, Rpc::*, Variant::*, Wmi::*}};
use windows::Win32::UI::WindowsAndMessaging::EDD_GET_DEVICE_INTERFACE_NAME;

pub struct Panel {
    services: IWbemServices,
    instance: String,
    pub current: u8,
    pub levels: Vec<u8>,
}

fn property(object: &IWbemClassObject, name: PCWSTR) -> Result<VARIANT, String> {
    let mut value = VARIANT::default();
    unsafe { object.Get(name, 0, &mut value, None, None) }.map_err(|e| e.to_string())?;
    Ok(value)
}

fn string(object: &IWbemClassObject, name: PCWSTR) -> Result<String, String> {
    BSTR::try_from(&property(object, name)?).map(|s| s.to_string()).map_err(|e| e.to_string())
}

fn query(services: &IWbemServices, text: &str) -> Result<Vec<IWbemClassObject>, String> {
    unsafe {
        let results = services.ExecQuery(&BSTR::from("WQL"), &BSTR::from(text), WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY, None).map_err(|e| e.to_string())?;
        let mut output = Vec::new();
        let started = std::time::Instant::now();
        loop {
            let (mut objects, mut returned) = ([None], 0);
            let status = results.Next(500, &mut objects, &mut returned);
            status.ok().map_err(|e| e.to_string())?;
            if status.0 == WBEM_S_TIMEDOUT.0 { return Err("WMI monitor query timed out".into()); }
            if returned == 0 { break; }
            if let Some(object) = objects[0].take() { output.push(object); }
            if output.len() > 32 || started.elapsed().as_secs() >= 2 { return Err("WMI returned too many monitor records or exceeded its query budget".into()); }
        }
        Ok(output)
    }
}

// EnumDisplayDevices supplies a monitor interface path. WMI appends an
// instance suffix to the corresponding PnP ID. Never fall back to an unrelated
// panel just because it happens to be the only brightness provider.
fn interface_id(value: &str) -> Option<String> {
    let rest = value.strip_prefix(r"\\?\")?;
    let parts: Vec<_> = rest.split('#').collect();
    (parts.len() == 4 && parts[0].eq_ignore_ascii_case("DISPLAY"))
        .then(|| parts[..3].join("\\").to_ascii_uppercase())
}
fn instance_id(value: &str) -> String {
    let value = value.rsplit_once('_').filter(|(_, suffix)| !suffix.is_empty() && suffix.bytes().all(|c| c.is_ascii_digit())).map_or(value, |(id, _)| id);
    value.to_ascii_uppercase()
}

fn monitor_ids(display: &str) -> Vec<String> {
    let name: Vec<u16> = display.encode_utf16().chain([0]).collect();
    let mut ids = Vec::new();
    for index in 0..32 {
        let mut device = DISPLAY_DEVICEW { cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
        if !unsafe { EnumDisplayDevicesW(PCWSTR(name.as_ptr()), index, &mut device, EDD_GET_DEVICE_INTERFACE_NAME) }.as_bool() { break; }
        let len = device.DeviceID.iter().position(|c| *c == 0).unwrap_or(device.DeviceID.len());
        if let Some(id) = interface_id(&String::from_utf16_lossy(&device.DeviceID[..len])) { ids.push(id); }
    }
    ids
}

fn supported_levels(value: &VARIANT) -> Result<Vec<u8>, String> {
    if value.vt() != VT_ARRAY | VT_UI1 { return Err("WMI brightness levels are not a byte array".into()); }
    unsafe {
        let array = value.Anonymous.Anonymous.Anonymous.parray;
        if array.is_null() || SafeArrayGetDim(array) != 1 { return Err("WMI returned an invalid brightness array".into()); }
        let low = SafeArrayGetLBound(array, 1).map_err(|e| e.to_string())?;
        let high = SafeArrayGetUBound(array, 1).map_err(|e| e.to_string())?;
        if high < low || i64::from(high) - i64::from(low) > 100 { return Err("WMI returned an invalid number of brightness levels".into()); }
        let mut levels = Vec::new();
        for index in low..=high {
            let mut level = 0u8;
            SafeArrayGetElement(array, &index, &mut level as *mut _ as _).map_err(|e| e.to_string())?;
            if level > 100 { return Err("WMI brightness must be a percentage".into()); }
            levels.push(level);
        }
        levels.sort_unstable(); levels.dedup();
        Ok(levels)
    }
}

pub fn panel(display: &str) -> Result<Panel, String> {
    let ids = monitor_ids(display);
    if ids.is_empty() { return Err("No monitor interface matches the selected display".into()); }
    unsafe {
        let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        let empty = BSTR::new();
        let services = locator.ConnectServer(&BSTR::from(r"ROOT\WMI"), &empty, &empty, &empty, WBEM_FLAG_CONNECT_USE_MAX_WAIT.0, &empty, None).map_err(|e| e.to_string())?;
        CoSetProxyBlanket(&services, RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE, PCWSTR::null(), RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE, None, EOAC_NONE).map_err(|e| e.to_string())?;
        let mut selected = None;
        for object in query(&services, "SELECT * FROM WmiMonitorBrightness WHERE Active = TRUE")? {
            let instance = string(&object, w!("InstanceName"))?;
            if !ids.contains(&instance_id(&instance)) { continue; }
            if selected.is_some() { return Err("More than one WMI panel matches the selected display".into()); }
            let current = u32::try_from(&property(&object, w!("CurrentBrightness"))?).map_err(|e| e.to_string())?;
            let current = u8::try_from(current).map_err(|e| e.to_string())?;
            let levels = supported_levels(&property(&object, w!("Level"))?)?;
            if !levels.contains(&current) { return Err("The panel reported an unsupported brightness level".into()); }
            selected = Some(Panel { services: services.clone(), instance, current, levels });
        }
        selected.ok_or_else(|| "No active WMI brightness panel matches this display".into())
    }
}

impl Panel {
    pub fn set(&self, level: f64) -> Result<(), String> {
        let wanted = (level * 100.0).round() as u8;
        let nearest = self.levels.iter().min_by_key(|v| v.abs_diff(wanted)).ok_or("The panel has no brightness levels")?;
        let mut target = None;
        for object in query(&self.services, "SELECT * FROM WmiMonitorBrightnessMethods WHERE Active = TRUE")? {
            if string(&object, w!("InstanceName"))?.eq_ignore_ascii_case(&self.instance) {
                if target.is_some() { return Err("Ambiguous WMI brightness method target".into()); }
                target = Some(object);
            }
        }
        let object = target.ok_or("The WMI brightness control disappeared")?;
        let path = string(&object, w!("__PATH"))?;
        unsafe {
            let mut class = None;
            self.services.GetObject(&BSTR::from("WmiMonitorBrightnessMethods"), WBEM_FLAG_RETURN_IMMEDIATELY, None, None, Some(&mut class)).map_err(|e| e.to_string())?;
            let class = class.ok_or("WMI did not return the brightness class")?.GetResultObject(2000).map_err(|e| e.to_string())?;
            let (mut input, mut output) = (None, None);
            class.GetMethod(w!("WmiSetBrightness"), 0, &mut input, &mut output).map_err(|e| e.to_string())?;
            let parameters = input.ok_or("WMI did not describe brightness parameters")?.SpawnInstance(0).map_err(|e| e.to_string())?;
            // WMI represents CIM_UINT32 parameters as VT_I4.
            parameters.Put(w!("Timeout"), 0, &VARIANT::from(0i32), 0).map_err(|e| e.to_string())?;
            parameters.Put(w!("Brightness"), 0, &VARIANT::from(*nearest), 0).map_err(|e| e.to_string())?;
            let mut result = None;
            self.services.ExecMethod(&BSTR::from(path), &BSTR::from("WmiSetBrightness"), WBEM_FLAG_RETURN_IMMEDIATELY, None, &parameters, None, Some(&mut result)).map_err(|e| e.to_string())?;
            let result = result.ok_or("WMI did not return a brightness result")?.GetResultObject(2000).map_err(|e| e.to_string())?;
            let code = u32::try_from(&property(&result, w!("ReturnValue"))?).map_err(|e| e.to_string())?;
            if code != 0 { return Err(format!("WMI rejected the brightness change: {code}")); }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_monitor_identity_without_crossing_to_another_panel() {
        let id = interface_id(r"\\?\DISPLAY#BOE1234#4&abcd&0&UID256#{monitor-guid}").unwrap();
        assert_eq!(id, instance_id(r"DISPLAY\BOE1234\4&abcd&0&UID256_0"));
        assert_ne!(id, instance_id(r"DISPLAY\BOE1234\4&abcd&0&UID257_0"));
        assert!(interface_id(r"\\.\DISPLAY1").is_none());
        assert!(supported_levels(&VARIANT::from(42u32)).is_err());
    }

    #[test]
    fn validates_driver_brightness_steps() {
        fn array(bytes: &[u8]) -> VARIANT {
            unsafe {
                let raw = SafeArrayCreateVector(VT_UI1, 0, bytes.len() as u32);
                assert!(!raw.is_null());
                let mut value = VARIANT::default();
                let inner = &mut *value.Anonymous.Anonymous;
                inner.vt = VT_ARRAY | VT_UI1;
                inner.Anonymous.parray = raw;
                for (index, byte) in bytes.iter().enumerate() {
                    SafeArrayPutElement(raw, &(index as i32), byte as *const _ as _).unwrap();
                }
                value
            }
        }
        assert_eq!(supported_levels(&array(&[100, 30, 30, 10])).unwrap(), [10, 30, 100]);
        assert!(supported_levels(&array(&[101])).is_err());
        assert!(supported_levels(&array(&[0; 102])).is_err());
    }

    #[test]
    #[ignore = "requires a laptop panel; set PLEAMAR_TEST_WMI_DISPLAY to its GDI display name; changes and restores one supported step"]
    fn live_wmi_brightness_roundtrip() {
        let _apartment = super::super::windows_system::Apartment::new().unwrap();
        let display = std::env::var("PLEAMAR_TEST_WMI_DISPLAY").expect("choose the test panel explicitly, e.g. \\\\.\\DISPLAY1");
        let original = panel(&display).unwrap();
        assert!(original.levels.len() > 1);
        let index = original.levels.iter().position(|v| *v == original.current).unwrap();
        let next = original.levels[if index + 1 < original.levels.len() { index + 1 } else { index - 1 }];
        let previous = original.current;
        struct Restore(Panel);
        impl Drop for Restore {
            fn drop(&mut self) {
                if let Err(error) = self.0.set(self.0.current as f64 / 100.0) { eprintln!("internal-panel brightness restore: {error}"); }
            }
        }
        let restore = Restore(original);
        let confirmed = |wanted| {
            let start = std::time::Instant::now();
            loop {
                let current = panel(&display).unwrap().current;
                if current == wanted { break; }
                assert!(start.elapsed().as_secs() < 5, "panel has not confirmed {wanted}% (actual {current}%)");
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        };
        restore.0.set(next as f64 / 100.0).unwrap();
        confirmed(next);
        drop(restore);
        confirmed(previous);
    }
}
