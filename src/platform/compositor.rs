//! The services that depend on the compositor, spoken through **standard protocols**:
//! `wlr-foreign-toplevel-management` for the active window and `ext-workspace` for the
//! workspaces. sway, river, niri, Wayfire, COSMIC, Hyprland… understand them, so what
//! is here works on any Wayland that ships them, without knowing which one it is.
//!
//! Hyprland has its own path (`hyprland.rs`), which gives more data and is tried first.
//! This is what keeps pleamar from belonging to a single compositor.
//!
//! It lives on its own Wayland connection and its own thread: the surfaces already
//! have theirs, and mixing them would tie together two things that have no reason to go together.

use super::SysValue;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::workspace::v1::client::{
    ext_workspace_group_handle_v1::{self as group_handle, ExtWorkspaceGroupHandleV1},
    ext_workspace_handle_v1::{self as workspace_handle, ExtWorkspaceHandleV1},
    ext_workspace_manager_v1::{self as workspace_manager, ExtWorkspaceManagerV1},
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self as toplevel_handle, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self as toplevel_manager, ZwlrForeignToplevelManagerV1},
};

#[derive(Default, Clone)]
struct Toplevel {
    title: String,
    class: String,
    active: bool,
    /// Which monitor it is on: the protocol reports it with `output_enter` and
    /// `output_leave`. It is what a scene with one copy per monitor needs
    /// to know which one you are on: a surface only receives the pointer when
    /// it is over it, so "where the focus is" has to be asked.
    monitor: String,
}

#[derive(Default, Clone)]
struct Workspace {
    name: String,
    active: bool,
    /// The order in which the compositor reported it: it is the number it is called by.
    number: i64,
    monitor: String,
}

#[derive(Default)]
struct State {
    toplevels: HashMap<u32, Toplevel>,
    workspaces: HashMap<u32, Workspace>,
    /// For `workspaces.focus`: whom it has to be told to.
    handles: HashMap<u32, ExtWorkspaceHandleV1>,
    manager: Option<ExtWorkspaceManagerV1>,
    /// Which monitor each workspace group belongs to.
    group_monitor: HashMap<u32, String>,
    workspace_group: HashMap<u32, u32>,
    output_name: HashMap<u32, String>,
    dispatch_window: Option<Box<dyn Fn(SysValue) + Send>>,
    dispatch_workspaces: Option<Box<dyn Fn(SysValue) + Send>>,
    last_window: String,
    last_workspaces: String,
    next: i64,
}

impl State {
    /// What the `window` service reports: the window that has the focus.
    fn report_window(&mut self) {
        let Some(dispatch) = &self.dispatch_window else { return };
        let active = self.toplevels.values().find(|v| v.active).cloned().unwrap_or_default();
        let v = SysValue::Map(vec![("title".into(), SysValue::Text(active.title)), ("class".into(), SysValue::Text(active.class)), ("monitor".into(), SysValue::Text(active.monitor))]);
        let fingerprint = format!("{v:?}");
        if fingerprint != self.last_window {
            self.last_window = fingerprint;
            dispatch(v);
        }
    }

    /// And `workspaces`: which one is active and which ones there are. The `id` is the order in which the
    /// compositor reported them, so that `workspaces.focus(3)` means the same here
    /// as in Hyprland, which numbers them.
    fn report_workspaces(&mut self) {
        let Some(dispatch) = &self.dispatch_workspaces else { return };
        let mut list: Vec<&Workspace> = self.workspaces.values().collect();
        list.sort_by_key(|e| e.number);
        let active = list.iter().find(|e| e.active).map_or(0, |e| e.number);
        let v = SysValue::Map(vec![
            ("active".into(), SysValue::Num(active as f64)),
            ("list".into(), SysValue::List(list.iter().map(|e| SysValue::Map(vec![
                ("id".into(), SysValue::Num(e.number as f64)),
                ("name".into(), SysValue::Text(e.name.clone())),
                // How many windows it has is not something this protocol says.
                ("windows".into(), SysValue::Num(0.0)),
                ("monitor".into(), SysValue::Text(e.monitor.clone())),
                // Whether it is the active one on its monitor: what a per-screen bar needs.
                ("active".into(), SysValue::Bool(e.active)),
            ])).collect())),
        ]);
        let fingerprint = format!("{v:?}");
        if fingerprint != self.last_workspaces {
            self.last_workspaces = fingerprint;
            dispatch(v);
        }
    }
}

