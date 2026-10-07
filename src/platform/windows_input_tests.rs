//! Real Win32 message/capture tests on hidden, owned windows. No mouse or
//! keyboard injection, visible windows, GPU, or system settings are required.
use super::*;
use std::sync::mpsc::{channel, Receiver};

struct Probe {
    hwnd: HWND,
    _state: Arc<WindowState>,
    events: Receiver<ToRender>,
}
impl Probe {
    fn new() -> Self {
        Self::configured(|_| {})
    }
    fn configured(configure: impl FnOnce(&mut WindowState)) -> Self {
        static CLASS: OnceLock<()> = OnceLock::new();
        let instance = HINSTANCE(unsafe { GetModuleHandleW(None).unwrap() }.0);
        CLASS.get_or_init(|| {
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(window_proc), hInstance: instance,
                lpszClassName: w!("PleamarInputTest"), ..Default::default()
            };
            assert_ne!(unsafe { RegisterClassExW(&class) }, 0);
        });
        let (to_render, events) = channel();
        let mut state = WindowState {
            id: 0, which: 0, hwnd: AtomicIsize::new(0), input_hwnd: AtomicIsize::new(0),
            region_pending: AtomicBool::new(false), to_render, origin: (0.0, 0.0),
            scale: AtomicU32::new(1.0f32.to_bits()), boxes: Mutex::default(),
            cursor: AtomicU8::new(0), keyboard: AtomicU8::new(0),
            mouse_inside: AtomicBool::new(false), mouse_buttons: AtomicU8::new(0),
            right_click_quits: false, hidden_from_captures: AtomicBool::new(false), is_window: true,
            placement: Mutex::new(None), appbar: AtomicBool::new(false), fullscreen: AtomicBool::new(false), gone: AtomicBool::new(false),
            surrogate: Mutex::new(None), monitor_name: String::new(), copy: 0, popup: None,
            output: Mutex::new(Monitor { rect: RECT::default(), name: String::new(), scale: 1.0, mhz: 0 }),
            popup_armed: AtomicBool::new(false), released: AtomicBool::new(false), backdrop: OnceLock::new(),
        };
        configure(&mut state);
        let state = Arc::new(state);
        let hwnd = unsafe { CreateWindowExW(WINDOW_EX_STYLE::default(), w!("PleamarInputTest"),
            w!("Hidden input probe"), WS_POPUP, 0, 0, 100, 100, None, None, Some(instance),
            Some(Arc::as_ptr(&state).cast())) }.unwrap();
        state.hwnd.store(hwnd.0 as isize, Ordering::Relaxed);
        let probe = Self { hwnd, _state: state, events };
        probe.drain();
        probe
    }
    fn send(&self, message: u32, value: usize) {
        unsafe { SendMessageW(self.hwnd, message, Some(WPARAM(value)), Some(LPARAM(0))); }
    }
    fn drain(&self) -> Vec<ToRender> { self.events.try_iter().collect() }
    fn take_reposition(&self) -> bool {
        let mut message = MSG::default();
        loop {
            if !unsafe { PeekMessageW(&mut message, Some(self.hwnd), WM_PLEAMAR_REPOSITION,
                WM_PLEAMAR_REPOSITION, PM_REMOVE) }.as_bool() { return false; }
            // WM_QUIT bypasses PeekMessage's filter; an earlier normal probe
            // posts one when dropped. It is not a request to move this panel.
            if message.message == WM_PLEAMAR_REPOSITION { return true; }
            assert_eq!(message.message, WM_QUIT);
        }
    }
}
impl Drop for Probe {
    fn drop(&mut self) { unsafe { let _ = DestroyWindow(self.hwnd); } }
}

