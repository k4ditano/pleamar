//! Los servicios que dependen del compositor, hablados con **protocolos estándar**:
//! `wlr-foreign-toplevel-management` para la ventana activa y `ext-workspace` para los
//! escritorios. Los entienden sway, river, niri, Wayfire, COSMIC, Hyprland…, así que lo
//! de aquí vale en cualquier Wayland que los traiga, sin saber cuál es.
//!
//! Hyprland tiene su propio camino (`hyprland.rs`), que da más datos y se prueba antes.
//! Esto es lo que hace que pleamar no sea de un solo compositor.
//!
//! Vive en su propia conexión de Wayland y en su propio hilo: lo de las superficies ya
//! tiene el suyo, y mezclarlos ataría dos cosas que no tienen por qué ir juntas.

use super::Valor;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::workspace::v1::client::{
    ext_workspace_group_handle_v1::{self as grupo, ExtWorkspaceGroupHandleV1},
    ext_workspace_handle_v1::{self as escritorio, ExtWorkspaceHandleV1},
    ext_workspace_manager_v1::{self as escritorios, ExtWorkspaceManagerV1},
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self as ventana, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self as ventanas, ZwlrForeignToplevelManagerV1},
};

#[derive(Default, Clone)]
struct Ventana {
    titulo: String,
    clase: String,
    activa: bool,
}

#[derive(Default, Clone)]
struct Escritorio {
    nombre: String,
    activo: bool,
    /// El orden en que lo contó el compositor: es el número con el que se le llama.
    numero: i64,
    monitor: String,
}

#[derive(Default)]
struct Estado {
    ventanas: HashMap<u32, Ventana>,
    escritorios: HashMap<u32, Escritorio>,
    /// Para `workspaces.focus`: a quién hay que decírselo.
    handles: HashMap<u32, ExtWorkspaceHandleV1>,
    mando: Option<ExtWorkspaceManagerV1>,
    /// De qué monitor es cada grupo de escritorios.
    monitor_del_grupo: HashMap<u32, String>,
    grupo_del_escritorio: HashMap<u32, u32>,
    nombre_de_salida: HashMap<u32, String>,
    avisar_ventana: Option<Box<dyn Fn(Valor) + Send>>,
    avisar_escritorios: Option<Box<dyn Fn(Valor) + Send>>,
    ultimo_ventana: String,
    ultimo_escritorios: String,
    siguiente: i64,
}

impl Estado {
    /// Lo que cuenta el servicio `window`: la ventana que tiene el foco.
    fn contar_ventana(&mut self) {
        let Some(avisar) = &self.avisar_ventana else { return };
        let activa = self.ventanas.values().find(|v| v.activa).cloned().unwrap_or_default();
        let v = Valor::Mapa(vec![("title".into(), Valor::Texto(activa.titulo)), ("class".into(), Valor::Texto(activa.clase))]);
        let huella = format!("{v:?}");
        if huella != self.ultimo_ventana {
            self.ultimo_ventana = huella;
            avisar(v);
        }
    }

    /// Y `workspaces`: cuál está activo y cuáles hay. El `id` es el orden en que el
    /// compositor los contó, para que `workspaces.focus(3)` signifique lo mismo aquí
    /// que en Hyprland, que los numera.
    fn contar_escritorios(&mut self) {
        let Some(avisar) = &self.avisar_escritorios else { return };
        let mut lista: Vec<&Escritorio> = self.escritorios.values().collect();
        lista.sort_by_key(|e| e.numero);
        let activo = lista.iter().find(|e| e.activo).map_or(0, |e| e.numero);
        let v = Valor::Mapa(vec![
            ("active".into(), Valor::Num(activo as f64)),
            ("list".into(), Valor::Lista(lista.iter().map(|e| Valor::Mapa(vec![
                ("id".into(), Valor::Num(e.numero as f64)),
                ("name".into(), Valor::Texto(e.nombre.clone())),
                // Cuántas ventanas tiene no lo dice este protocolo.
                ("windows".into(), Valor::Num(0.0)),
                ("monitor".into(), Valor::Texto(e.monitor.clone())),
            ])).collect())),
        ]);
        let huella = format!("{v:?}");
        if huella != self.ultimo_escritorios {
            self.ultimo_escritorios = huella;
            avisar(v);
        }
    }
}