/// What is needed to ask the compositor for something from the logic thread.
struct Control {
    connection: Connection,
    state: Arc<Mutex<State>>,
}
static CONTROL: OnceLock<Control> = OnceLock::new();

fn key(p: &impl Proxy) -> u32 {
    p.id().protocol_id()
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(e: &mut Self, registry: &wl_registry::WlRegistry, ev: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = ev {
            match interface.as_str() {
                "zwlr_foreign_toplevel_manager_v1" => {
                    registry.bind::<ZwlrForeignToplevelManagerV1, _, _>(name, version.min(3), qh, ());
                }
                "ext_workspace_manager_v1" => {
                    e.manager = Some(registry.bind::<ExtWorkspaceManagerV1, _, _>(name, version.min(1), qh, ()));
                }
                "wl_output" => {
                    registry.bind::<wl_output::WlOutput, _, _>(name, version.min(4), qh, ());
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(e: &mut Self, output: &wl_output::WlOutput, ev: wl_output::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_output::Event::Name { name } = ev {
            e.output_name.insert(key(output), name);
        }
    }
}

// ── windows ───────────────────────────────────────────────────────

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for State {
    fn event(e: &mut Self, _: &ZwlrForeignToplevelManagerV1, ev: toplevel_manager::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let toplevel_manager::Event::Toplevel { toplevel } = ev {
            e.toplevels.insert(key(&toplevel), Toplevel::default());
        }
    }
    // The manager creates the handles: we have to say what data they are born with.
    wayland_client::event_created_child!(State, ZwlrForeignToplevelManagerV1, [
        toplevel_manager::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for State {
    fn event(e: &mut Self, h: &ZwlrForeignToplevelHandleV1, ev: toplevel_handle::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let k = key(h);
        match ev {
            toplevel_handle::Event::Title { title } => e.toplevels.entry(k).or_default().title = title,
            toplevel_handle::Event::AppId { app_id } => e.toplevels.entry(k).or_default().class = app_id,
            toplevel_handle::Event::State { state } => {
                // The list of states comes as bytes; `activated` is 2.
                let activated = state.chunks_exact(4).any(|b| u32::from_ne_bytes([b[0], b[1], b[2], b[3]]) == toplevel_handle::State::Activated as u32);
                e.toplevels.entry(k).or_default().active = activated;
            }
            toplevel_handle::Event::OutputEnter { output } => {
                let name = e.output_name.get(&key(&output)).cloned().unwrap_or_default();
                e.toplevels.entry(k).or_default().monitor = name;
            }
            toplevel_handle::Event::OutputLeave { output } => {
                let name = e.output_name.get(&key(&output)).cloned().unwrap_or_default();
                let v = e.toplevels.entry(k).or_default();
                if v.monitor == name {
                    v.monitor.clear();
                }
            }
            toplevel_handle::Event::Closed => {
                e.toplevels.remove(&k);
                e.report_window();
            }
            toplevel_handle::Event::Done => e.report_window(),
            _ => {}
        }
    }
}

// ── workspaces ────────────────────────────────────────────────────

impl Dispatch<ExtWorkspaceManagerV1, ()> for State {
    fn event(e: &mut Self, _: &ExtWorkspaceManagerV1, ev: workspace_manager::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            workspace_manager::Event::Workspace { workspace } => {
                e.next += 1;
                let number = e.next;
                e.workspaces.insert(key(&workspace), Workspace { number, ..Default::default() });
                e.handles.insert(key(&workspace), workspace);
            }
            workspace_manager::Event::Done => {
                // Now we know which monitor each one belongs to.
                let groups = e.workspace_group.clone();
                for (k, g) in groups {
                    if let (Some(m), Some(w)) = (e.group_monitor.get(&g).cloned(), e.workspaces.get_mut(&k)) {
                        w.monitor = m;
                    }
                }
                e.report_workspaces();
            }
            _ => {}
        }
    }
    wayland_client::event_created_child!(State, ExtWorkspaceManagerV1, [
        workspace_manager::EVT_WORKSPACE_GROUP_OPCODE => (ExtWorkspaceGroupHandleV1, ()),
        workspace_manager::EVT_WORKSPACE_OPCODE => (ExtWorkspaceHandleV1, ()),
    ]);
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for State {
    fn event(e: &mut Self, g: &ExtWorkspaceGroupHandleV1, ev: group_handle::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let k = key(g);
        match ev {
            group_handle::Event::OutputEnter { output } => {
                let name = e.output_name.get(&key(&output)).cloned().unwrap_or_default();
                e.group_monitor.insert(k, name);
            }
            group_handle::Event::WorkspaceEnter { workspace } => {
                e.workspace_group.insert(key(&workspace), k);
            }
            group_handle::Event::WorkspaceLeave { workspace } => {
                e.workspace_group.remove(&key(&workspace));
            }
            group_handle::Event::Removed => {
                e.group_monitor.remove(&k);
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtWorkspaceHandleV1, ()> for State {
    fn event(e: &mut Self, w: &ExtWorkspaceHandleV1, ev: workspace_handle::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let k = key(w);
        match ev {
            workspace_handle::Event::Name { name } => {
                // If it is named with a number, that is its number: so `focus(3)` is the "3".
                if let Ok(n) = name.parse::<i64>() {
                    if let Some(x) = e.workspaces.get_mut(&k) {
                        x.number = n;
                    }
                }
                e.workspaces.entry(k).or_default().name = name;
            }
            workspace_handle::Event::State { state } => {
                let active = match state {
                    wayland_client::WEnum::Value(v) => v.contains(workspace_handle::State::Active),
                    _ => false,
                };
                e.workspaces.entry(k).or_default().active = active;
            }
            workspace_handle::Event::Removed => {
                e.workspaces.remove(&k);
                e.handles.remove(&k);
                e.workspace_group.remove(&k);
            }
            _ => {}
        }
    }
}

// ── the counter ───────────────────────────────────────────────────

/// Does this compositor ship what is needed? If not, the caller has to fend for itself.
pub fn service(name: &str, dispatch: Box<dyn Fn(SysValue) + Send>) -> bool {
    let state = match start() {
        Some(e) => e,
        None => return false,
    };
    let mut e = state.lock().unwrap();
    match name {
        "window" => {
            e.dispatch_window = Some(dispatch);
            e.last_window.clear();
            e.report_window();
        }
        "workspaces" => {
            if e.manager.is_none() {
                return false;
            }
            e.dispatch_workspaces = Some(dispatch);
            e.last_workspaces.clear();
            e.report_workspaces();
        }
        _ => return false,
    }
    true
}

pub fn command(name: &str, args: &[SysValue]) -> Result<(), String> {
    let ("workspaces.focus", [SysValue::Num(n)]) = (name, args) else {
        return Err(format!("this compositor cannot do '{name}'"));
    };
    let control = CONTROL.get().ok_or("there are no workspaces that count here")?;
    let e = control.state.lock().unwrap();
    let found = e.workspaces.iter().find(|(_, x)| x.number == *n as i64).map(|(k, _)| *k);
    let (Some(k), Some(m)) = (found, e.manager.clone()) else { return Err(format!("there is no workspace {n}")) };
    let Some(h) = e.handles.get(&k) else { return Err(format!("workspace {n} is gone")) };
    h.activate();
    m.commit();
    drop(e);
    control.connection.flush().map_err(|e| e.to_string())
}

/// Opens the connection the first time someone asks, and leaves its thread serving.
fn start() -> Option<Arc<Mutex<State>>> {
    if let Some(m) = CONTROL.get() {
        return Some(m.state.clone());
    }
    let connection = Connection::connect_to_env().ok()?;
    let mut queue = connection.new_event_queue::<State>();
    connection.display().get_registry(&queue.handle(), ());
    let mut state = State::default();
    // Two roundtrips: the globals, and what they report when binding.
    queue.roundtrip(&mut state).ok()?;
    queue.roundtrip(&mut state).ok()?;
    if state.toplevels.is_empty() && state.manager.is_none() {
        return None;
    }
    let state = Arc::new(Mutex::new(state));
    let _ = CONTROL.set(Control { connection: connection.clone(), state: state.clone() });
    let shared = state.clone();
    std::thread::Builder::new()
        .name("compositor".into())
        .spawn(move || loop {
            // Pending events are handled with the lock held; the wait, without it, so that
            // whoever wants to send something from another thread does not have to wait for something to happen.
            {
                let mut e = shared.lock().unwrap();
                if queue.dispatch_pending(&mut e).is_err() {
                    return;
                }
            }
            let _ = connection.flush();
            let Some(read_guard) = queue.prepare_read() else { continue };
            if read_guard.read().is_err() {
                return;
            }
        })
        .ok()?;
    Some(state)
}
