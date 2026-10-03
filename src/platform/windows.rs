//! Win32: transparent DirectComposition windows, monitor placement and input.
//!
//! The renderer still owns every pixel and every animation. This module only
//! turns the scene's surfaces into HWNDs, translates Win32 input into
//! `ToRender`, and tells Windows which logical rectangles accept a click.

use super::PlatformWindow;
use crate::scene::{ToRender, SurfaceAnchor, Cursor, Mods, Level, Screens, Surface, Keyboard};
use crate::gpu;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};
use std::ffi::c_void;
use std::mem::size_of;
use std::num::NonZeroIsize;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak, mpsc::Sender};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW,
    HBRUSH, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW, ScreenToClient, ClientToScreen,
    BeginPaint, EndPaint, FillRect, PAINTSTRUCT, GetStockObject, BLACK_BRUSH,
    CreateRectRgn, CombineRgn, RGN_OR, DeleteObject, SetWindowRgn,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor,
    MDT_EFFECTIVE_DPI, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyNameTextW, GetKeyState, GetAsyncKeyState, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent, GetCapture, SetCapture, ReleaseCapture, SetFocus, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END,
    VK_ESCAPE, VK_HOME, VK_INSERT, VK_LEFT, VK_LWIN, VK_MENU, VK_NEXT,
    VK_PRIOR, VK_RETURN, VK_RIGHT, VK_RWIN, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP, VK_LBUTTON, VK_RBUTTON,
};
use windows::Win32::UI::Shell::{
    ABE_BOTTOM, ABE_LEFT, ABE_RIGHT, ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS,
    ABM_ACTIVATE, ABM_WINDOWPOSCHANGED, ABN_FULLSCREENAPP, ABN_POSCHANGED,
    APPBARDATA, SHAppBarMessage,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, PCWSTR, w};

#[path = "windows_drag.rs"]
mod drag;
pub fn start_drag(text: &str) -> bool { drag::queue(text) }

const WM_PLEAMAR_CURSOR: u32 = WM_APP + 1;
const WM_PLEAMAR_KEYBOARD: u32 = WM_APP + 2;
const WM_PLEAMAR_REPOSITION: u32 = WM_APP + 3;
const WM_PLEAMAR_QUIT: u32 = WM_APP + 4;
const WM_PLEAMAR_APPBAR: u32 = WM_APP + 5;
const WM_PLEAMAR_REGION: u32 = WM_APP + 6;
const WM_PLEAMAR_DRAG: u32 = WM_APP + 7;
const WM_PLEAMAR_TASK: u32 = WM_APP + 8;
const WM_MOUSELEAVE_MSG: u32 = 0x02a3;
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();

#[derive(Clone)]
struct Monitor {
    rect: RECT,
    name: String,
    scale: f32,
    mhz: i32,
}

#[derive(Clone)]
struct Placement {
    monitor: RECT,
    anchor: SurfaceAnchor,
    margin: [i32; 4],
    width: u32,
    height: u32,
    exclusive_zone: i32,
    level: Level,
}

struct WindowState {
    id: u32,
    which: usize,
    hwnd: AtomicIsize,
    input_hwnd: AtomicIsize,
    region_pending: AtomicBool,
    to_render: Sender<ToRender>,
    origin: (f32, f32),
    scale: AtomicU32,
    boxes: Mutex<Vec<[i32; 4]>>,
    cursor: AtomicU8,
    keyboard: AtomicU8,
    mouse_inside: AtomicBool,
    mouse_buttons: AtomicU8,
    right_click_quits: bool,
    is_window: bool,
    placement: Mutex<Option<Placement>>,
    appbar: AtomicBool,
    fullscreen: AtomicBool,
    gone: AtomicBool,
    surrogate: Mutex<Option<u16>>,
    monitor_name: String,
    copy: usize,
    popup: Option<usize>,
    popup_armed: AtomicBool,
    released: AtomicBool,
    backdrop: OnceLock<Option<super::windows_backdrop::Capture>>,
}

impl WindowState {
    fn hwnd(&self) -> HWND {
        HWND(self.hwnd.load(Ordering::Relaxed) as *mut c_void)
    }

    fn scale(&self) -> f32 {
        f32::from_bits(self.scale.load(Ordering::Relaxed)).max(0.25)
    }

    fn input(&self) -> HWND {
        let input = self.input_hwnd.load(Ordering::Relaxed);
        if input == 0 { self.hwnd() } else { HWND(input as *mut c_void) }
    }

    fn pointer(&self, x_px: i32, y_px: i32) {
        if self.popup.is_none() { INPUT_PARENT.store(self.hwnd.load(Ordering::Relaxed), Ordering::Relaxed); }
        let e = self.scale();
        let _ = self.to_render.send(ToRender::Pointer(Some((
            x_px as f32 / e + self.origin.0,
            y_px as f32 / e + self.origin.1,
        ))));
    }

    fn cancel_pointer(&self, hwnd: HWND) {
        let buttons = self.mouse_buttons.swap(0, Ordering::Relaxed);
        drag::press(hwnd, false);
        if buttons == 0 { return; }
        // Capture may move to a different window without any button-up message.
        // Clear the old position before ending its drag; never recapture here.
        let _ = self.to_render.send(ToRender::Pointer(None));
        for button in 0..3 {
            if buttons & (1 << button) != 0 {
                let _ = self.to_render.send(ToRender::Button(button, false));
            }
        }
    }

    fn removed(&self) {
        if !self.gone.swap(true, Ordering::Relaxed) {
            let _ = self.to_render.send(ToRender::SheetGone(self.id));
        }
    }
}

/// The renderer only needs an HWND it can talk to and the shared hit-test state.
struct WindowsWindow(Arc<WindowState>);

impl Drop for WindowsWindow {
    fn drop(&mut self) {
        // Target is dropped before PlatformWindow in both NewSheet and Sheet.
        // The UI thread may now destroy the HWND safely.
        self.0.released.store(true, Ordering::Release);
    }
}

impl PlatformWindow for WindowsWindow {
    fn has_frame_callbacks(&self) -> bool { false }

    fn update_input_region(&self, boxes: &[[i32; 4]]) {
        *self.0.boxes.lock().unwrap() = boxes.to_vec();
        if !self.0.is_window && !self.0.region_pending.swap(true, Ordering::Relaxed) {
            unsafe { let _ = PostMessageW(Some(self.0.hwnd()), WM_PLEAMAR_REGION, WPARAM(0), LPARAM(0)); }
        }
    }

    fn cursor(&self, c: Cursor) {
        self.0.cursor.store(
            match c {
                Cursor::Normal => 0,
                Cursor::Hand => 1,
                Cursor::Text => 2,
                Cursor::Grab => 3,
                Cursor::Grabbing => 4,
            },
            Ordering::Relaxed,
        );
        unsafe {
            let _ = PostMessageW(Some(self.0.hwnd()), WM_PLEAMAR_CURSOR, WPARAM(0), LPARAM(0));
        }
    }