/// Lo que hace falta para pedirle algo al compositor desde el hilo de la lógica.
struct Mando {
    conexion: Connection,
    estado: Arc<Mutex<Estado>>,
}
static MANDO: OnceLock<Mando> = OnceLock::new();

fn clave(p: &impl Proxy) -> u32 {
    p.id().protocol_id()
}

impl Dispatch<wl_registry::WlRegistry, ()> for Estado {
    fn event(e: &mut Self, registro: &wl_registry::WlRegistry, ev: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = ev {
            match interface.as_str() {
                "zwlr_foreign_toplevel_manager_v1" => {
                    registro.bind::<ZwlrForeignToplevelManagerV1, _, _>(name, version.min(3), qh, ());
                }
                "ext_workspace_manager_v1" => {
                    e.mando = Some(registro.bind::<ExtWorkspaceManagerV1, _, _>(name, version.min(1), qh, ()));
                }
                "wl_output" => {
                    registro.bind::<wl_output::WlOutput, _, _>(name, version.min(4), qh, ());
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for Estado {
    fn event(e: &mut Self, salida: &wl_output::WlOutput, ev: wl_output::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_output::Event::Name { name } = ev {
            e.nombre_de_salida.insert(clave(salida), name);
        }
    }
}

// ── ventanas ──────────────────────────────────────────────────────

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for Estado {
    fn event(e: &mut Self, _: &ZwlrForeignToplevelManagerV1, ev: ventanas::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let ventanas::Event::Toplevel { toplevel } = ev {
            e.ventanas.insert(clave(&toplevel), Ventana::default());
        }
    }
    // El manager crea los handles: hay que decir con qué datos nacen.
    wayland_client::event_created_child!(Estado, ZwlrForeignToplevelManagerV1, [
        ventanas::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for Estado {
    fn event(e: &mut Self, h: &ZwlrForeignToplevelHandleV1, ev: ventana::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let k = clave(h);
        match ev {
            ventana::Event::Title { title } => e.ventanas.entry(k).or_default().titulo = title,
            ventana::Event::AppId { app_id } => e.ventanas.entry(k).or_default().clase = app_id,
            ventana::Event::State { state } => {
                // La lista de estados viene como bytes; `activated` es el 2.
                let activada = state.chunks_exact(4).any(|b| u32::from_ne_bytes([b[0], b[1], b[2], b[3]]) == ventana::State::Activated as u32);
                e.ventanas.entry(k).or_default().activa = activada;
            }
            ventana::Event::Closed => {
                e.ventanas.remove(&k);
                e.contar_ventana();
            }
            ventana::Event::Done => e.contar_ventana(),
            _ => {}
        }
    }
}

// ── escritorios ───────────────────────────────────────────────────

impl Dispatch<ExtWorkspaceManagerV1, ()> for Estado {
    fn event(e: &mut Self, _: &ExtWorkspaceManagerV1, ev: escritorios::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            escritorios::Event::Workspace { workspace } => {
                e.siguiente += 1;
                let numero = e.siguiente;
                e.escritorios.insert(clave(&workspace), Escritorio { numero, ..Default::default() });
                e.handles.insert(clave(&workspace), workspace);
            }
            escritorios::Event::Done => {
                // Ahora ya se sabe de qué monitor es cada uno.
                let de = e.grupo_del_escritorio.clone();
                for (k, g) in de {
                    if let (Some(m), Some(w)) = (e.monitor_del_grupo.get(&g).cloned(), e.escritorios.get_mut(&k)) {
                        w.monitor = m;
                    }
                }
                e.contar_escritorios();
            }
            _ => {}
        }
    }
    wayland_client::event_created_child!(Estado, ExtWorkspaceManagerV1, [
        escritorios::EVT_WORKSPACE_GROUP_OPCODE => (ExtWorkspaceGroupHandleV1, ()),
        escritorios::EVT_WORKSPACE_OPCODE => (ExtWorkspaceHandleV1, ()),
    ]);
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for Estado {
    fn event(e: &mut Self, g: &ExtWorkspaceGroupHandleV1, ev: grupo::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let k = clave(g);
        match ev {
            grupo::Event::OutputEnter { output } => {
                let nombre = e.nombre_de_salida.get(&clave(&output)).cloned().unwrap_or_default();
                e.monitor_del_grupo.insert(k, nombre);
            }
            grupo::Event::WorkspaceEnter { workspace } => {
                e.grupo_del_escritorio.insert(clave(&workspace), k);
            }
            grupo::Event::WorkspaceLeave { workspace } => {
                e.grupo_del_escritorio.remove(&clave(&workspace));
            }
            grupo::Event::Removed => {
                e.monitor_del_grupo.remove(&k);
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtWorkspaceHandleV1, ()> for Estado {
    fn event(e: &mut Self, w: &ExtWorkspaceHandleV1, ev: escritorio::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let k = clave(w);
        match ev {
            escritorio::Event::Name { name } => {
                // Si se llama con un número, ese es su número: así `focus(3)` es el «3».
                if let Ok(n) = name.parse::<i64>() {
                    if let Some(x) = e.escritorios.get_mut(&k) {
                        x.numero = n;
                    }
                }
                e.escritorios.entry(k).or_default().nombre = name;
            }
            escritorio::Event::State { state } => {
                let activo = match state {
                    wayland_client::WEnum::Value(v) => v.contains(escritorio::State::Active),
                    _ => false,
                };
                e.escritorios.entry(k).or_default().activo = activo;
            }
            escritorio::Event::Removed => {
                e.escritorios.remove(&k);
                e.handles.remove(&k);
                e.grupo_del_escritorio.remove(&k);
            }
            _ => {}
        }
    }
}

// ── el mostrador ──────────────────────────────────────────────────

/// ¿Trae este compositor lo que hace falta? Si no, quien llame se busca la vida.
pub fn servicio(nombre: &str, avisar: Box<dyn Fn(Valor) + Send>) -> bool {
    let estado = match arrancar() {
        Some(e) => e,
        None => return false,
    };
    let mut e = estado.lock().unwrap();
    match nombre {
        "window" => {
            e.avisar_ventana = Some(avisar);
            e.ultimo_ventana.clear();
            e.contar_ventana();
        }
        "workspaces" => {
            if e.mando.is_none() {
                return false;
            }
            e.avisar_escritorios = Some(avisar);
            e.ultimo_escritorios.clear();
            e.contar_escritorios();
        }
        _ => return false,
    }
    true
}

pub fn orden(nombre: &str, args: &[Valor]) -> Result<(), String> {
    let ("workspaces.focus", [Valor::Num(n)]) = (nombre, args) else {
        return Err(format!("this compositor cannot do '{nombre}'"));
    };
    let mando = MANDO.get().ok_or("there are no workspaces that count here")?;
    let e = mando.estado.lock().unwrap();
    let cual = e.escritorios.iter().find(|(_, x)| x.numero == *n as i64).map(|(k, _)| *k);
    let (Some(k), Some(m)) = (cual, e.mando.clone()) else { return Err(format!("there is no workspace {n}")) };
    let Some(h) = e.handles.get(&k) else { return Err(format!("workspace {n} is gone")) };
    h.activate();
    m.commit();
    drop(e);
    mando.conexion.flush().map_err(|e| e.to_string())
}

/// Abre la conexión la primera vez que alguien pregunta, y deja su hilo atendiendo.
fn arrancar() -> Option<Arc<Mutex<Estado>>> {
    if let Some(m) = MANDO.get() {
        return Some(m.estado.clone());
    }
    let conexion = Connection::connect_to_env().ok()?;
    let mut cola = conexion.new_event_queue::<Estado>();
    conexion.display().get_registry(&cola.handle(), ());
    let mut estado = Estado::default();
    // Dos vueltas: los globales, y lo que cuenten al apuntarse.
    cola.roundtrip(&mut estado).ok()?;
    cola.roundtrip(&mut estado).ok()?;
    if estado.ventanas.is_empty() && estado.mando.is_none() {
        return None;
    }
    let estado = Arc::new(Mutex::new(estado));
    let _ = MANDO.set(Mando { conexion: conexion.clone(), estado: estado.clone() });
    let suyo = estado.clone();
    std::thread::Builder::new()
        .name("compositor".into())
        .spawn(move || loop {
            // Lo pendiente se atiende con el cerrojo puesto; la espera, sin él, para que
            // quien quiera mandar algo desde otro hilo no tenga que aguardar a que pase algo.
            {
                let mut e = suyo.lock().unwrap();
                if cola.dispatch_pending(&mut e).is_err() {
                    return;
                }
            }
            let _ = conexion.flush();
            let Some(leer) = cola.prepare_read() else { continue };
            if leer.read().is_err() {
                return;
            }
        })
        .ok()?;
    Some(estado)
}