#[test]
fn native_window_output_updates_metadata_without_rewriting_placement_identity() {
    let probe = Probe::configured(|s| { s.monitor_name = "launch-output".into(); });
    let screen = monitor_details(unsafe { MonitorFromWindow(probe.hwnd, MONITOR_DEFAULTTONEAREST) }).unwrap();
    probe._state.output.lock().unwrap().name = "old-output".into();
    let foreground = unsafe { GetForegroundWindow() };
    unsafe { SetWindowPos(probe.hwnd, None, screen.rect.left + 40, screen.rect.top + 40, 120, 100,
        SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER) }.unwrap();
    let events = probe.drain();
    assert!(events.iter().any(|event| matches!(event, ToRender::WindowsOutput(0, name, mhz)
        if name == &screen.name && *mhz == screen.mhz)));
    assert_eq!(probe._state.monitor_name, "launch-output");
    assert_eq!(probe._state.output.lock().unwrap().rect, screen.rect);
    assert_eq!(unsafe { GetForegroundWindow() }, foreground);
    assert!(!unsafe { IsWindowVisible(probe.hwnd) }.as_bool());
    // Model a refresh-rate notification without changing a display setting.
    let changed = Monitor { mhz: screen.mhz + 1000, ..screen };
    probe._state.output_changed(changed.clone());
    assert!(matches!(probe.drain().as_slice(), [ToRender::WindowsOutput(0, _, mhz)] if *mhz == changed.mhz));
    probe._state.output_changed(changed);
    assert!(probe.drain().is_empty(), "unchanged reconciliation should not wake the renderer");
}

#[test]
fn native_decorated_desktop_position_uses_current_output_and_client_corner() {
    let probe = Probe::configured(|s| { s.monitor_name = "launch-output".into(); });
    let screen = monitor_details(unsafe { MonitorFromWindow(probe.hwnd, MONITOR_DEFAULTTONEAREST) }).unwrap();
    unsafe { SetWindowLongPtrW(probe.hwnd, GWL_STYLE, WS_OVERLAPPEDWINDOW.0 as isize); }
    unsafe { SetWindowPos(probe.hwnd, None, screen.rect.left + 30, screen.rect.top + 40, 240, 180,
        SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER) }.unwrap();
    let mut client = POINT::default();
    let mut outer = RECT::default();
    assert!(unsafe { ClientToScreen(probe.hwnd, &mut client) }.as_bool());
    unsafe { GetWindowRect(probe.hwnd, &mut outer) }.unwrap();
    assert_ne!((client.x, client.y), (outer.left, outer.top));
    let window = WindowsWindow(probe._state.clone());
    let scale = probe._state.scale();
    assert_eq!(window.desktop_place(), Some((screen.name,
        (((client.x - screen.rect.left) as f32 / scale).round() as i32,
         ((client.y - screen.rect.top) as f32 / scale).round() as i32))));
    assert!(!unsafe { IsWindowVisible(probe.hwnd) }.as_bool());
}

#[test]
fn native_taskbar_recreation_invalidates_reservation_and_queues_reposition() {
    let probe = Probe::configured(|state| { state.is_window = false; });
    while probe.take_reposition() {}
    // Model the cached registration lost with Explorer. This hidden probe has
    // no placement: neither this test nor the queued message reserves screen space.
    probe._state.appbar.store(true, Ordering::Relaxed);
    let recreated = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    assert_ne!(recreated, 0);
    probe.send(recreated, 0);
    assert!(!probe._state.appbar.load(Ordering::Relaxed), "The old Shell registration remained cached");
    assert!(probe.take_reposition(), "The panel did not request a new reservation/placement");
    assert!(!unsafe { IsWindowVisible(probe.hwnd) }.as_bool());
}

#[test]
fn native_taskbar_recreation_preserves_normal_and_retired_windows() {
    let recreated = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    assert_ne!(recreated, 0);
    for window in [true, false] {
        let probe = Probe::configured(|state| {
            state.is_window = window;
            state.gone.store(!window, Ordering::Relaxed);
        });
        while probe.take_reposition() {}
        probe.send(recreated, 0);
        assert!(!probe.take_reposition(), "A normal or retired window was repositioned");
        assert!(probe.drain().is_empty());
    }
}