    fn keyboard(&self, t: Keyboard) {
        self.0.keyboard.store(keyboard_num(t), Ordering::Relaxed);
        unsafe {
            let _ = PostMessageW(
                Some(self.0.hwnd()),
                WM_PLEAMAR_KEYBOARD,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }

    fn desktop_place(&self) -> Option<(String, (i32, i32))> {
        let e = &self.0;
        let placement = e.placement.lock().unwrap();
        let p = placement.as_ref()?;
        let mut r = RECT::default();
        unsafe { GetWindowRect(e.hwnd(), &mut r).ok()?; }
        Some((e.monitor_name.clone(), (((r.left - p.monitor.left) as f32 / e.scale()).round() as i32, ((r.top - p.monitor.top) as f32 / e.scale()).round() as i32)))
    }

    fn capture_backdrop(&self, bounds: [i32; 4], on_change: bool) -> bool {
        let e = &self.0;
        let capture = e.backdrop.get_or_init(|| super::windows_backdrop::Capture::new(e.id, e.to_render.clone()).ok());
        let Some(capture) = capture else { return false; };
        let mut origin = POINT::default();
        if !unsafe { ClientToScreen(e.hwnd(), &mut origin) }.as_bool() { return false; }
        let scale = e.scale();
        let x = (bounds[0] as f32 * scale).floor() as i32;
        let y = (bounds[1] as f32 * scale).floor() as i32;
        let right = ((bounds[0] + bounds[2]) as f32 * scale).ceil() as i32;
        let bottom = ((bounds[1] + bounds[3]) as f32 * scale).ceil() as i32;
        capture.request([origin.x + x, origin.y + y, right - x, bottom - y], on_change)
    }

    fn cancel_backdrop(&self) {
        if let Some(Some(capture)) = self.0.backdrop.get() { capture.cancel(); }
    }
}

// Composition alpha does not participate in User32 hit testing. Keep the GPU
// canvas click-through, and give its owned input window only the scene's input
// region. Shadows and particles must not block other applications.
unsafe fn apply_input_region(e: &WindowState) {
    e.region_pending.store(false, Ordering::Relaxed);
    if e.input_hwnd.load(Ordering::Relaxed) == 0 { return; }
    let region = unsafe { CreateRectRgn(0, 0, 0, 0) };
    if region.is_invalid() { return; }
    let scale = e.scale();
    for b in e.boxes.lock().unwrap().iter() {
        let r = physical_input_box(*b, scale);
        let part = unsafe { CreateRectRgn(r[0], r[1], r[2], r[3]) };
        if !part.is_invalid() {
            unsafe { CombineRgn(Some(region), Some(region), Some(part), RGN_OR); let _ = DeleteObject(part.into()); }
        }
    }
    // SetWindowRgn transfers ownership only on success.
    if unsafe { SetWindowRgn(e.input(), Some(region), true) } == 0 {
        unsafe { let _ = DeleteObject(region.into()); }
    }
}

fn physical_input_box(b: [i32; 4], scale: f32) -> [i32; 4] {
    [(b[0] as f32 * scale).floor() as i32, (b[1] as f32 * scale).floor() as i32,
     (b[2] as f32 * scale).ceil() as i32, (b[3] as f32 * scale).ceil() as i32]
}

struct WindowsPlatform {
    windows: Mutex<Vec<Weak<WindowState>>>,
}

static PLATFORM: OnceLock<WindowsPlatform> = OnceLock::new();
static QUIT: AtomicBool = AtomicBool::new(false);
static INPUT_PARENT: AtomicIsize = AtomicIsize::new(0);
static PREVIOUS_APP: Mutex<Option<(isize, u32)>> = Mutex::new(None);
static RESTACK_PENDING: AtomicBool = AtomicBool::new(true);
static RESTACKING: AtomicBool = AtomicBool::new(false);
type PopupRequest = (usize, Option<([i32; 4], (f32, f32))>);
static POPUPS: Mutex<Vec<PopupRequest>> = Mutex::new(Vec::new());
static UI_TASKS: Mutex<Vec<Box<dyn FnOnce() + Send>>> = Mutex::new(Vec::new());

pub(super) fn on_ui_thread(task: Box<dyn FnOnce() + Send>) -> Result<(), String> {
    let platform = PLATFORM.get().ok_or("Windows UI is not running")?;
    let windows = platform.windows.lock().unwrap();
    let window = windows.iter().filter_map(Weak::upgrade).find(|w| !w.gone.load(Ordering::Relaxed)).ok_or("No live Windows surface")?;
    let mut tasks = UI_TASKS.lock().unwrap();
    if tasks.len() >= 8 { return Err("Windows UI request queue is full".into()); }
    tasks.push(task);
    if let Err(error) = unsafe { PostMessageW(Some(window.hwnd()), WM_PLEAMAR_TASK, WPARAM(0), LPARAM(0)) } {
        tasks.pop();
        return Err(error.to_string());
    }
    Ok(())
}

fn keyboard_num(t: Keyboard) -> u8 {
    match t {
        Keyboard::Never => 0,
        Keyboard::OnDemand => 1,
        Keyboard::Always => 2,
    }
}

fn monitor_scale(monitor: HMONITOR) -> f32 {
    let (mut x, mut y) = (96, 96);
    unsafe {
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y).is_err() {
            return 1.0;
        }
    }
    x as f32 / 96.0
}

unsafe extern "system" fn enumerate_monitor(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let monitors = unsafe { &mut *(data.0 as *mut Vec<Monitor>) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if !unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo as *mut MONITORINFO) }.as_bool() {
        return BOOL(1);
    }
    let end = info
        .szDevice
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(info.szDevice.len());
    let name = String::from_utf16_lossy(&info.szDevice[..end]);
    let mut name_w = info.szDevice[..end].to_vec();
    name_w.push(0);
    let mut mode = DEVMODEW {
        dmSize: size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    let hz = if unsafe {
        EnumDisplaySettingsW(PCWSTR(name_w.as_ptr()), ENUM_CURRENT_SETTINGS, &mut mode)
    }
    .as_bool()
    {
        mode.dmDisplayFrequency as i32 * 1000
    } else {
        0
    };
    monitors.push(Monitor {
        rect: info.monitorInfo.rcMonitor,
        name,
        scale: monitor_scale(monitor),
        mhz: hz,
    });
    BOOL(1)
}

fn monitors() -> Vec<Monitor> {
    let mut result = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enumerate_monitor),
            LPARAM((&mut result as *mut Vec<Monitor>) as isize),
        );
    }
    result
}

fn wanted_on(s: &Surface, name: &str, k: usize) -> usize {
    match &s.screens {
        Screens::All => 1,
        Screens::Named(n) => n.iter().filter(|x| x.as_str() == name).count(),
        Screens::Number(x) => (*x == k) as usize,
    }
}

fn px(n: i32, scale: f32) -> i32 {
    (n as f32 * scale).round() as i32
}

fn rect_panel(c: &Placement, scale: f32) -> (RECT, (u32, u32)) {
    let m = c.monitor;
    let margin = [
        px(c.margin[0], scale),
        px(c.margin[1], scale),
        px(c.margin[2], scale),
        px(c.margin[3], scale),
    ];
    let area = RECT {
        left: m.left + margin[3],
        top: m.top + margin[0],
        right: m.right - margin[1],
        bottom: m.bottom - margin[2],
    };
    let monitor_w = (area.right - area.left).max(1);
    let monitor_h = (area.bottom - area.top).max(1);
    let w = if c.width == 0 {
        monitor_w
    } else {
        px(c.width as i32, scale).max(1)
    };
    let h = if c.height == 0 { monitor_h } else { px(c.height as i32, scale).max(1) };
    let center_x = area.left + (monitor_w - w) / 2;
    let center_y = area.top + (monitor_h - h) / 2;
    let (x, y) = match c.anchor {
        SurfaceAnchor::Top => (center_x, area.top),
        SurfaceAnchor::Bottom => (center_x, area.bottom - h),
        SurfaceAnchor::Left => (area.left, center_y),
        SurfaceAnchor::Right => (area.right - w, center_y),
        SurfaceAnchor::TopLeft => (area.left, area.top),
        SurfaceAnchor::TopRight => (area.right - w, area.top),
        SurfaceAnchor::BottomLeft => (area.left, area.bottom - h),
        SurfaceAnchor::BottomRight => (area.right - w, area.bottom - h),
        SurfaceAnchor::Center => (center_x, center_y),
    };
    (
        RECT {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        },
        ((w as f32 / scale).round().max(1.0) as u32, (h as f32 / scale).round().max(1.0) as u32),
    )
}

