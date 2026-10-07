//! Invoke Explorer's accessibility provider on a worker, never from the render
//! thread. Match a live native icon rectangle, not a tooltip or a remembered
//! screen coordinate. Explorer remains the owner of notification-area input.
use super::{SysValue, windows_tray::{self, Identity}};
use std::{sync::Mutex, time::{Duration, Instant}};
use windows::{core::{w, Interface, PCWSTR}, Win32::{Foundation::*, System::Com::*, UI::{Accessibility::*, WindowsAndMessaging::*}}};

fn same_icon(native: RECT, provider: RECT) -> bool {
    // The provider can include padding around the shell's icon rectangle.
    // Require its centre and similar dimensions, never a surrounding panel.
    let nw = native.right - native.left;
    let nh = native.bottom - native.top;
    let pw = provider.right - provider.left;
    let ph = provider.bottom - provider.top;
    nw > 0 && nh > 0 && pw > 0 && ph > 0
        && (native.left + native.right - provider.left - provider.right).abs() <= 4
        && (native.top + native.bottom - provider.top - provider.bottom).abs() <= 4
        && (nw - pw).abs() <= 8 && (nh - ph).abs() <= 8
}

unsafe fn roots(automation: &IUIAutomation, explorer: u32) -> Vec<IUIAutomationElement> { unsafe {
    let mut roots = Vec::new();
    for class in [w!("Shell_TrayWnd"), w!("NotifyIconOverflowWindow"), w!("TopLevelWindowForOverflowXamlIsland")] {
        let Ok(hwnd) = FindWindowW(class, PCWSTR::null()) else { continue; };
        let mut process = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut process));
        if process != explorer || !IsWindowVisible(hwnd).as_bool() { continue; }
        if let Ok(root) = automation.ElementFromHandle(hwnd) { roots.push(root); }
    }
    roots
} }

unsafe fn find(automation: &IUIAutomation, id: &Identity) -> Result<Option<IUIAutomationElement>, String> { unsafe {
    let rect = id.rect()?;
    let condition = automation.CreatePropertyCondition(UIA_ControlTypePropertyId, &windows::Win32::System::Variant::VARIANT::from(UIA_ButtonControlTypeId.0)).map_err(|e| e.to_string())?;
    let mut found = None;
    for root in roots(automation, id.explorer) {
        let Ok(items) = root.FindAll(TreeScope_Descendants, &condition) else { continue; };
        for index in 0..items.Length().unwrap_or(0).min(256) {
            let item = items.GetElement(index).map_err(|e| e.to_string())?;
            let class = item.CurrentClassName().map_err(|e| e.to_string())?.to_string();
            // Exclude chevrons, volume/network, clock and app/taskbar buttons:
            // hidden icons all return the chevron rectangle until expanded.
            let automation_id = item.CurrentAutomationId().map_err(|e| e.to_string())?.to_string();
            if class != "SystemTray.NormalButton" || automation_id != "NotifyItemIcon" { continue; }
            if item.CurrentIsOffscreen().map_err(|e| e.to_string())?.as_bool() { continue; }
            if !same_icon(rect, item.CurrentBoundingRectangle().map_err(|e| e.to_string())?) { continue; }
            if found.is_some() { return Err("Explorer exposed an ambiguous tray icon; no action was sent".into()); }
            found = Some(item);
        }
    }
    Ok(found)
} }

unsafe fn expand(automation: &IUIAutomation, explorer: u32) -> Result<(), String> { unsafe {
    // Locate the overflow control by its stable provider identity, independent
    // of the user's display language. Never click the shared collapsed rect.
    let hwnd = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()).map_err(|e| e.to_string())?;
    let root = automation.ElementFromHandle(hwnd).map_err(|e| e.to_string())?;
    if root.CurrentProcessId().map_err(|e| e.to_string())? as u32 != explorer { return Err("Explorer restarted".into()); }
    let items = root.FindAll(TreeScope_Descendants, &automation.CreateTrueCondition().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut button = None;
    for index in 0..items.Length().map_err(|e| e.to_string())?.min(256) {
        let item = items.GetElement(index).map_err(|e| e.to_string())?;
        if item.CurrentClassName().map_err(|e| e.to_string())?.to_string() == "SystemTray.NormalButton"
            && item.CurrentAutomationId().map_err(|e| e.to_string())?.to_string() == "SystemTrayIcon" {
            if button.is_some() { return Err("Explorer exposed more than one overflow control".into()); }
            button = Some(item);
        }
    }
    let button = button.ok_or("this Explorer version does not expose its notification-area overflow control")?;
    button.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId).and_then(|p| p.Invoke()).map_err(|e| e.to_string())
} }

pub fn command(name: &str, args: &[SysValue]) -> Result<(), String> {
    if !matches!(name, "tray.activate" | "tray.context") { return Err(format!("Windows does not expose '{name}'; applications own their native menus")); }
    // Serialize explicit requests; background catalog refresh never opens UI.
    static ACTION: Mutex<()> = Mutex::new(());
    let _action = ACTION.try_lock().map_err(|_| "another tray action is still pending")?;
    let id = windows_tray::identity(args)?;
    let _apartment = super::windows_system::Apartment::new()?;
    unsafe {
        let automation: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        let options: IUIAutomation2 = automation.cast().map_err(|e| e.to_string())?;
        options.SetConnectionTimeout(1000).and_then(|_| options.SetTransactionTimeout(1000)).map_err(|e| e.to_string())?;
        let mut item = find(&automation, &id)?;
        if item.is_none() {
            expand(&automation, id.explorer)?;
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(2) {
                item = find(&automation, &id)?;
                if item.is_some() { break; }
                std::thread::sleep(Duration::from_millis(40));
            }
        }
        let item = item.ok_or("Explorer does not expose this icon as an actionable accessibility element")?;
        if !same_icon(id.rect()?, item.CurrentBoundingRectangle().map_err(|e| e.to_string())?) { return Err("the tray moved during the request; try again".into()); }
        if name == "tray.activate" {
            item.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId).and_then(|p| p.Invoke()).map_err(|e| e.to_string())
        } else {
            item.cast::<IUIAutomationElement3>().and_then(|p| p.ShowContextMenu()).map_err(|e| e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tray_rectangle_never_matches_an_enclosing_panel_or_neighbour() {
        let icon = RECT { left: 100, top: 100, right: 140, bottom: 140 };
        assert!(same_icon(icon, icon));
        assert!(same_icon(icon, RECT { left: 102, top: 102, right: 138, bottom: 138 }));
        assert!(!same_icon(icon, RECT { left: 140, top: 100, right: 180, bottom: 140 }));
        assert!(!same_icon(icon, RECT { left: 0, top: 0, right: 240, bottom: 240 }));
        assert!(!same_icon(RECT::default(), RECT::default()));
    }
}