#[test]
fn native_retired_panel_ignores_queued_placement() {
    let probe = Probe::configured(|state| {
        state.is_window = false;
        *state.placement.lock().unwrap() = Some(Placement {
            monitor: RECT { left: 300, top: 200, right: 1200, bottom: 800 },
            anchor: SurfaceAnchor::TopLeft, margin: [0; 4], width: 240, height: 80,
            exclusive_zone: 0, level: Level::Below,
        });
    });
    while probe.take_reposition() {}
    let mut before = RECT::default();
    unsafe { GetWindowRect(probe.hwnd, &mut before).unwrap(); }
    probe._state.removed();
    probe.drain();
    probe.send(WM_PLEAMAR_REPOSITION, 0);
    let mut after = RECT::default();
    unsafe { GetWindowRect(probe.hwnd, &mut after).unwrap(); }
    assert_eq!(before, after, "An outstanding placement moved a retired surface");
    assert!(probe.drain().is_empty());
    assert!(!unsafe { IsWindowVisible(probe.hwnd) }.as_bool());
}

#[test]
fn native_windows_use_timer_pacing_without_compositor_callbacks() {
    let probe = Probe::new();
    let window = WindowsWindow(probe._state.clone());
    assert!(!window.has_frame_callbacks(), "Win32 never delivers ToRender::Frame callbacks");
}

#[test]
fn native_appbar_yields_to_fullscreen_and_restores_both_windows() {
    let panel = Probe::configured(|state| {
        state.is_window = false;
        *state.placement.lock().unwrap() = Some(Placement {
            monitor: RECT::default(), anchor: SurfaceAnchor::Top, margin: [0; 4],
            width: 100, height: 100, exclusive_zone: 0, level: Level::Above,
        });
    });
    let input = Probe::new();
    panel._state.input_hwnd.store(input.hwnd.0 as isize, Ordering::Relaxed);
    // Only model the registration flag; these hidden windows never reserve
    // work area. Exercise the real HWND z-order and production restacking.
    panel._state.appbar.store(true, Ordering::Relaxed);
    let live = [panel._state.clone()];
    let topmost = |hwnd| unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0 };
    restack_panels(&live);
    assert!(topmost(panel.hwnd) && topmost(input.hwnd));
    unsafe { SendMessageW(panel.hwnd, WM_PLEAMAR_APPBAR,
        Some(WPARAM(windows::Win32::UI::Shell::ABN_FULLSCREENAPP as usize)), Some(LPARAM(1))); }
    assert!(!topmost(panel.hwnd) && !topmost(input.hwnd), "The AppBar still covers a full-screen application");
    restack_panels(&live);
    assert!(!topmost(panel.hwnd) && !topmost(input.hwnd), "Restacking raised the AppBar back over full-screen content");
    unsafe { SendMessageW(panel.hwnd, WM_PLEAMAR_APPBAR,
        Some(WPARAM(windows::Win32::UI::Shell::ABN_FULLSCREENAPP as usize)), Some(LPARAM(0))); }
    restack_panels(&live);
    assert!(topmost(panel.hwnd) && topmost(input.hwnd), "The AppBar failed to restore its scene layer");
    unsafe { SendMessageW(panel.hwnd, WM_PLEAMAR_APPBAR,
        Some(WPARAM(windows::Win32::UI::Shell::ABN_FULLSCREENAPP as usize)), Some(LPARAM(1))); }
    panel._state.placement.lock().unwrap().as_mut().unwrap().level = Level::Below;
    unsafe { SendMessageW(panel.hwnd, WM_PLEAMAR_APPBAR,
        Some(WPARAM(windows::Win32::UI::Shell::ABN_FULLSCREENAPP as usize)), Some(LPARAM(0))); }
    restack_panels(&live);
    assert!(!topmost(panel.hwnd) && !topmost(input.hwnd), "Restoration ignored a scene layer change");
    assert!(!unsafe { IsWindowVisible(panel.hwnd).as_bool() || IsWindowVisible(input.hwnd).as_bool() });
    panel._state.input_hwnd.store(0, Ordering::Relaxed);
}

