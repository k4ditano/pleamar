//! Native placement uses the outer window rectangle; captures use DWM's visible frame.
use super::*;
use windows::core::w;

fn selected_monitor<'a>(listed: &[Monitor], live: &'a [Monitor], index: usize) -> Result<&'a Monitor, String> {
    let chosen = listed.get(index).ok_or("monitor is not in this desktop catalog; list windows again")?;
    let actual = live.iter().find(|monitor| monitor.name == chosen.name).ok_or("the selected monitor disconnected; list windows again")?;
    if actual.rect != chosen.rect || actual.work != chosen.work {
        return Err("the selected monitor geometry changed; list windows again".into());
    }
    Ok(actual)
}
pub(super) fn destination(index: usize) -> Result<Monitor, String> {
    let live = monitors()?;
    CATALOG.with(|catalog| selected_monitor(&catalog.borrow().monitors, &live, index).cloned())
}

fn dimensions(rect: RECT) -> Result<(i32, i32), String> {
    let width = rect.right.checked_sub(rect.left).filter(|n| *n > 0);
    let height = rect.bottom.checked_sub(rect.top).filter(|n| *n > 0);
    width.zip(height).ok_or_else(|| "invalid window or monitor dimensions".into())
}
fn contains(outer: RECT, inner: RECT) -> bool {
    dimensions(inner).is_ok() && inner.left >= outer.left && inner.top >= outer.top
        && inner.right <= outer.right && inner.bottom <= outer.bottom
}
fn geometry(current: RECT, from: RECT, to: RECT, source_dpi: u32, destination_dpi: u32) -> Result<RECT, String> {
    let (width, height) = dimensions(current)?;
    let (old_width, old_height) = dimensions(from)?;
    let (new_width, new_height) = dimensions(to)?;
    if source_dpi == 0 || destination_dpi == 0 { return Err("monitor DPI is unavailable".into()); }
    let scale = f64::from(destination_dpi) / f64::from(source_dpi);
    let scaled = |size: i32, room: i32| -> Result<i32, String> {
        let wanted = (f64::from(size) * scale).round().min(f64::from(room));
        if !(1.0..=8192.0).contains(&wanted) { return Err("sent window exceeds supported dimensions".into()); }
        Ok(wanted as i32)
    };
    let (w, h) = (scaled(width, new_width)?, scaled(height, new_height)?);
    if u64::from(w as u32) * u64::from(h as u32) > 16_777_216 { return Err("sent window exceeds the capture pixel budget".into()); }
    let position = |at: i32, origin: i32, old_room: i32, old_size: i32, destination: i32, room: i32, size: i32| -> Result<i32, String> {
        let fraction = if old_room > old_size {
            ((i64::from(at) - i64::from(origin)) as f64 / f64::from(old_room - old_size)).clamp(0.0, 1.0)
        } else { 0.5 };
        let offset = (fraction * f64::from(room - size)).round() as i64;
        i32::try_from(i64::from(destination) + offset).map_err(|_| "window placement overflow".into())
    };
    let left = position(current.left, from.left, old_width, width, to.left, new_width, w)?;
    let top = position(current.top, from.top, old_height, height, to.top, new_height, h)?;
    let placed = RECT { left, top, right: left.checked_add(w).ok_or("window placement overflow")?,
        bottom: top.checked_add(h).ok_or("window placement overflow")? };
    if !contains(to, placed) { return Err("sent window is outside the destination work area".into()); }
    Ok(placed)
}