fn appbar_edge(a: SurfaceAnchor) -> u32 {
    match a {
        SurfaceAnchor::Bottom | SurfaceAnchor::BottomLeft | SurfaceAnchor::BottomRight => ABE_BOTTOM,
        SurfaceAnchor::Left => ABE_LEFT,
        SurfaceAnchor::Right => ABE_RIGHT,
        _ => ABE_TOP,
    }
}

unsafe fn remove_appbar(e: &WindowState) {
    e.fullscreen.store(false, Ordering::Relaxed);
    if e.appbar.swap(false, Ordering::Relaxed) {
        let mut d = APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            hWnd: e.hwnd(),
            ..Default::default()
        };
        unsafe { SHAppBarMessage(ABM_REMOVE, &mut d) };
    }
}

unsafe fn reserve_appbar(e: &WindowState, c: &Placement, scale: f32) -> Option<RECT> {
    if c.exclusive_zone <= 0 || e.is_window {
        unsafe { remove_appbar(e) };
        return None;
    }
    let edge = appbar_edge(c.anchor);
    let thickness = px(c.exclusive_zone, scale).max(1);
    let mut d = APPBARDATA {
        cbSize: size_of::<APPBARDATA>() as u32,
        hWnd: e.hwnd(),
        uEdge: edge,
        uCallbackMessage: WM_PLEAMAR_APPBAR,
        rc: c.monitor,
        ..Default::default()
    };
    if !e.appbar.load(Ordering::Relaxed) {
        if unsafe { SHAppBarMessage(ABM_NEW, &mut d) } == 0 {
            eprintln!("windows · the shell refused the AppBar reservation");
            return None;
        }
        e.appbar.store(true, Ordering::Relaxed);
    }
    match edge {
        ABE_BOTTOM => d.rc.top = d.rc.bottom - thickness,
        ABE_LEFT => d.rc.right = d.rc.left + thickness,
        ABE_RIGHT => d.rc.left = d.rc.right - thickness,
        _ => d.rc.bottom = d.rc.top + thickness,
    }
    unsafe { SHAppBarMessage(ABM_QUERYPOS, &mut d) };
    match edge {
        ABE_BOTTOM => d.rc.top = d.rc.bottom - thickness,
        ABE_LEFT => d.rc.right = d.rc.left + thickness,
        ABE_RIGHT => d.rc.left = d.rc.right - thickness,
        _ => d.rc.bottom = d.rc.top + thickness,
    }
    unsafe { SHAppBarMessage(ABM_SETPOS, &mut d) };
    Some(d.rc)
}

unsafe fn reposition(e: &WindowState) {
    if e.gone.load(Ordering::Relaxed) { return; }
    let Some(c) = e.placement.lock().unwrap().clone() else {
        return;
    };
    let scale = e.scale();
    let reserved = unsafe { reserve_appbar(e, &c, scale) };
    let (mut r, _) = rect_panel(&c, scale);
    // The shell may move our reservation away from another AppBar (normally
    // the Windows taskbar). Keep the panel inside the rectangle it granted.
    if let Some(a) = reserved {
        let w = r.right - r.left;
        let h = r.bottom - r.top;
        match appbar_edge(c.anchor) {
            ABE_BOTTOM => {
                r.top = a.bottom - h;
                r.bottom = a.bottom;
            }
            ABE_LEFT => {
                r.left = a.left;
                r.right = a.left + w;
            }
            ABE_RIGHT => {
                r.left = a.right - w;
                r.right = a.right;
            }
            _ => {
                r.top = a.top;
                r.bottom = a.top + h;
            }
        }
    }
    let z = Some(panel_z(e, c.level));
    let _ = unsafe {
        SetWindowPos(
            e.hwnd(),
            z,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            SWP_NOACTIVATE,
        )
    };
    if e.input_hwnd.load(Ordering::Relaxed) != 0 {
        let _ = unsafe { SetWindowPos(e.input(), z, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOACTIVATE) };
    }
}

fn cursor_for(n: u8) -> PCWSTR {
    match n {
        1 => IDC_HAND,
        2 => IDC_IBEAM,
        3 => IDC_SIZEALL,
        4 => IDC_SIZEALL,
        _ => IDC_ARROW,
    }
}

unsafe fn apply_cursor(e: &WindowState) {
    if let Ok(c) = unsafe { LoadCursorW(None, cursor_for(e.cursor.load(Ordering::Relaxed))) } {
        unsafe { SetCursor(Some(c)) };
    }
}

unsafe fn apply_keyboard(e: &WindowState) {
    if e.is_window {
        return;
    }
    let hwnd = e.input();
    let mut ex = WINDOW_EX_STYLE(unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32);
    if e.keyboard.load(Ordering::Relaxed) == 0 || e.popup.is_some() {
        ex |= WS_EX_NOACTIVATE;
    } else {
        ex &= !WS_EX_NOACTIVATE;
    }
    unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex.0 as isize) };
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED | SWP_NOACTIVATE,
        )
    };
    if e.keyboard.load(Ordering::Relaxed) == 2 && e.popup.is_none() {
        unsafe { let _ = SetForegroundWindow(hwnd); let _ = SetFocus(Some(hwnd)); }
    } else if e.keyboard.load(Ordering::Relaxed) == 0 {
        unsafe { return_keyboard(e); }
    }
}

fn restack_panels(live: &[Arc<WindowState>]) {
    // Win32 has one topmost band; the scene has both top and overlay layers.
    // Keep an outside-click catcher below its overlay even when Windows (or
    // an accessibility client) activates or repositions that catcher.
    let mut panels: Vec<_> = live.iter().filter_map(|v| {
        if v.is_window || v.gone.load(Ordering::Relaxed) || v.fullscreen.load(Ordering::Relaxed) { return None; }
        let level = v.placement.lock().unwrap().as_ref()?.level;
        let rank = if v.popup.is_some() { 3 } else { match level { Level::Above => 1, Level::Overlay => 2, _ => return None } };
        Some((rank, v.id, v))
    }).collect();
    panels.sort_by_key(|(rank, id, _)| (*rank, *id));
    RESTACKING.store(true, Ordering::Relaxed);
    for (_, _, panel) in panels {
        for hwnd in [panel.hwnd(), panel.input()] {
            unsafe { let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER); }
        }
    }
    RESTACKING.store(false, Ordering::Relaxed);
}

fn panel_z(e: &WindowState, level: Level) -> HWND {
    if e.fullscreen.load(Ordering::Relaxed) || matches!(level, Level::Background | Level::Below) {
        HWND_BOTTOM
    } else { HWND_TOPMOST }
}

unsafe fn fullscreen_appbar(e: &WindowState, open: bool) {
    e.fullscreen.store(open, Ordering::Relaxed);
    let level = e.placement.lock().unwrap().as_ref().map(|p| p.level);
    if let Some(level) = level {
        // The transparent drawing and its input proxy must yield together.
        // Restacking and later placement keep this state until Shell clears it.
        for hwnd in [e.hwnd(), e.input()] {
            unsafe { let _ = SetWindowPos(hwnd, Some(panel_z(e, level)), 0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER); }
        }
    }
}

unsafe fn return_keyboard(e: &WindowState) {
    // NOACTIVATE prevents future activation; it does not release existing focus.
    // Only give back focus while our own panel still owns it, never after the
    // user has already switched to another application.
    let foreground = unsafe { GetForegroundWindow() };
    if foreground != e.input() && foreground != e.hwnd() { return; }
    let previous = *PREVIOUS_APP.lock().unwrap();
    let Some((raw, owner)) = previous else { return; };
    let previous = HWND(raw as *mut c_void);
    let mut process = 0;
    unsafe { GetWindowThreadProcessId(previous, Some(&mut process)); }
    if process == owner && unsafe { IsWindowVisible(previous).as_bool() && !IsIconic(previous).as_bool() } {
        unsafe { let _ = SetForegroundWindow(previous); }
    }
}