#[test]
fn native_fullscreen_notice_leaves_unregistered_and_retired_windows_alone() {
    for kind in 0..3 {
        let probe = Probe::configured(|state| {
            state.is_window = kind == 0;
            state.gone.store(kind == 2, Ordering::Relaxed);
            state.appbar.store(kind == 2, Ordering::Relaxed);
        });
        unsafe {
            SetWindowPos(probe.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE).unwrap();
            // A retired window can already reject placement. The notice must
            // preserve its actual z-order, rather than assume SetWindowPos won.
            let before = GetWindowLongPtrW(probe.hwnd, GWL_EXSTYLE);
            SendMessageW(probe.hwnd, WM_PLEAMAR_APPBAR,
                Some(WPARAM(windows::Win32::UI::Shell::ABN_FULLSCREENAPP as usize)), Some(LPARAM(1)));
            assert_eq!(GetWindowLongPtrW(probe.hwnd, GWL_EXSTYLE), before, "kind {kind}");
            assert!(!probe._state.fullscreen.load(Ordering::Relaxed));
        }
    }
}

#[test]
fn native_capture_loss_releases_drag_without_stealing_new_capture() {
    let source = Probe::new();
    let target = Probe::new();
    source.send(WM_LBUTTONDOWN, 0);
    assert_eq!(unsafe { GetCapture() }, source.hwnd);
    source.drain();
    unsafe { SetCapture(target.hwnd); }
    assert_eq!(unsafe { GetCapture() }, target.hwnd);
    let events = source.drain();
    assert!(matches!(events.as_slice(), [ToRender::Pointer(None), ToRender::Button(0, false)]),
        "Capture loss must end the old drag exactly once");
    source.send(WM_CANCELMODE, 0);
    assert!(source.drain().is_empty());
    assert_eq!(unsafe { GetCapture() }, target.hwnd);
}

#[test]
fn native_capture_stays_until_all_held_buttons_are_released() {
    let probe = Probe::new();
    probe.send(WM_LBUTTONDOWN, 0);
    probe.send(WM_RBUTTONDOWN, 0);
    probe.drain();
    probe.send(WM_RBUTTONUP, 0);
    assert_eq!(unsafe { GetCapture() }, probe.hwnd, "Left button still owns capture");
    probe.send(WM_LBUTTONUP, 0);
    assert_eq!(unsafe { GetCapture() }, HWND::default());
    let events = probe.drain();
    assert_eq!(events.iter().filter(|e| matches!(e, ToRender::Button(_, false))).count(), 2);
    assert!(!events.iter().any(|e| matches!(e, ToRender::Pointer(None))),
        "A normal release must preserve the final pointer position");
}

#[test]
fn native_cancel_mode_releases_buttons_once_and_allows_retry() {
    let probe = Probe::new();
    probe.send(WM_LBUTTONDOWN, 0);
    probe.send(WM_MBUTTONDOWN, 0);
    probe.drain();
    probe.send(WM_CANCELMODE, 0);
    assert_eq!(unsafe { GetCapture() }, HWND::default());
    assert!(matches!(probe.drain().as_slice(),
        [ToRender::Pointer(None), ToRender::Button(0, false), ToRender::Button(2, false)]));
    probe.send(WM_CANCELMODE, 0);
    assert!(probe.drain().is_empty());
    probe.send(WM_LBUTTONDOWN, 0);
    assert_eq!(unsafe { GetCapture() }, probe.hwnd);
    probe.send(WM_LBUTTONUP, 0);
    assert_eq!(unsafe { GetCapture() }, HWND::default());
}

#[test]
fn native_focus_loss_discards_incomplete_utf16_character() {
    let probe = Probe::new();
    probe.send(WM_CHAR, 0xd83d);
    probe.send(WM_KILLFOCUS, 0);
    probe.send(WM_CHAR, 0xde80);
    probe.send(WM_CHAR, 'a' as usize);
    let text: String = probe.drain().into_iter().filter_map(|e| match e {
        ToRender::Key(_, text, _, _) => text, _ => None,
    }).collect();
    assert_eq!(text, "a", "A surrogate from the old focus must not join new input");
}

