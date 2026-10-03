//! DDC/CI and internal-panel WMI brightness for the selected monitor.
use super::SysValue;
use windows::Win32::{Devices::Display::*, Graphics::Gdi::*, UI::WindowsAndMessaging::GetForegroundWindow};
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::core::BOOL;
use std::sync::Mutex;

static TARGET: Mutex<Option<String>> = Mutex::new(None);
static IO: Mutex<()> = Mutex::new(());

fn logical_monitors() -> Vec<(String, HMONITOR)> {
    unsafe extern "system" fn visit(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let output = unsafe { &mut *(data.0 as *mut Vec<(String, HMONITOR)>) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) }.as_bool() {
            let len = info.szDevice.iter().position(|c| *c == 0).unwrap_or(info.szDevice.len());
            output.push((String::from_utf16_lossy(&info.szDevice[..len]), monitor));
        }
        true.into()
    }
    let mut result = Vec::new();
    unsafe { let _ = EnumDisplayMonitors(None, None, Some(visit), LPARAM(&mut result as *mut _ as isize)); }
    result
}

struct Monitors(Vec<PHYSICAL_MONITOR>);
impl Drop for Monitors { fn drop(&mut self) { unsafe { let _ = DestroyPhysicalMonitors(&self.0); } } }
fn target_monitor() -> Result<(String, HMONITOR), String> {
    let target = TARGET.lock().unwrap().clone();
    let monitors = logical_monitors();
    if let Some(target) = target {
        monitors.into_iter().find(|(name, _)| name == &target).ok_or_else(|| "The selected brightness monitor was removed".into())
    } else {
        let foreground = unsafe { MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY) };
        monitors.into_iter().find(|(_, handle)| *handle == foreground).ok_or_else(|| "No brightness monitor is attached".into())
    }
}
fn monitors(monitor: HMONITOR) -> Result<Monitors, String> {
    unsafe {
        let mut count = 0;
        GetNumberOfPhysicalMonitorsFromHMONITOR(monitor, &mut count).map_err(|e| e.to_string())?;
        if count == 0 || count > 32 { return Err("No physical monitor exposes DDC/CI".into()); }
        let mut monitors = Monitors(vec![PHYSICAL_MONITOR::default(); count as usize]);
        GetPhysicalMonitorsFromHMONITOR(monitor, &mut monitors.0).map_err(|e| e.to_string())?;
        Ok(monitors)
    }
}
fn range(monitor: &PHYSICAL_MONITOR) -> Result<(u32, u32, u32), String> {
    let (mut min, mut current, mut max) = (0, 0, 0);
    unsafe {
        // Some monitors advertise incomplete capability flags. Test the actual
        // read operation before declaring brightness unavailable.
        if GetMonitorBrightness(monitor.hPhysicalMonitor, &mut min, &mut current, &mut max) == 0 {
            return Err(windows::core::Error::from_thread().to_string());
        }
    }
    if max <= min || current < min || current > max { return Err("The monitor returned an invalid brightness range".into()); }
    Ok((min, current, max))
}
fn ddc_snapshot(handle: HMONITOR) -> Result<SysValue, String> {
    let monitors = monitors(handle)?;
    let monitor = &monitors.0[0];
    let (min, current, max) = range(monitor)?;
    let name = monitor.szPhysicalMonitorDescription;
    Ok(SysValue::Map(vec![
        ("present".into(), SysValue::Bool(true)),
        ("backend".into(), SysValue::Text("ddc".into())),
        ("level".into(), SysValue::Num((current - min) as f64 / (max - min) as f64)),
        ("name".into(), SysValue::Text(String::from_utf16_lossy(&name[..name.iter().position(|c| *c == 0).unwrap_or(name.len())]))),
        ("error".into(), SysValue::Text(String::new())),
    ]))
}
fn snapshot() -> Result<SysValue, String> {
    let _io = IO.lock().unwrap();
    let (name, handle) = target_monitor()?;
    match ddc_snapshot(handle) {
        Ok(value) => Ok(value),
        Err(ddc) => {
            let panel = super::windows_brightness_wmi::panel(&name).map_err(|wmi| format!("DDC/CI: {ddc}; internal panel: {wmi}"))?;
            Ok(SysValue::Map(vec![
                ("present".into(), SysValue::Bool(true)),
                ("backend".into(), SysValue::Text("wmi".into())),
                ("name".into(), SysValue::Text(name)),
                ("level".into(), SysValue::Num(panel.current as f64 / 100.0)),
                ("levels".into(), SysValue::List(panel.levels.iter().map(|n| SysValue::Num(*n as f64 / 100.0)).collect())),
                ("error".into(), SysValue::Text(String::new())),
            ]))
        }
    }
}
pub fn read() -> windows::core::Result<SysValue> {
    Ok(snapshot().unwrap_or_else(|error| SysValue::Map(vec![
        ("present".into(), SysValue::Bool(false)), ("error".into(), SysValue::Text(error)),
    ])))
}
pub fn command(name: &str, args: &[SysValue]) -> Result<(), String> {
    if let ("brightness.monitor", [SysValue::Text(name)]) = (name, args) {
        if !logical_monitors().iter().any(|(id, _)| id == name) { return Err("The selected monitor is not attached".into()); }
        *TARGET.lock().unwrap() = Some(name.clone());
        return Ok(());
    }
    let ("brightness.level", [SysValue::Num(level)]) = (name, args) else { return Err("brightness.level takes a number from 0 to 1".into()); };
    if !level.is_finite() || !(0.0..=1.0).contains(level) { return Err("brightness must be between 0 and 1".into()); }
    let _io = IO.lock().unwrap();
    let (name, handle) = target_monitor()?;
    let ddc = monitors(handle).and_then(|monitors| range(&monitors.0[0]).map(|range| (monitors, range)));
    match ddc {
        Ok((monitors, (min, _, max))) => {
            let value = min + ((max - min) as f64 * level).round() as u32;
            if unsafe { SetMonitorBrightness(monitors.0[0].hPhysicalMonitor, value) } == 0 {
                return Err(windows::core::Error::from_thread().to_string());
            }
            Ok(())
        }
        Err(ddc) => super::windows_brightness_wmi::panel(&name).and_then(|panel| panel.set(*level))
            .map_err(|wmi| format!("DDC/CI: {ddc}; internal panel: {wmi}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_brightness_never_reaches_a_monitor() {
        for level in [f64::NAN, f64::INFINITY, -0.1, 1.1] { assert!(command("brightness.level", &[SysValue::Num(level)]).is_err()); }
    }
    #[test]
    #[ignore = "requires DDC/CI hardware; changes and restores the foreground monitor brightness"]
    fn live_brightness_roundtrip() {
        let monitors = monitors(target_monitor().unwrap().1).unwrap();
        let monitor = &monitors.0[0];
        let (min, original, max) = range(monitor).unwrap();
        struct Restore(windows::Win32::Foundation::HANDLE, u32);
        impl Drop for Restore { fn drop(&mut self) { assert_ne!(unsafe { SetMonitorBrightness(self.0, self.1) }, 0); } }
        let restore = Restore(monitor.hPhysicalMonitor, original);
        let next = if original < max { original + 1 } else { original - 1 };
        // Keep this exact handle through the round trip even if focus changes.
        assert!(next >= min);
        assert_ne!(unsafe { SetMonitorBrightness(monitor.hPhysicalMonitor, next) }, 0);
        assert_eq!(range(monitor).unwrap().1, next);
        drop(restore);
        assert_eq!(range(monitor).unwrap().1, original);
    }
}