pub(super) fn capture_window() -> Option<HWND> {
    unsafe {
        let foreground = GetForegroundWindow();
        let mut owner = 0;
        GetWindowThreadProcessId(foreground, Some(&mut owner));
        if owner != 0 && owner != windows::Win32::System::Threading::GetCurrentProcessId() {
            return Some(foreground);
        }
        let (raw, expected) = (*PREVIOUS_APP.lock().unwrap())?;
        let previous = HWND(raw as *mut c_void);
        GetWindowThreadProcessId(previous, Some(&mut owner));
        (owner == expected && IsWindowVisible(previous).as_bool() && !IsIconic(previous).as_bool()).then_some(previous)
    }
}

fn mods() -> Mods {
    let down = |v: VIRTUAL_KEY| unsafe { GetKeyState(v.0 as i32) < 0 };
    Mods {
        ctrl: down(VK_CONTROL),
        alt: down(VK_MENU),
        shift: down(VK_SHIFT),
        logo: down(VK_LWIN) || down(VK_RWIN),
    }
}

fn key_name(vk: u32, lparam: LPARAM) -> String {
    match vk as u16 {
        x if x == VK_ESCAPE.0 => "Escape".into(),
        x if x == VK_RETURN.0 => "Return".into(),
        x if x == VK_BACK.0 => "BackSpace".into(),
        x if x == VK_TAB.0 => "Tab".into(),
        x if x == VK_SPACE.0 => "space".into(),
        x if x == VK_LEFT.0 => "Left".into(),
        x if x == VK_RIGHT.0 => "Right".into(),
        x if x == VK_UP.0 => "Up".into(),
        x if x == VK_DOWN.0 => "Down".into(),
        x if x == VK_HOME.0 => "Home".into(),
        x if x == VK_END.0 => "End".into(),
        x if x == VK_PRIOR.0 => "Page_Up".into(),
        x if x == VK_NEXT.0 => "Page_Down".into(),
        x if x == VK_INSERT.0 => "Insert".into(),
        x if x == VK_DELETE.0 => "Delete".into(),
        0x41..=0x5a => char::from_u32(vk + 32).unwrap().to_string(),
        0x30..=0x39 => char::from_u32(vk).unwrap().to_string(),
        _ => {
            let mut b = [0u16; 64];
            let n = unsafe { GetKeyNameTextW(lparam.0 as i32, &mut b) };
            if n > 0 {
                String::from_utf16_lossy(&b[..n as usize])
            } else {
                format!("0x{vk:x}")
            }
        }
    }
}