// Use the production reconciliation loop with synthetic monitor observations
// and real hidden HWNDs. The host's display configuration is never changed.
struct Topology {
    live: Vec<Arc<WindowState>>,
    probes: Vec<Probe>,
}
impl Topology {
    fn new() -> Self { Self { live: Vec::new(), probes: Vec::new() } }
    fn reconcile(&mut self, wanted: &[Surface], names: &[&str]) {
        let screens: Vec<_> = names.iter().enumerate().map(|(i, name)| Monitor {
            rect: RECT { left: i as i32 * 1920, top: 0, right: (i as i32 + 1) * 1920, bottom: 1080 },
            name: (*name).into(), scale: 1.0, mhz: 60000,
        }).collect();
        let (tx, _rx) = channel();
        reconcile_on_monitors(wanted, &screens, &tx, &mut self.live, |spec, which, monitor, copy| {
            let probe = Probe::configured(|state| {
                state.id = self.probes.len() as u32;
                state.which = which;
                state.is_window = spec.window.is_some();
                state.monitor_name = monitor.name.clone();
                state.output = Mutex::new(monitor.clone());
                state.copy = copy;
            });
            let state = probe._state.clone();
            self.probes.push(probe);
            Ok(state)
        });
    }
    fn active(&self) -> Vec<u32> {
        self.live.iter().filter(|w| !w.gone.load(Ordering::Relaxed)).map(|w| w.id).collect()
    }
}

#[test]
fn native_window_is_single_across_monitor_and_copy_selection() {
    for screens in [Screens::All, Screens::Named(vec!["A".into(), "A".into(), "B".into()])] {
        let wanted = vec![Surface { window: Some("First".into()), screens, ..Default::default() }];
        let mut topology = Topology::new();
        topology.reconcile(&wanted, &["A", "B"]);
        assert_eq!(topology.active(), [0], "A normal surface must create one window, not one per output/copy");
        topology.reconcile(&wanted, &["B", "A", "C"]);
        assert_eq!(topology.active(), [0]);
        topology.reconcile(&wanted, &[]);
        topology.reconcile(&wanted, &["C"]);
        assert_eq!(topology.active(), [0], "A topology change must not replace a user's window");
    }
}

#[test]
fn native_numbered_windows_keep_identity_position_and_scale() {
    let wanted = vec![
        Surface { window: Some("First".into()), screens: Screens::Number(0), ..Default::default() },
        Surface { window: Some("Second".into()), screens: Screens::Number(1), ..Default::default() },
    ];
    let mut topology = Topology::new();
    topology.reconcile(&wanted, &["A", "B"]);
    assert_eq!(topology.active(), [0, 1], "Each declared window needs its own identity");
    let first = &topology.probes[0];
    unsafe { SetWindowPos(first.hwnd, None, 31, 43, 210, 180, SWP_NOACTIVATE | SWP_NOZORDER).unwrap(); }
    first._state.scale.store(1.75f32.to_bits(), Ordering::Relaxed);
    topology.reconcile(&wanted, &["B", "A"]);
    topology.reconcile(&wanted, &["C"]);
    assert_eq!(topology.active(), [0, 1], "Monitor order/name changes duplicated an existing window");
    let mut rect = RECT::default();
    unsafe { GetWindowRect(topology.probes[0].hwnd, &mut rect).unwrap(); }
    assert_eq!((rect.left, rect.top, rect.right, rect.bottom), (31, 43, 241, 223));
    assert_eq!(topology.live[0].scale(), 1.75, "Reconciliation reset the current window DPI");
}

#[test]
fn native_panels_keep_per_monitor_copies_and_retire_removed_outputs() {
    let wanted = vec![Surface { screens: Screens::Named(vec!["A".into(), "A".into(), "B".into()]), ..Default::default() }];
    let mut topology = Topology::new();
    topology.reconcile(&wanted, &["A", "B"]);
    assert_eq!(topology.active(), [0, 1, 2]);
    topology.reconcile(&wanted, &["A", "B"]);
    assert_eq!(topology.probes.len(), 3);
    topology.reconcile(&wanted, &["B"]);
    assert_eq!(topology.active(), [2]);
    for id in 0..2 {
        let events = topology.probes[id].drain();
        assert!(matches!(events.as_slice(), [ToRender::SheetGone(gone)] if *gone == id as u32));
    }
    topology.reconcile(&wanted, &["B", "A"]);
    assert_eq!(topology.active(), [2, 3, 4], "Returned panels require fresh surface identities");
}