struct HiddenWindow(HWND);
impl Drop for HiddenWindow { fn drop(&mut self) { let _ = unsafe { DestroyWindow(self.0) }; } }
fn monitor_dpi(screen: &Monitor) -> Result<u32, String> {
    dimensions(screen.work)?;
    // GetDpiForMonitor is not specified for per-monitor-aware callers. A tiny,
    // never-shown window reads the actual output DPI without changing focus or
    // depending on a foreign window's DPI-awareness mode.
    let window = HiddenWindow(unsafe { CreateWindowExW(WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
        w!("STATIC"), w!(""), WS_POPUP, screen.work.left, screen.work.top, 1, 1, None, None, None, None) }
        .map_err(|error| format!("cannot read destination DPI: {error}"))?);
    let actual = monitor_info(unsafe { MonitorFromWindow(window.0, MONITOR_DEFAULTTONULL) })
        .ok_or("monitor disconnected while reading DPI")?;
    let dpi = unsafe { GetDpiForWindow(window.0) };
    if dpi == 0 || actual.name != screen.name || actual.rect != screen.rect || actual.work != screen.work {
        return Err("monitor geometry changed while reading DPI".into());
    }
    Ok(dpi)
}
fn outer(entry: &Entry) -> Result<RECT, String> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(entry.identity.window(), &mut rect) }.map_err(|error| error.to_string())?;
    dimensions(rect)?;
    Ok(rect)
}
pub(super) fn send(id: &str, entry: &Entry, destination: &Monitor, epoch: u64) -> Result<(), String> {
    check_epoch(epoch)?;
    let hwnd = entry.identity.window();
    let source = monitor_info(unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL) })
        .ok_or("source monitor disconnected")?;
    // A repeated request to the same output must not recenter or progressively
    // shrink the window by feeding its visible frame back into SetWindowPos.
    if source.name == destination.name { return Ok(()); }
    if unsafe { IsZoomed(hwnd) }.as_bool() { return Err("restore the maximized window before sending it to another monitor".into()); }
    let current = outer(entry)?;
    let wanted = geometry(current, source.work, destination.work, monitor_dpi(&source)?, monitor_dpi(destination)?)?;
    let now = target(id)?;
    check_epoch(epoch)?;
    if now.identity != entry.identity || outer(&now)? != current || now.rect != entry.rect {
        return Err("the window changed while preparing monitor transfer; list windows again".into());
    }
    let (width, height) = dimensions(wanted)?;
    unsafe { SetWindowPos(hwnd, None, wanted.left, wanted.top, width, height,
        SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS) }.map_err(|error| error.to_string())?;
    CATALOG.with(|catalog| { catalog.borrow_mut().shots.remove(id); });
    let start = Instant::now();
    loop {
        check_epoch(epoch)?;
        let now = target(id)?;
        if now.identity != entry.identity { return Err("the input target changed during monitor transfer".into()); }
        let output = monitor_info(unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL) }).ok_or("monitor disconnected during transfer")?;
        if output.name == destination.name && outer(&now)? == wanted { return Ok(()); }
        if start.elapsed() >= Duration::from_secs(2) {
            return Err("the application did not accept the requested monitor and bounds; its position may have changed, so list windows again".into());
        }
        pump();
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rect(x: i32, y: i32, w: i32, h: i32) -> RECT { RECT {left:x,top:y,right:x+w,bottom:y+h} }
    #[test] fn monitor_numbers_keep_catalog_identity_after_hotplug() {
        let make = |name: &str, x| Monitor {name:name.into(),rect:rect(x,0,1920,1080),work:rect(x,0,1920,1040),primary:x==0};
        let listed = [make("first",0),make("second",-1920)];
        let live = [listed[1].clone()];
        assert!(selected_monitor(&listed,&live,0).is_err(), "disconnected output must not retarget the remaining output");
        assert_eq!(selected_monitor(&listed,&live,1).unwrap().name,"second");
        assert!(selected_monitor(&listed,&live,2).is_err());
        let changed = [make("first",200),listed[1].clone()];
        assert!(selected_monitor(&listed,&changed,0).is_err());
        assert!(selected_monitor(&[],&live,0).is_err());
    }
    #[test] fn transfer_preserves_outer_logical_size_across_mixed_dpi() {
        let from = rect(0,0,2560,1400);
        let to = rect(-1920,200,1920,1040);
        let current = rect(800,300,960,800);
        let moved = geometry(current,from,to,144,96).unwrap();
        assert_eq!(moved,rect(-1280,454,640,533));
        let back = geometry(moved,to,from,96,144).unwrap();
        for (a,b) in [(back.left,current.left),(back.top,current.top),(back.right,current.right),(back.bottom,current.bottom)] {
            assert!((a-b).abs() <= 1);
        }
        assert_eq!(geometry(rect(50,60,560,420),from,from,120,120).unwrap(),rect(50,60,560,420));
    }
    #[test] fn lower_resolution_and_partial_offscreen_bounds_remain_reachable() {
        let from = rect(-2560,-1440,2560,1400);
        let to = rect(0,0,1280,680);
        assert_eq!(geometry(rect(-2500,-1400,2500,1300),from,to,96,192).unwrap(),to);
        assert!(contains(to,geometry(rect(-2600,-1460,800,600),from,to,144,96).unwrap()));
    }
    #[test] fn malformed_geometry_dpi_and_oversized_captures_are_refused() {
        let normal = rect(0,0,1920,1040);
        for invalid in [RECT::default(),RECT{left:i32::MIN,right:i32::MAX,top:0,bottom:100}] {
            assert!(geometry(invalid,normal,normal,96,96).is_err());
            assert!(geometry(normal,invalid,normal,96,96).is_err());
        }
        assert!(geometry(normal,normal,normal,0,96).is_err());
        assert!(geometry(normal,normal,normal,96,0).is_err());
        let huge = rect(0,0,6000,6000);
        assert!(geometry(huge,huge,huge,96,96).is_err());
    }
    #[test] fn native_monitor_dpi_read_uses_only_never_shown_windows() {
        let _dpi = Dpi::physical().unwrap();
        let before = unsafe { GetForegroundWindow() };
        for screen in monitors().unwrap() { assert!(monitor_dpi(&screen).unwrap() >= 96); }
        assert_eq!(unsafe { GetForegroundWindow() }, before);
    }
}