fn decode_character(pending: &mut Option<u16>, unit: u16) -> Option<String> {
    if (0xd800..=0xdbff).contains(&unit) { *pending = Some(unit); return None; }
    let high = pending.take();
    let text = if (0xdc00..=0xdfff).contains(&unit) {
        let high = high?;
        String::from_utf16(&[high, unit]).ok()?
    } else {
        char::from_u32(unit as u32)?.to_string()
    };
    (!text.chars().any(char::is_control)).then_some(text)
}
fn state(hwnd: HWND) -> Option<&'static WindowState> {
    let p = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const WindowState;
    unsafe { p.as_ref() }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let c = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        let p = c.lpCreateParams as *const WindowState;
        // The HWND owns one reference until WM_NCDESTROY, even if creation fails.
        unsafe { Arc::increment_strong_count(p); }
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, p as isize) };
    }
    let Some(e) = state(hwnd) else {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    if msg >= 0xc000 && msg == *TASKBAR_CREATED.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) }) {
        if hwnd == e.hwnd() && !e.is_window && e.popup.is_none() && !e.gone.load(Ordering::Relaxed) {
            // Explorer loses its AppBar list on restart. This broadcast also
            // occurs on primary-DPI changes, so remove any surviving entry
            // before the queued placement registers with the current Shell.
            unsafe {
                remove_appbar(e);
                let _ = PostMessageW(Some(hwnd), WM_PLEAMAR_REPOSITION, WPARAM(0), LPARAM(0));
            }
        }
        return LRESULT(0);
    }
    let appbar = !e.is_window && e.popup.is_none() && !e.gone.load(Ordering::Relaxed)
        && e.appbar.load(Ordering::Relaxed);
    if appbar && (msg == WM_ACTIVATE || (msg == WM_WINDOWPOSCHANGED && hwnd == e.hwnd())) {
        // Shell needs these notifications to order other autohide bars on the
        // same edge. Keyboard activation may arrive through the input proxy.
        let mut data = APPBARDATA { cbSize: size_of::<APPBARDATA>() as u32, hWnd: e.hwnd(), ..Default::default() };
        unsafe { SHAppBarMessage(if msg == WM_ACTIVATE { ABM_ACTIVATE } else { ABM_WINDOWPOSCHANGED }, &mut data); }
    }
    match msg {
        WM_WINDOWPOSCHANGED if !e.is_window && !RESTACKING.load(Ordering::Relaxed) => {
            let position = unsafe { &*(lparam.0 as *const WINDOWPOS) };
            if !position.flags.contains(SWP_NOZORDER) { RESTACK_PENDING.store(true, Ordering::Relaxed); }
        }
        WM_ACTIVATE if !e.is_window && low_u16(wparam.0 as isize) != WA_INACTIVE as u16 => {
            let previous = HWND(lparam.0 as *mut c_void);
            let mut process = 0;
            unsafe { GetWindowThreadProcessId(previous, Some(&mut process)); }
            if process != 0 && process != unsafe { windows::Win32::System::Threading::GetCurrentProcessId() } {
                *PREVIOUS_APP.lock().unwrap() = Some((lparam.0, process));
            }
            // DefWindowProc supplies the usual activation/focus behavior.
        }
        WM_NCHITTEST => {
            if hwnd == e.input() && hwnd != e.hwnd() { return LRESULT(HTCLIENT as isize); }
            if e.popup.is_some() { return LRESULT(HTCLIENT as isize); }
            if e.is_window {
                return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
            }
            let mut p = POINT {
                x: low_i16(lparam.0),
                y: high_i16(lparam.0),
            };
            let _ = unsafe { ScreenToClient(hwnd, &mut p) };
            let s = e.scale();
            let (x, y) = (p.x as f32 / s, p.y as f32 / s);
            if e.boxes
                .lock()
                .unwrap()
                .iter()
                .any(|b| x >= b[0] as f32 && x < b[2] as f32 && y >= b[1] as f32 && y < b[3] as f32)
            {
                return LRESULT(HTCLIENT as isize);
            }
            return LRESULT(HTTRANSPARENT as isize);
        }
        WM_MOUSEMOVE => {
            if !e.mouse_inside.swap(true, Ordering::Relaxed) {
                let mut t = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = unsafe { TrackMouseEvent(&mut t) };
            }
            e.pointer(low_i16(lparam.0), high_i16(lparam.0));
            return LRESULT(0);
        }
        WM_MOUSELEAVE_MSG => {
            e.mouse_inside.store(false, Ordering::Relaxed);
            let _ = e.to_render.send(ToRender::Pointer(None));
            return LRESULT(0);
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONUP
        | WM_MBUTTONUP => {
            let down = matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN);
            let button = if matches!(msg, WM_LBUTTONDOWN | WM_LBUTTONUP) {
                0
            } else if matches!(msg, WM_RBUTTONDOWN | WM_RBUTTONUP) {
                1
            } else {
                2
            };
            if button == 1 && down && e.right_click_quits {
                unsafe { PostQuitMessage(0) };
                return LRESULT(0);
            }
            if down {
                // SetCapture can synchronously cancel the previous window's
                // drag. Publish this window's pointer only after that callback.
                if unsafe { GetCapture() } != hwnd { unsafe { SetCapture(hwnd); } }
                e.mouse_buttons.fetch_or(1 << button, Ordering::Relaxed);
            } else {
                e.mouse_buttons.fetch_and(!(1 << button), Ordering::Relaxed);
            }
            e.pointer(low_i16(lparam.0), high_i16(lparam.0));
            if button == 0 { drag::press(hwnd, down); }
            let _ = e.to_render.send(ToRender::Button(button, down));
            if !down && e.mouse_buttons.load(Ordering::Relaxed) == 0 && unsafe { GetCapture() } == hwnd {
                unsafe { let _ = ReleaseCapture(); }
            }
            if down && e.keyboard.load(Ordering::Relaxed) != 0 {
                let _ = unsafe { SetForegroundWindow(hwnd) };
                let _ = unsafe { SetFocus(Some(hwnd)) };
            }
            return LRESULT(0);
        }
        WM_CAPTURECHANGED => {
            e.cancel_pointer(hwnd);
            return LRESULT(0);
        }
        WM_CANCELMODE => {
            e.cancel_pointer(hwnd);
            // A late cancellation for an old surface must not release another
            // surface's capture. DefWindowProc also cancels native menu modes.
            if unsafe { GetCapture() } == hwnd { unsafe { let _ = ReleaseCapture(); } }
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let mut p = POINT { x: low_i16(lparam.0), y: high_i16(lparam.0) };
            unsafe { let _ = ScreenToClient(hwnd, &mut p); }
            e.pointer(p.x, p.y);
            let delta = high_u16(wparam.0 as isize) as i16 as f32 / 120.0;
            let _ = e.to_render.send(ToRender::Wheel(if msg == WM_MOUSEWHEEL {
                delta
            } else {
                -delta
            }));
            return LRESULT(0);
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let m = mods();
            // Let DefWindowProc handle system shortcuts such as Alt+F4.
            if msg == WM_SYSKEYDOWN && wparam.0 == 0x73 {
                return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
            }
            let name = key_name(wparam.0 as u32, lparam);
            let paste = (m.ctrl && name == "v").then(super::clipboard_read).flatten();
            let _ = e.to_render.send(ToRender::Key(name, paste, m, 0));
            return LRESULT(0);
        }
        WM_CHAR => {
            if let Some(text) = decode_character(&mut e.surrogate.lock().unwrap(), wparam.0 as u16) {
                // TranslateMessage owns dead keys, AltGr and UTF-16 composition.
                let _ = e.to_render.send(ToRender::Key(String::new(), Some(text), Mods::default(), 0));
            }
            return LRESULT(0);
        }
        WM_KEYUP | WM_SYSKEYUP => {
            let _ = e
                .to_render
                .send(ToRender::KeyReleased(key_name(wparam.0 as u32, lparam), 0));
            return LRESULT(0);
        }
        WM_SETFOCUS => {
            if e.popup.is_none() { INPUT_PARENT.store(e.hwnd().0 as isize, Ordering::Relaxed); }
            let _ = e.to_render.send(ToRender::KeyboardFocus(true));
            return LRESULT(0);
        }
        WM_KILLFOCUS => {
            *e.surrogate.lock().unwrap() = None;
            let _ = e.to_render.send(ToRender::KeyboardFocus(false));
            return LRESULT(0);
        }
        WM_SIZE => {
            if hwnd == e.input() && hwnd != e.hwnd() { return LRESULT(0); }
            let s = e.scale();
            let new_size = (low_u16(lparam.0) as f32 / s, high_u16(lparam.0) as f32 / s);
            if new_size.0 > 0.0 && new_size.1 > 0.0 {
                let _ = e.to_render.send(ToRender::SheetSize(e.id, new_size));
            }
            return LRESULT(0);
        }
        WM_DPICHANGED => {
            let new_scale = low_u16(wparam.0 as isize) as f32 / 96.0;
            e.scale.store(new_scale.to_bits(), Ordering::Relaxed);
            let _ = e.to_render.send(ToRender::Scale(e.id, new_scale));
            if e.is_window {
                let r = unsafe { &*(lparam.0 as *const RECT) };
                let _ = unsafe {
                    SetWindowPos(
                        hwnd,
                        None,
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOACTIVATE | SWP_NOZORDER,
                    )
                };
            } else {
                unsafe { reposition(e) };
            }
            unsafe { apply_input_region(e); }
            return LRESULT(0);
        }
        WM_DISPLAYCHANGE => {
            if !e.is_window {
                unsafe { reposition(e) };
            }
        }
        WM_SETCURSOR if e.is_window && low_u16(lparam.0) as u32 != HTCLIENT => {
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        WM_SETCURSOR | WM_PLEAMAR_CURSOR => {
            unsafe { apply_cursor(e) };
            return LRESULT(1);
        }
        WM_PLEAMAR_KEYBOARD => {
            unsafe { apply_keyboard(e) };
            return LRESULT(0);
        }
        WM_PLEAMAR_REPOSITION => {
            unsafe { reposition(e) };
            return LRESULT(0);
        }
        WM_PLEAMAR_REGION => {
            unsafe { apply_input_region(e); }
            return LRESULT(0);
        }
        WM_PLEAMAR_APPBAR if appbar && hwnd == e.hwnd() && wparam.0 == ABN_FULLSCREENAPP as usize => {
            unsafe { fullscreen_appbar(e, lparam.0 != 0); }
            return LRESULT(0);
        }
        WM_PLEAMAR_APPBAR if appbar && hwnd == e.hwnd() && wparam.0 == ABN_POSCHANGED as usize => {
            unsafe { reposition(e); }
            return LRESULT(0);
        }
        WM_PLEAMAR_QUIT => {
            unsafe { PostQuitMessage(0) };
            return LRESULT(0);
        }
        WM_CLOSE => {
            if let Some(k) = e.popup {
                let _ = e.to_render.send(ToRender::PopupClosed(k));
                e.removed();
                unsafe { let _ = ShowWindow(e.input(), SW_HIDE); let _ = ShowWindow(e.hwnd(), SW_HIDE); }
            } else {
                unsafe { PostQuitMessage(0) };
            }
            return LRESULT(0);
        }
        WM_DESTROY => {
            drag::revoke(hwnd);
            if hwnd != e.hwnd() { return LRESULT(0); }
            unsafe { remove_appbar(e) };
            e.removed();
            if e.is_window {
                unsafe { PostQuitMessage(0) };
            }
            return LRESULT(0);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_PAINT if hwnd == e.input() && hwnd != e.hwnd() => {
            let mut paint = PAINTSTRUCT::default();
            unsafe {
                let dc = BeginPaint(hwnd, &mut paint);
                FillRect(dc, &paint.rcPaint, HBRUSH(GetStockObject(BLACK_BRUSH).0));
                let _ = EndPaint(hwnd, &paint);
            }
            return LRESULT(0);
        }
        WM_NCDESTROY => {
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            let p = e as *const WindowState;
            unsafe { drop(Arc::from_raw(p)) };
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        WM_PLEAMAR_TASK => {
            let tasks = std::mem::take(&mut *UI_TASKS.lock().unwrap());
            for task in tasks { task(); }
            return LRESULT(0);
        }
        WM_PLEAMAR_DRAG => { drag::perform(e); return LRESULT(0); }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn low_u16(v: isize) -> u16 {
    v as u16
}
fn high_u16(v: isize) -> u16 {
    ((v as usize >> 16) & 0xffff) as u16
}
fn low_i16(v: isize) -> i32 {
    low_u16(v) as i16 as i32
}
fn high_i16(v: isize) -> i32 {
    high_u16(v) as i16 as i32
}

fn register_class(instance: HINSTANCE) -> Result<(), String> {
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
        hbrBackground: HBRUSH(std::ptr::null_mut()),
        lpszClassName: w!("PleamarWindow"),
        ..Default::default()
    };
    let a = unsafe { RegisterClassExW(&class) };
    if a == 0 {
        Err("Win32 could not register the pleamar window class".into())
    } else {
        Ok(())
    }
}

fn create(
    p: &Surface,
    which: usize,
    monitor: &Monitor,
    extra_height: u32,
    id: u32,
    win_instance: HINSTANCE,
    gpu_instance: &wgpu::Instance,
    to_render: &Sender<ToRender>,
    copy: usize,
    popup: Option<(usize, HWND)>,
) -> Result<Arc<WindowState>, String> {
    let is_window = p.window.is_some();
    let height = if p.height == 0 { 0 } else { p.height + if which == 0 { extra_height } else { 0 } };
    let placement = (!is_window).then(|| Placement {
        monitor: monitor.rect,
        anchor: p.anchor,
        margin: p.margin,
        width: p.width,
        height,
        exclusive_zone: if p.reserve_while.is_some() { 0 } else { p.exclusive_zone },
        level: p.level,
    });
    let (r, logical_size) = if let Some(c) = &placement {
        rect_panel(c, monitor.scale)
    } else {
        let w = if p.width == 0 { 640 } else { p.width };
        let h = if height == 0 { 480 } else { height };
        let mut r = RECT {
            left: 0,
            top: 0,
            right: px(w as i32, monitor.scale),
            bottom: px(h as i32, monitor.scale),
        };
        let _ = unsafe {
            AdjustWindowRectExForDpi(
                &mut r,
                WS_OVERLAPPEDWINDOW,
                false,
                WINDOW_EX_STYLE::default(),
                (monitor.scale * 96.0).round() as u32,
            )
        };
        let width = r.right - r.left;
        let height = r.bottom - r.top;
        let x = monitor.rect.left + (monitor.rect.right - monitor.rect.left - width) / 2;
        let y = monitor.rect.top + (monitor.rect.bottom - monitor.rect.top - height) / 2;
        (
            RECT {
                left: x,
                top: y,
                right: x + width,
                bottom: y + height,
            },
            (w, h),
        )
    };
    let initial_keyboard = if p.keyboard_while {
        Keyboard::Never
    } else {
        p.keyboard
    };
    let state = Arc::new(WindowState {
        id,
        which,
        hwnd: AtomicIsize::new(0),
        input_hwnd: AtomicIsize::new(0),
        region_pending: AtomicBool::new(false),
        to_render: to_render.clone(),
        origin: p.origin,
        scale: AtomicU32::new(monitor.scale.to_bits()),
        boxes: Mutex::default(),
        cursor: AtomicU8::new(0),
        keyboard: AtomicU8::new(keyboard_num(initial_keyboard)),
        mouse_inside: AtomicBool::new(false),
        mouse_buttons: AtomicU8::new(0),
        right_click_quits: p.right_click_quits,
        is_window,
        placement: Mutex::new(placement),
        appbar: AtomicBool::new(false),
        fullscreen: AtomicBool::new(false),
        gone: AtomicBool::new(false),
        surrogate: Mutex::new(None),
        monitor_name: monitor.name.clone(),
        copy,
        popup: popup.map(|p| p.0),
        popup_armed: AtomicBool::new(false),
        released: AtomicBool::new(false),
        backdrop: OnceLock::new(),
    });
    let diagnostic = std::env::var_os("PLEAMAR_TEST_WINDOWS").is_some();
    let diagnostic_title = format!("pleamar surface {id} · {}", monitor.name);
    let title: Vec<u16> = p
        .window
        .as_deref()
        .filter(|t| !t.is_empty())
        .unwrap_or(if diagnostic { &diagnostic_title } else { "pleamar" })
        .encode_utf16()
        .chain([0])
        .collect();
    let style = if is_window {
        WS_OVERLAPPEDWINDOW
    } else {
        WS_POPUP
    };
    let mut ex = if is_window {
        WS_EX_APPWINDOW | WS_EX_NOREDIRECTIONBITMAP
    } else {
        WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP | WS_EX_LAYERED | WS_EX_TRANSPARENT
    };
    // Opt-in diagnostic visibility lets UI test tools select a panel without
    // changing its native hit testing, owner relationships, or scene geometry.
    if !is_window && diagnostic {
        ex = (ex & !WS_EX_TOOLWINDOW) | WS_EX_APPWINDOW;
    }
    if !is_window && (initial_keyboard == Keyboard::Never || popup.is_some()) {
        ex |= WS_EX_NOACTIVATE;
    }
    if !is_window && matches!(p.level, Level::Above | Level::Overlay) {
        ex |= WS_EX_TOPMOST;
    }
    let raw = Arc::as_ptr(&state) as *const c_void;
    let hwnd = unsafe {
        CreateWindowExW(
            ex,
            w!("PleamarWindow"),
            PCWSTR(title.as_ptr()),
            style,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            popup.map(|p| p.1),
            None,
            Some(win_instance),
            Some(raw),
        )
    }
    .map_err(|e| format!("Win32 could not create a pleamar window: {e}"))?;
    state.hwnd.store(hwnd.0 as isize, Ordering::Relaxed);
    if !is_window {
        let initialize = || -> Result<(), String> {
        unsafe { SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA) }
            .map_err(|err| format!("Win32 could not initialize the composition canvas: {err}"))?;
        let input_ex = (ex & !(WS_EX_TRANSPARENT | WS_EX_NOREDIRECTIONBITMAP)) | WS_EX_LAYERED;
        let input_title: Vec<u16> = (if diagnostic { format!("pleamar input {id} · {}", monitor.name) } else { "pleamar input".into() }).encode_utf16().chain([0]).collect();
        let input = unsafe {
            CreateWindowExW(input_ex, w!("PleamarWindow"), PCWSTR(input_title.as_ptr()), WS_POPUP,
                r.left, r.top, r.right - r.left, r.bottom - r.top, Some(hwnd), None, Some(win_instance), Some(raw))
        }.map_err(|err| format!("Win32 could not create the input window: {err}"))?;
        state.input_hwnd.store(input.0 as isize, Ordering::Relaxed);
        // Nonzero alpha makes User32 hit-test this canvas. It is clipped to
        // the input region and contains no scene pixels or effects.
        unsafe { SetLayeredWindowAttributes(input, COLORREF(0), 1, LWA_ALPHA) }
            .map_err(|err| format!("Win32 could not initialize the input window: {err}"))?;
        if popup.is_some() {
            *state.boxes.lock().unwrap() = vec![[0, 0, logical_size.0 as i32, logical_size.1 as i32]];
        }
        unsafe { apply_input_region(&state); }
        unsafe { reposition(&state) };
        Ok(())
        };
        if let Err(error) = initialize() {
            unsafe { remove_appbar(&state); let _ = DestroyWindow(hwnd); }
            return Err(error);
        }
    }
    if let Err(error) = drag::register(&state) {
        unsafe { remove_appbar(&state); let _ = DestroyWindow(hwnd); }
        return Err(format!("could not register native drag/drop: {error}"));
    }
    unsafe {
        let _ = ShowWindow(
            hwnd,
            if is_window {
                SW_SHOW
            } else {
                SW_SHOWNOACTIVATE
            },
        );
    }
    if !is_window { unsafe { let _ = ShowWindow(state.input(), SW_SHOWNOACTIVATE); } }
    let mut handle = Win32WindowHandle::new(NonZeroIsize::new(hwnd.0 as isize).unwrap());
    handle.hinstance = NonZeroIsize::new(win_instance.0 as isize);
    let surface = unsafe {
        gpu_instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(RawDisplayHandle::Windows(WindowsDisplayHandle::new())),
            raw_window_handle: RawWindowHandle::Win32(handle),
        })
    }
    .map_err(|e| {
        unsafe { remove_appbar(&state); let _ = DestroyWindow(hwnd); }
        format!("wgpu could not create the DirectComposition surface: {e}")
    })?;
    to_render.send(ToRender::Sheet(Box::new(gpu::NewSheet {
        id,
        target: gpu::Target::Surface(surface),
        window: Box::new(WindowsWindow(state.clone())),
        scale: monitor.scale,
        size: logical_size,
        mhz: monitor.mhz,
        name: monitor.name.clone(),
        view: gpu::View {
            surface: which,
            popup: popup.map(|p| p.0),
            origin: p.origin,
            size: (logical_size.0 as f32, logical_size.1 as f32),
        },
    }))).map_err(|e| {
        drop(e);
        unsafe { remove_appbar(&state); let _ = DestroyWindow(hwnd); }
        "the renderer stopped while opening a window".to_owned()
    })?;
    Ok(state)
}