#[test]
#[ignore = "subprocess helper for scripts/windows-display-lifecycle.py; requires a native desktop/GPU"]
fn native_display_lifecycle_helper() {
    let scene = std::env::var("PLEAMAR_DISPLAY_TEST_SCENE").expect("rehearsal scene path");
    let control = std::path::PathBuf::from(std::env::var_os("PLEAMAR_DISPLAY_TEST_CONTROL").expect("rehearsal control path"));
    assert!(std::path::Path::new(&scene).is_file() && control.is_file());
    struct Rehearsal(std::path::PathBuf);
    impl super::super::Platform for Rehearsal {
        fn run(self: Box<Self>, wanted: Vec<Surface>, extra: u32, instance: wgpu::Instance, tx: Sender<ToRender>) {
            run_event_loop_with_monitors(wanted, extra, instance, tx, || {
                // Only hide/reveal the first real output for this process. No
                // fictitious geometry, display settings or production override.
                if std::fs::read_to_string(&self.0).is_ok_and(|s| s.trim() == "connected") {
                    monitors().into_iter().take(1).collect()
                } else { Vec::new() }
            });
        }
    }
    crate::provide_platform(Box::new(Rehearsal(control)));
    crate::run_with(vec!["--scene".into(), scene, "--no-hud".into(), "--stall".into(), "0".into(), "--seconds".into(), "300".into()]);
}


#[test]
fn native_hidden_popup_tracks_client_moves_and_defers_owner_destruction() {
    let parent = Probe::new();
    let popup = Probe::configured(|state| {
        state.is_window = false;
        state.popup = Some(0);
        *state.placement.lock().unwrap() = Some(Placement { monitor:RECT::default(),
            anchor:SurfaceAnchor::TopLeft, margin:[0;4], width:160, height:100,
            exclusive_zone:0, level:Level::Overlay });
    });
    let anchor = popup_position::Tracked::new(&parent._state,&popup._state,[20,30,160,100]);
    let foreground = unsafe { GetForegroundWindow() };
    let screen = monitor_details(unsafe { MonitorFromWindow(parent.hwnd,MONITOR_DEFAULTTONEAREST) }).unwrap();
    let mut positions = Vec::new();
    for offset in [50,130] {
        unsafe { SetWindowPos(parent.hwnd,None,screen.rect.left+offset,screen.rect.top+offset,120,100,
            SWP_NOACTIVATE|SWP_NOZORDER|SWP_NOOWNERZORDER) }.unwrap();
        anchor.move_with_parent(&parent._state,&popup._state).unwrap();
        let expected = popup_position::placement(&parent._state,[20,30,160,100]).unwrap().rect;
        let mut actual = RECT::default();
        unsafe { GetWindowRect(popup.hwnd,&mut actual) }.unwrap();
        assert_eq!(actual,expected);
        positions.push(actual);
    }
    assert_ne!(positions[0],positions[1]);
    parent._state.removed();
    assert!(anchor.refresh());
    assert!(popup._state.gone.load(Ordering::Relaxed));
    assert!(popup.drain().iter().any(|e| matches!(e,ToRender::PopupClosed(0))));
    assert!(anchor.blocks_parent(&parent._state));
    popup._state.released.store(true,Ordering::Release);
    assert!(!anchor.blocks_parent(&parent._state));
    assert!(!anchor.refresh());
    assert!(!unsafe { IsWindowVisible(parent.hwnd) }.as_bool());
    assert!(!unsafe { IsWindowVisible(popup.hwnd) }.as_bool());
    assert_eq!(unsafe { GetForegroundWindow() },foreground);
}