/// Creates every requested surface and owns the Win32 message loop.
pub fn run_event_loop(
    wanted: Vec<Surface>,
    extra_height: u32,
    instance: wgpu::Instance,
    to_render: Sender<ToRender>,
) {
    run_event_loop_with_monitors(wanted, extra_height, instance, to_render, monitors);
}

fn run_event_loop_with_monitors(
    wanted: Vec<Surface>, extra_height: u32, instance: wgpu::Instance, to_render: Sender<ToRender>,
    mut observe_monitors: impl FnMut() -> Vec<Monitor>,
) {
    let _ole = match drag::Apartment::new() {
        Ok(ole) => ole,
        Err(error) => { eprintln!("windows · cannot initialize OLE: {error}"); std::process::exit(1); }
    };
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let module = match unsafe { GetModuleHandleW(None) } {
        Ok(m) => HINSTANCE(m.0),
        Err(e) => {
            eprintln!("windows · cannot get this process module: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = register_class(module) {
        eprintln!("windows · {e}");
        std::process::exit(1);
    }
    if wanted.iter().any(|s| s.lock_screen) {
        eprintln!("windows · custom lock surfaces are unavailable; Windows owns its secure lock screen");
        std::process::exit(1);
    }
    let platform = PLATFORM.get_or_init(|| WindowsPlatform { windows: Mutex::default() });
    let _ = to_render.send(ToRender::KeyRepeat(None));
    let mut live: Vec<Arc<WindowState>> = Vec::new();
    let mut next_id = 0;
    let mut desktop = observe_monitors();
    reconcile(&wanted, &desktop, extra_height, module, &instance, &to_render, &mut live, &mut next_id);
    if live.is_empty() {
        if wanted.iter().any(|spec| desktop.iter().enumerate().any(|(i, m)| wanted_on(spec, &m.name, i) > 0)) {
            eprintln!("windows · no requested monitor could be opened");
            std::process::exit(1);
        }
        // A saved output may be unplugged at login. Keep Luau and IPC alive
        // while the thread timer waits for a matching monitor to return.
        eprintln!("windows · no requested monitor is available; waiting for one to appear");
    }
    // A thread timer also runs when a monitor disappears and no HWND remains.
    let timer = unsafe { SetTimer(None, 0, 16, None) };
    let mut failed = timer == 0;
    if failed { eprintln!("windows · cannot create the event-loop timer"); }
    let mut last_monitors = std::time::Instant::now();
    let mut last_cursor = None;
    let mut was_down = false;
    let mut msg = MSG::default();
    while !failed && !QUIT.load(Ordering::Relaxed) {
        let status = unsafe { GetMessageW(&mut msg, None, 0, 0) }.0;
        if status < 0 { failed = true; eprintln!("windows · message loop failed: {}", std::io::Error::last_os_error()); }
        if status <= 0 || crate::RENDER_DONE.load(Ordering::SeqCst) { break; }
        unsafe { let _ = TranslateMessage(&msg); DispatchMessageW(&msg); }
        if RESTACK_PENDING.swap(false, Ordering::Relaxed) { restack_panels(&live); }
        if msg.message != WM_TIMER { continue; }
        if super::CURSOR_WANTED.load(Ordering::Relaxed) {
            let mut p = POINT::default();
            if unsafe { GetCursorPos(&mut p) }.is_ok() && last_cursor != Some((p.x, p.y)) {
                if let Some(m) = desktop.iter().find(|m| p.x >= m.rect.left && p.x < m.rect.right && p.y >= m.rect.top && p.y < m.rect.bottom) {
                    let point = (m.rect.left as f32 + (p.x - m.rect.left) as f32 / m.scale,
                                 m.rect.top as f32 + (p.y - m.rect.top) as f32 / m.scale);
                    let outputs = desktop.iter().map(|m| (m.name.clone(), [m.rect.left, m.rect.top,
                        ((m.rect.right - m.rect.left) as f32 / m.scale).round() as i32, ((m.rect.bottom - m.rect.top) as f32 / m.scale).round() as i32])).collect();
                    let _ = to_render.send(ToRender::Cursor(point, outputs));
                    last_cursor = Some((p.x, p.y));
                }
            }
        } else { last_cursor = None; }
        for (k, request) in POPUPS.lock().unwrap().drain(..) {
            for v in live.iter().filter(|v| v.popup == Some(k)) { retire(v); }
            let Some(([x, y, w, h], origin)) = request else { continue };
            let parent = live.iter().filter(|v| v.popup.is_none() && !v.gone.load(Ordering::Relaxed))
                .max_by_key(|v| v.hwnd.load(Ordering::Relaxed) == INPUT_PARENT.load(Ordering::Relaxed));
            if let Some(parent) = parent {
                let scale = parent.scale();
                let mut corner = POINT { x: px(x, scale), y: px(y, scale) };
                unsafe { let _ = ClientToScreen(parent.hwnd(), &mut corner); }
                let monitor = Monitor { rect: RECT { left: corner.x, top: corner.y, right: corner.x + px(w, scale), bottom: corner.y + px(h, scale) }, name: parent.monitor_name.clone(), scale, mhz: 0 };
                let spec = Surface { width: w.max(1) as u32, height: h.max(1) as u32, origin, anchor: SurfaceAnchor::TopLeft, right_click_quits: false, keyboard: Keyboard::OnDemand, ..Surface::default() };
                match create(&spec, parent.which, &monitor, 0, next_id, module, &instance, &to_render, 0, Some((k, parent.hwnd()))) {
                    Ok(v) => { live.push(v); next_id += 1; }
                    Err(e) => { eprintln!("popup · {e}"); let _ = to_render.send(ToRender::PopupClosed(k)); }
                }
            }
        }
        let down = unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 || GetAsyncKeyState(VK_RBUTTON.0 as i32) < 0 };
        if !down {
            // The press that opened a popup belongs to its parent. Dismiss only
            // after that press has been released and a new outside press starts.
            for v in live.iter().filter(|v| v.popup.is_some()) { v.popup_armed.store(true, Ordering::Relaxed); }
        }
        if down && !was_down {
            let mut at = POINT::default();
            unsafe { let _ = GetCursorPos(&mut at); }
            for v in live.iter().filter(|v| v.popup.is_some() && v.popup_armed.load(Ordering::Relaxed) && !v.gone.load(Ordering::Relaxed)) {
                let mut r = RECT::default();
                unsafe { let _ = GetWindowRect(v.hwnd(), &mut r); }
                if at.x < r.left || at.x >= r.right || at.y < r.top || at.y >= r.bottom {
                    let _ = to_render.send(ToRender::PopupClosed(v.popup.unwrap()));
                    retire(v);
                }
            }
        }
        was_down = down;
        live.retain(|v| {
            if v.gone.load(Ordering::Relaxed) && v.released.load(Ordering::Acquire) {
                unsafe { let _ = DestroyWindow(v.hwnd()); }
                false
            } else { true }
        });
        if last_monitors.elapsed() >= std::time::Duration::from_secs(1) {
            desktop = observe_monitors();
            reconcile(&wanted, &desktop, extra_height, module, &instance, &to_render, &mut live, &mut next_id);
            last_cursor = None;
            last_monitors = std::time::Instant::now();
        }
        *platform.windows.lock().unwrap() = live.iter().filter(|v| !v.gone.load(Ordering::Relaxed)).map(Arc::downgrade).collect();
    }
    unsafe { let _ = KillTimer(None, timer); }
    let _ = to_render.send(ToRender::Quit);
    while !crate::RENDER_DONE.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    for v in &live {
        unsafe { remove_appbar(v); let _ = DestroyWindow(v.hwnd()); }
    }
    if failed { super::finish_recordings(); std::process::exit(1); }
}

fn retire(v: &WindowState) {
    unsafe { remove_appbar(v); let _ = ShowWindow(v.input(), SW_HIDE); let _ = ShowWindow(v.hwnd(), SW_HIDE); }
    v.removed();
}

fn reconcile(wanted: &[Surface], screens: &[Monitor], extra: u32, module: HINSTANCE, instance: &wgpu::Instance, tx: &Sender<ToRender>, live: &mut Vec<Arc<WindowState>>, next: &mut u32) {
    reconcile_on_monitors(wanted, screens, tx, live, |spec, which, monitor, copy| {
        let result = create(spec, which, monitor, extra, *next, module, instance, tx, copy, None);
        if result.is_ok() { *next += 1; }
        result
    });
    if let Some(p) = PLATFORM.get() { *p.windows.lock().unwrap() = live.iter().filter(|v| !v.gone.load(Ordering::Relaxed)).map(Arc::downgrade).collect(); }
}

fn reconcile_on_monitors(
    wanted: &[Surface], screens: &[Monitor], tx: &Sender<ToRender>, live: &mut Vec<Arc<WindowState>>,
    mut create_window: impl FnMut(&Surface, usize, &Monitor, usize) -> Result<Arc<WindowState>, String>,
) {
    for v in live.iter().filter(|v| !v.is_window && v.popup.is_none() && !v.gone.load(Ordering::Relaxed)) {
        if !screens.iter().enumerate().any(|(i, m)| m.name == v.monitor_name && wanted_on(&wanted[v.which], &m.name, i) > v.copy) { retire(v); }
    }
    for (which, spec) in wanted.iter().enumerate() {
        for (number, monitor) in screens.iter().enumerate() {
            for copy in 0..wanted_on(spec, &monitor.name, number) {
                // A decorated window belongs to its scene surface, not to an
                // output. Keep the user's HWND when monitors appear or reorder.
                if spec.window.is_some() && live.iter().any(|v| v.which == which && v.is_window && v.popup.is_none() && !v.gone.load(Ordering::Relaxed)) { break; }
                if let Some(v) = live.iter().find(|v| v.which == which && v.monitor_name == monitor.name && v.copy == copy && v.popup.is_none() && !v.gone.load(Ordering::Relaxed)) {
                    // A normal window follows the user's moves and WM_DPICHANGED;
                    // monitor reconciliation must not reset it to its launch scale.
                    if v.is_window { continue; }
                    let changed = v.placement.lock().unwrap().as_ref().is_some_and(|c| c.monitor != monitor.rect) || v.scale() != monitor.scale;
                    if changed {
                        if let Some(c) = v.placement.lock().unwrap().as_mut() { c.monitor = monitor.rect; }
                        v.scale.store(monitor.scale.to_bits(), Ordering::Relaxed);
                        let _ = tx.send(ToRender::Scale(v.id, monitor.scale));
                        unsafe { reposition(v); apply_input_region(v); }
                        if let Some(c) = v.placement.lock().unwrap().as_ref() {
                            let (_, size) = rect_panel(c, monitor.scale);
                            let _ = tx.send(ToRender::SheetSize(v.id, (size.0 as f32, size.1 as f32)));
                        }
                    }
                } else {
                    match create_window(spec, which, monitor, copy) {
                        Ok(v) => live.push(v),
                        Err(e) => eprintln!("windows · {e}"),
                    }
                }
            }
        }
    }
}

pub fn reanchor(which: usize, anchor: SurfaceAnchor) {
    let Some(p) = PLATFORM.get() else { return };
    let mut windows = p.windows.lock().unwrap();
    windows.retain(|v| v.strong_count() > 0);
    for v in windows
        .iter()
        .filter_map(Weak::upgrade)
        .filter(|v| v.which == which && v.popup.is_none())
    {
        if let Some(c) = v.placement.lock().unwrap().as_mut() {
            c.anchor = anchor;
        }
        unsafe {
            let _ = PostMessageW(Some(v.hwnd()), WM_PLEAMAR_REPOSITION, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn relayer(which: usize, level: Level) {
    change_placement(which, |c| c.level = level);
}

pub fn rezone(which: usize, zone: i32) {
    change_placement(which, |c| c.exclusive_zone = zone);
}

fn change_placement(which: usize, change: impl Fn(&mut Placement)) {
    let Some(p) = PLATFORM.get() else { return };
    for v in p.windows.lock().unwrap().iter().filter_map(Weak::upgrade).filter(|v| v.which == which && v.popup.is_none()) {
        if let Some(c) = v.placement.lock().unwrap().as_mut() { change(c); }
        unsafe { let _ = PostMessageW(Some(v.hwnd()), WM_PLEAMAR_REPOSITION, WPARAM(0), LPARAM(0)); }
    }
}

pub fn request_quit() {
    QUIT.store(true, Ordering::Relaxed);
}

pub fn popup(k: usize, what: Option<([i32; 4], (f32, f32))>) {
    POPUPS.lock().unwrap().push((k, what));
}

#[cfg(test)]
#[path = "windows_input_tests.rs"]
mod input_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn placement(anchor: SurfaceAnchor, width: u32, margin: [i32; 4]) -> Placement {
        Placement {
            monitor: RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            anchor,
            margin,
            width,
            height: 72,
            exclusive_zone: 0,
            level: Level::Above,
        }
    }

    #[test]
    fn full_width_respects_margins_and_scale() {
        let (r, size) = rect_panel(&placement(SurfaceAnchor::Bottom, 0, [8, 12, 10, 14]), 1.25);
        assert_eq!((r.left, r.top, r.right, r.bottom), (18, 977, 1905, 1067));
        assert_eq!(size, (1510, 72));
    }

    #[test]
    fn fixed_width_centers_in_available_area() {
        let (r, size) = rect_panel(&placement(SurfaceAnchor::Top, 400, [8, 12, 10, 14]), 1.25);
        assert_eq!((r.left, r.top, r.right, r.bottom), (711, 10, 1211, 100));
        assert_eq!(size, (400, 72));
    }

    #[test]
    fn text_input_handles_unicode_and_surrogates() {
        let mut pending = None;
        assert_eq!(decode_character(&mut pending, 'ñ' as u16), Some("ñ".into()));
        assert_eq!(decode_character(&mut pending, 0xd83d), None);
        assert_eq!(decode_character(&mut pending, 0xde80), Some("🚀".into()));
        assert_eq!(decode_character(&mut pending, 13), None);
        assert_eq!(decode_character(&mut pending, 0xdc00), None);
        assert_eq!(decode_character(&mut pending, 0xd83d), None);
        assert_eq!(decode_character(&mut pending, 'a' as u16), Some("a".into()));
    }

    #[test]
    fn input_region_covers_fractional_dpi_edges() {
        assert_eq!(physical_input_box([-3, 1, 9, 11], 1.25), [-4, 1, 12, 14]);
        assert_eq!(physical_input_box([0, 0, 20, 30], 2.0), [0, 0, 40, 60]);
    }

    #[test]
    fn full_height_and_negative_monitor_coordinates() {
        let mut p = placement(SurfaceAnchor::TopLeft, 0, [8, 12, 10, 14]);
        p.monitor.left = -1920;
        p.monitor.right = 0;
        p.height = 0;
        let (r, size) = rect_panel(&p, 1.25);
        assert_eq!((r.left, r.top, r.right, r.bottom), (-1902, 10, -15, 1067));
        assert_eq!(size, (1510, 846));
    }
}
