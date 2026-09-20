//! Wayland: una superficie layer-shell por monitor, su escala —también la
//! fraccional— y el ratón. Es lo único del programa que sabe qué es Wayland.

use super::Ventana;
use crate::escena::*;
use crate::gpu;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle};
use smithay_client_toolkit::data_device_manager::{
    data_device::{DataDevice, DataDeviceData, DataDeviceHandler},
    data_offer::{DataOfferHandler, DragOffer},
    data_source::DataSourceHandler,
    DataDeviceManagerState, WritePipe,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers, RepeatInfo},
        pointer::{cursor_shape::CursorShapeManager, PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
        xdg::{
            popup::{Popup, PopupConfigure, PopupHandler},
            XdgPositioner, XdgShell,
        },
        WaylandSurface,
    },
};
use smithay_client_toolkit::reexports::protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{Shape, WpCursorShapeDeviceV1};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::Sender;
use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::reexports::protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter};
use wayland_client::protocol::wl_data_device_manager::DndAction;
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_seat, wl_surface},
    Connection, Proxy, QueueHandle,
};


/// Lo que el render necesita de una superficie de Wayland, y nada más.
struct VentanaWayland {
    wl: wl_surface::WlSurface,
    compositor: CompositorState,
    /// Para cambiar el cursor hace falta el «dispositivo» del puntero y el número
    /// de serie de la última vez que entró: los dos los pone el hilo de Wayland.
    cursores: Arc<Mutex<Option<WpCursorShapeDeviceV1>>>,
    serie: Arc<AtomicU32>,
    /// Una emergente no tiene: el teclado es cosa de su madre.
    capa: Option<LayerSurface>,
}

fn interactividad(t: Teclado) -> KeyboardInteractivity {
    match t {
        Teclado::Nunca => KeyboardInteractivity::None,
        Teclado::AlPulsar => KeyboardInteractivity::OnDemand,
        Teclado::Siempre => KeyboardInteractivity::Exclusive,
    }
}

impl Ventana for VentanaWayland {
    fn region_de_entrada(&self, cajas: &[[i32; 4]]) {
        if let Ok(region) = Region::new(&self.compositor) {
            for c in cajas {
                region.add(c[0], c[1], c[2] - c[0], c[3] - c[1]);
            }
            // Se aplica con el siguiente frame que se presente.
            self.wl.set_input_region(Some(region.wl_region()));
        }
    }

    /// Vale con el siguiente frame que se presente, como la región de entrada.
    fn teclado(&self, t: Teclado) {
        if let Some(capa) = &self.capa {
            capa.set_keyboard_interactivity(interactividad(t));
        }
    }

    fn cursor(&self, c: Cursor) {
        if let Some(d) = self.cursores.lock().unwrap().as_ref() {
            d.set_shape(self.serie.load(Ordering::Relaxed), match c {
                Cursor::Normal => Shape::Default,
                Cursor::Mano => Shape::Pointer,
                Cursor::Texto => Shape::Text,
                Cursor::Agarrar => Shape::Grab,
                Cursor::Agarrando => Shape::Grabbing,
            });
        }
    }
}

/// Una superficie puesta en un monitor.
struct Puesta {
    id: u32,
    /// Qué superficie de la escena es.
    cual: usize,
    capa: LayerSurface,
    salida: wl_output::WlOutput,
    /// Para pintar a una escala que no sea entera.
    ventanilla: Option<WpViewport>,
    _escala: Option<WpFractionalScaleV1>,
    /// La última escala que dijo el compositor. Suele llegar ANTES de que la
    /// superficie esté configurada, cuando el render aún no la conoce.
    escala: f32,
    /// Se le entrega al render cuando el compositor la configura.
    pendiente: Option<(wgpu::Surface<'static>, String, i32)>,
}

struct Estado {
    registro: RegistryState,
    asientos: SeatState,
    salidas: OutputState,
    compositor: CompositorState,
    capas: LayerShell,
    ventanillas: Option<WpViewporter>,
    escalas: Option<WpFractionalScaleManagerV1>,
    conexion: Connection,
    instancia: wgpu::Instance,
    pide: Vec<Superficie>,
    alto_extra: u32,
    puestas: Vec<Puesta>,
    siguiente_id: u32,
    puntero: Option<wl_pointer::WlPointer>,
    teclado: Option<wayland_client::protocol::wl_keyboard::WlKeyboard>,
    formas_de_cursor: Option<CursorShapeManager>,
    cursores: Arc<Mutex<Option<WpCursorShapeDeviceV1>>>,
    serie: Arc<AtomicU32>,
    mods: Mods,
    /// Para recibir lo que se arrastre desde otra aplicación.
    arrastres: Option<DataDeviceManagerState>,
    dispositivo_de_datos: Option<DataDevice>,
    salir: bool,
    a_render: Sender<ARender>,
}

/// Los objetos de Wayland que no nos cuentan nada.
struct Mudo;
impl<I: Proxy> Dispatch2<I, Estado> for Mudo {
    fn event(&self, _: &mut Estado, _: &I, _: I::Event, _: &Connection, _: &QueueHandle<Estado>) {}
}

/// La escala que el compositor prefiere para una superficie, en 120avos.
struct EscalaDe(u32);
impl Dispatch2<WpFractionalScaleV1, Estado> for EscalaDe {
    fn event(&self, e: &mut Estado, _: &WpFractionalScaleV1, ev: wp_fractional_scale_v1::Event, _: &Connection, _: &QueueHandle<Estado>) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = ev {
            let escala = scale as f32 / 120.0;
            if let Some(p) = e.puestas.iter_mut().find(|p| p.id == self.0) {
                p.escala = escala;
            }
            let _ = e.a_render.send(ARender::Escala(self.0, escala));
        }
    }
}

impl Estado {
    fn quiere_en(s: &Superficie, nombre: &str) -> usize {
        match &s.pantallas {
            Pantallas::Todas => 1,
            Pantallas::Estas(n) => n.iter().filter(|x| x.as_str() == nombre).count(),
        }
    }

    /// Pone en un monitor las superficies que la escena pida para él.
    fn poner_en(&mut self, salida: &wl_output::WlOutput, qh: &QueueHandle<Estado>) {
        let Some(info) = self.salidas.info(salida) else { return };
        let nombre = info.name.clone().unwrap_or_default();
        let mhz = info.modes.iter().find(|m| m.current).map_or(0, |m| m.refresh_rate);
        for cual in 0..self.pide.len() {
            let ya = self.puestas.iter().filter(|p| &p.salida == salida && p.cual == cual).count();
            let quiere = Self::quiere_en(&self.pide[cual], &nombre);
            for k in ya..quiere {
            let p = &self.pide[cual];
            // El HUD solo va debajo de la principal.
            let alto = p.alto + if cual == 0 { self.alto_extra } else { 0 };
            let wl = self.compositor.create_surface(qh);
            let nivel = match p.nivel {
                Nivel::Fondo => Layer::Background,
                Nivel::Debajo => Layer::Bottom,
                Nivel::Encima => Layer::Top,
                Nivel::SobreTodo => Layer::Overlay,
            };
            let capa = self.capas.create_layer_surface(qh, wl, nivel, Some("pleamar"), Some(salida));
            capa.set_anchor(match p.ancla {
                Ancla::Arriba => Anchor::TOP,
                Ancla::Abajo => Anchor::BOTTOM,
                Ancla::Izquierda => Anchor::LEFT,
                Ancla::Derecha => Anchor::RIGHT,
                Ancla::ArribaIzquierda => Anchor::TOP | Anchor::LEFT,
                Ancla::ArribaDerecha => Anchor::TOP | Anchor::RIGHT,
                Ancla::AbajoIzquierda => Anchor::BOTTOM | Anchor::LEFT,
                Ancla::AbajoDerecha => Anchor::BOTTOM | Anchor::RIGHT,
                Ancla::Centro => Anchor::empty(),
            } | if p.ancho == 0 { Anchor::LEFT | Anchor::RIGHT } else { Anchor::empty() });
            // La segunda en el mismo monitor, debajo de la primera: es para ensayar.
            let m = p.margen;
            capa.set_margin(m[0] + k as i32 * (alto as i32 + 12), m[1], m[2], m[3]);
            capa.set_size(p.ancho, alto);
            capa.set_exclusive_zone(p.reserva);
            // Si el teclado depende de algo (`exclusive while open`), se nace sin él.
            capa.set_keyboard_interactivity(interactividad(if p.teclado_mientras { Teclado::Nunca } else { p.teclado }));
            let id = self.siguiente_id;
            self.siguiente_id += 1;
            // Con ventanilla, el tamaño lógico es fijo y los píxeles de verdad los
            // decide la escala: así vale también una que no sea entera.
            // Ancho 0 es «todo el monitor»: cuánto es lo dirá el compositor al configurarla.
            let ventanilla = self.ventanillas.as_ref().map(|v| v.get_viewport(capa.wl_surface(), qh, Mudo));
            let escala = self.escalas.as_ref().map(|m| m.get_fractional_scale(capa.wl_surface(), qh, EscalaDe(id)));
            capa.commit();
            let superficie = unsafe {
                self.instancia
                    .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                        raw_display_handle: Some(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
                            NonNull::new(self.conexion.backend().display_ptr() as *mut _).unwrap(),
                        ))),
                        raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(
                            NonNull::new(capa.wl_surface().id().as_ptr() as *mut _).unwrap(),
                        )),
                    })
                    .expect("the graphics surface could not be created")
            };
            self.puestas.push(Puesta { id, cual, capa, salida: salida.clone(), ventanilla, _escala: escala, escala: 1.0, pendiente: Some((superficie, nombre.clone(), mhz)) });
            }
        }
    }

    fn quitar_de(&mut self, salida: &wl_output::WlOutput) {
        for p in self.puestas.iter().filter(|p| &p.salida == salida) {
            let _ = self.a_render.send(ARender::LaminaFuera(p.id));
        }
        self.puestas.retain(|p| &p.salida != salida);
    }
}

// ── emergentes ────────────────────────────────────────────────────

/// Lo que hace falta para abrir una emergente desde el hilo del render, que es
/// quien sabe cuándo toca. Los objetos de Wayland se pueden usar desde cualquier
/// hilo; lo que les pase después llega al de siempre, por `PopupHandler`.
struct Emergentes {
    qh: QueueHandle<Estado>,
    compositor: CompositorState,
    xdg: XdgShell,
    conexion: Connection,
    instancia: wgpu::Instance,
    ventanillas: Option<WpViewporter>,
    escalas: Option<WpFractionalScaleManagerV1>,
    cursores: Arc<Mutex<Option<WpCursorShapeDeviceV1>>>,
    serie: Arc<AtomicU32>,
    /// De quién cuelga la próxima: la superficie donde se vio el ratón por última vez.
    madre: Mutex<Option<Madre>>,
    asiento: Mutex<Option<wl_seat::WlSeat>>,
    /// La última pulsación: con ella se puede pedir que un clic fuera la cierre.
    pulsacion: Mutex<Option<(u32, std::time::Instant)>>,
    abiertas: Mutex<Vec<Abierta>>,
    siguiente_id: AtomicU32,
}

#[derive(Clone)]
struct Madre {
    capa: LayerSurface,
    escala: f32,
    nombre: String,
    mhz: i32,
}

struct Abierta {
    k: usize,
    id: u32,
    origen: (f32, f32),
    tam: (u32, u32),
    // Por orden: primero se suelta lo que pinta, luego la superficie.
    pendiente: Option<wgpu::Surface<'static>>,
    ventanilla: Option<WpViewport>,
    _escala: Option<WpFractionalScaleV1>,
    madre: Madre,
    popup: Popup,
}

static EMERGENTES: std::sync::OnceLock<Emergentes> = std::sync::OnceLock::new();

pub fn emergente(k: usize, que: Option<([i32; 4], (f32, f32))>) {
    let Some(e) = EMERGENTES.get() else { return };
    let Some(([x, y, w, h], origen)) = que else {
        e.abiertas.lock().unwrap().retain(|a| a.k != k);
        let _ = e.conexion.flush();
        return;
    };
    let Some(madre) = e.madre.lock().unwrap().clone() else { return };
    let abrir = || -> Option<Abierta> {
        let donde = XdgPositioner::new(&e.xdg).ok()?;
        donde.set_size(w, h);
        donde.set_anchor_rect(x, y, 1, 1);
        {
            use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_positioner::{Anchor, ConstraintAdjustment, Gravity};
            donde.set_anchor(Anchor::TopLeft);
            donde.set_gravity(Gravity::BottomRight);
            // Si no cabe en la pantalla, que se deslice hasta que quepa.
            donde.set_constraint_adjustment(ConstraintAdjustment::SlideX | ConstraintAdjustment::SlideY);
        }
        let wl = e.compositor.create_surface(&e.qh);
        let popup = Popup::from_surface(None, &donde, &e.qh, wl, &e.xdg).ok()?;
        madre.capa.get_popup(popup.xdg_popup());
        // Si se abre por un clic de hace un momento, que un clic fuera la cierre.
        if let (Some(asiento), Some((serie, cuando))) = (e.asiento.lock().unwrap().as_ref(), *e.pulsacion.lock().unwrap()) {
            if cuando.elapsed() < std::time::Duration::from_millis(1500) {
                popup.xdg_popup().grab(asiento, serie);
            }
        }
        let id = 1000 + e.siguiente_id.fetch_add(1, Ordering::Relaxed);
        let ventanilla = e.ventanillas.as_ref().map(|v| v.get_viewport(popup.wl_surface(), &e.qh, Mudo));
        let escala = e.escalas.as_ref().map(|m| m.get_fractional_scale(popup.wl_surface(), &e.qh, EscalaDe(id)));
        popup.wl_surface().commit();
        let superficie = unsafe {
            e.instancia
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(NonNull::new(e.conexion.backend().display_ptr() as *mut _).unwrap()))),
                    raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(NonNull::new(popup.wl_surface().id().as_ptr() as *mut _).unwrap())),
                })
                .ok()?
        };
        Some(Abierta { k, id, origen, tam: (w as u32, h as u32), pendiente: Some(superficie), ventanilla, _escala: escala, madre: madre.clone(), popup })
    };
    match abrir() {
        Some(a) => e.abiertas.lock().unwrap().push(a),
        None => eprintln!("popup  · the compositor did not let it open"),
    }
    let _ = e.conexion.flush();
}

/// Ventanas normales no abrimos todavía (es lo que queda de S7), pero el
/// protocolo de las emergentes viene en el mismo paquete y pide que esto exista.
impl smithay_client_toolkit::shell::xdg::window::WindowHandler for Estado {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &smithay_client_toolkit::shell::xdg::window::Window) {}
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &smithay_client_toolkit::shell::xdg::window::Window, _: smithay_client_toolkit::shell::xdg::window::WindowConfigure, _: u32) {}
}

impl PopupHandler for Estado {
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, popup: &Popup, _: PopupConfigure) {
        let Some(e) = EMERGENTES.get() else { return };
        let mut abiertas = e.abiertas.lock().unwrap();
        let Some(a) = abiertas.iter_mut().find(|a| a.popup.wl_surface() == popup.wl_surface()) else { return };
        if let Some(v) = &a.ventanilla {
            v.set_destination(a.tam.0 as i32, a.tam.1 as i32);
        }
        if let Some(superficie) = a.pendiente.take() {
            let _ = self.a_render.send(ARender::Lamina(Box::new(gpu::NuevaLamina {
                id: a.id,
                superficie,
                ventana: Box::new(VentanaWayland { wl: a.popup.wl_surface().clone(), compositor: self.compositor.clone(), cursores: e.cursores.clone(), serie: e.serie.clone(), capa: None }),
                escala: a.madre.escala,
                tam: a.tam,
                mhz: a.madre.mhz,
                nombre: format!("{} (emergente)", a.madre.nombre),
                vista: gpu::Vista { superficie: 0, emergente: Some(a.k), origen: a.origen, tam: (a.tam.0 as f32, a.tam.1 as f32) },
            })));
        }
    }

    /// Han pulsado fuera, o el compositor la ha quitado. El render suelta lo suyo y la cierra.
    fn done(&mut self, _: &Connection, _: &QueueHandle<Self>, popup: &Popup) {
        let Some(e) = EMERGENTES.get() else { return };
        if let Some(a) = e.abiertas.lock().unwrap().iter().find(|a| a.popup.wl_surface() == popup.wl_surface()) {
            let _ = self.a_render.send(ARender::EmergenteCerrada(a.k));
        }
    }
}

/// Pone las superficies que pida la escena y atiende a Wayland hasta que
/// alguien cierre. Se queda con el hilo que la llama.
pub fn atender(pide: Vec<Superficie>, alto_extra: u32, instancia: wgpu::Instance, a_render: Sender<ARender>) {
    let conexion = Connection::connect_to_env().expect("there is no Wayland session");
    let (globales, mut eventos) = registry_queue_init::<Estado>(&conexion).unwrap();
    let qh = eventos.handle();
    let compositor = CompositorState::bind(&globales, &qh).expect("sin wl_compositor");
    let mut estado = Estado {
        registro: RegistryState::new(&globales),
        asientos: SeatState::new(&globales, &qh),
        salidas: OutputState::new(&globales, &qh),
        capas: LayerShell::bind(&globales, &qh).expect("the compositor has no layer-shell"),
        ventanillas: globales.bind(&qh, 1..=1, Mudo).ok(),
        escalas: globales.bind(&qh, 1..=1, Mudo).ok(),
        compositor,
        conexion: conexion.clone(),
        instancia,
        pide,
        alto_extra,
        puestas: Vec::new(),
        siguiente_id: 0,
        puntero: None,
        teclado: None,
        formas_de_cursor: CursorShapeManager::bind(&globales, &qh).ok(),
        cursores: Arc::default(),
        serie: Arc::default(),
        mods: Mods::default(),
        arrastres: DataDeviceManagerState::bind(&globales, &qh).ok(),
        dispositivo_de_datos: None,
        salir: false,
        a_render,
    };
    match XdgShell::bind(&globales, &qh) {
        Ok(xdg) => {
            let _ = EMERGENTES.set(Emergentes {
                qh: qh.clone(),
                compositor: estado.compositor.clone(),
                xdg,
                conexion: conexion.clone(),
                instancia: estado.instancia.clone(),
                ventanillas: estado.ventanillas.clone(),
                escalas: estado.escalas.clone(),
                cursores: estado.cursores.clone(),
                serie: estado.serie.clone(),
                madre: Mutex::default(),
                asiento: Mutex::default(),
                pulsacion: Mutex::default(),
                abiertas: Mutex::default(),
                siguiente_id: AtomicU32::new(0),
            });
        }
        Err(_) => eprintln!("warning: the compositor has no xdg-shell; there will be no popup surfaces"),
    }
    if estado.ventanillas.is_none() || estado.escalas.is_none() {
        eprintln!("warning: the compositor gives no fractional scale; it will paint at whatever whole scale it says");
    }
    // Los monitores que ya están llegan con las primeras vueltas; los que se
    // enchufen después, por `new_output`.
    eventos.roundtrip(&mut estado).unwrap();
    eventos.roundtrip(&mut estado).unwrap();
    for salida in estado.salidas.outputs().collect::<Vec<_>>() {
        estado.poner_en(&salida, &qh);
    }
    if estado.puestas.is_empty() {
        eprintln!("warning: none of the monitors asked for ({:?}) is plugged in; waiting for one to appear", estado.pide.iter().map(|s| &s.pantallas).collect::<Vec<_>>());
    }
    while !estado.salir {
        eventos.blocking_dispatch(&mut estado).unwrap();
    }
}

impl LayerShellHandler for Estado {
    /// El compositor la ha cerrado: casi siempre, porque su monitor se ha ido.
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, capa: &LayerSurface) {
        if let Some(p) = self.puestas.iter().find(|p| &p.capa == capa) {
            let _ = self.a_render.send(ARender::LaminaFuera(p.id));
        }
        self.puestas.retain(|p| &p.capa != capa);
    }
    /// Hasta que el compositor no la configura no se le puede pegar nada:
    /// es ahora cuando pasa a manos del render.
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, capa: &LayerSurface, conf: LayerSurfaceConfigure, _: u32) {
        let Some(p) = self.puestas.iter_mut().find(|p| &p.capa == capa) else { return };
        let suya = &self.pide[p.cual];
        let pedido = (suya.ancho, suya.alto + if p.cual == 0 { self.alto_extra } else { 0 });
        let origen = suya.origen;
        // Lo que el compositor haya dado; si dice 0, lo que se pidió.
        let tam = (if conf.new_size.0 > 0 { conf.new_size.0 } else { pedido.0 }, if conf.new_size.1 > 0 { conf.new_size.1 } else { pedido.1 });
        if let Some(v) = &p.ventanilla {
            v.set_destination(tam.0 as i32, tam.1 as i32);
        }
        if let Some((superficie, nombre, mhz)) = p.pendiente.take() {
            if let Some(em) = EMERGENTES.get() {
                // Mientras no se vea el ratón en ninguna, las emergentes cuelgan de la primera.
                em.madre.lock().unwrap().get_or_insert_with(|| Madre { capa: p.capa.clone(), escala: p.escala, nombre: nombre.clone(), mhz });
            }
            let _ = self.a_render.send(ARender::Lamina(Box::new(gpu::NuevaLamina {
                id: p.id,
                superficie,
                ventana: Box::new(VentanaWayland { wl: p.capa.wl_surface().clone(), compositor: self.compositor.clone(), cursores: self.cursores.clone(), serie: self.serie.clone(), capa: Some(p.capa.clone()) }),
                escala: p.escala,
                tam,
                mhz,
                nombre,
                vista: gpu::Vista { superficie: p.cual, emergente: None, origen, tam: (tam.0 as f32, tam.1 as f32) },
            })));
        }
    }
}

impl PointerHandler for Estado {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, eventos: &[PointerEvent]) {
        for e in eventos {
            let (mut x, mut y) = (e.position.0 as f32, e.position.1 as f32);
            if let Some(em) = EMERGENTES.get() {
                // Dentro de una emergente, el ratón está en el trozo de escena que ella enseña.
                if let Some(a) = em.abiertas.lock().unwrap().iter().find(|a| a.popup.wl_surface() == &e.surface) {
                    (x, y) = (x + a.origen.0, y + a.origen.1);
                } else if let Some(p) = self.puestas.iter().find(|p| p.capa.wl_surface() == &e.surface) {
                    let info = self.salidas.info(&p.salida);
                    // El ratón sobre una superficie está en el trozo del plano que ella enseña.
                    let origen = self.pide[p.cual].origen;
                    (x, y) = (x + origen.0, y + origen.1);
                    *em.madre.lock().unwrap() = Some(Madre {
                        capa: p.capa.clone(),
                        escala: p.escala,
                        nombre: info.as_ref().and_then(|i| i.name.clone()).unwrap_or_default(),
                        mhz: info.as_ref().and_then(|i| i.modes.iter().find(|m| m.current).map(|m| m.refresh_rate)).unwrap_or(0),
                    });
                }
                if let PointerEventKind::Press { serial, .. } = e.kind {
                    *em.pulsacion.lock().unwrap() = Some((serial, std::time::Instant::now()));
                }
            }
            match e.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    if let PointerEventKind::Enter { serial } = e.kind {
                        self.serie.store(serial, Ordering::Relaxed);
                    }
                    // El ratón va al render, que es quien sabe qué hay debajo;
                    // a la lógica le llega ya con nombre.
                    let _ = self.a_render.send(ARender::Puntero(Some((x, y))));
                }
                PointerEventKind::Leave { .. } => {
                    let _ = self.a_render.send(ARender::Puntero(None));
                }
                PointerEventKind::Press { button, .. } | PointerEventKind::Release { button, .. } => {
                    let abajo = matches!(e.kind, PointerEventKind::Press { .. });
                    // BTN_LEFT, BTN_RIGHT, BTN_MIDDLE
                    let boton = match button {
                        0x110 => 0,
                        0x111 => 1,
                        0x112 => 2,
                        _ => continue,
                    };
                    // Mientras una escena no le dé uso al botón derecho, cierra: es la
                    // salida de emergencia de un prototipo sin teclado.
                    if boton == 1 && abajo && self.pide.iter().all(|s| s.derecho_cierra) {
                        self.salir = true;
                        continue;
                    }
                    let _ = self.a_render.send(ARender::Puntero(Some((x, y))));
                    let _ = self.a_render.send(ARender::Boton(boton, abajo));
                }
                PointerEventKind::Axis { vertical, horizontal, .. } => {
                    // Muescas de rueda, positivo hacia arriba. Una rueda de verdad manda
                    // 120 por muesca; un panel táctil, píxeles.
                    let muescas = |a: &smithay_client_toolkit::seat::pointer::AxisScroll| {
                        if a.value120 != 0 { a.value120 as f32 / 120.0 } else if a.discrete != 0 { a.discrete as f32 } else { a.absolute as f32 / 15.0 }
                    };
                    let d = -(muescas(&vertical) + muescas(&horizontal));
                    if d != 0.0 {
                        let _ = self.a_render.send(ARender::Rueda(d));
                    }
                }
            }
        }
    }
}

/// Lo que sabemos recibir, por orden de preferencia: ficheros, y si no, texto.
const TIPOS: [&str; 3] = ["text/uri-list", "text/plain;charset=utf-8", "text/plain"];

fn oferta(d: &wayland_client::protocol::wl_data_device::WlDataDevice) -> Option<DragOffer> {
    d.data::<DataDeviceData>()?.drag_offer()
}

/// Arrastrar desde otra aplicación. Mientras dura, el compositor no manda el
/// ratón por el camino de siempre: llega por aquí, y se reenvía igual.
impl DataDeviceHandler for Estado {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, d: &wayland_client::protocol::wl_data_device::WlDataDevice, x: f64, y: f64, _: &wl_surface::WlSurface) {
        if let Some(o) = oferta(d) {
            let tipo = o.with_mime_types(|t| TIPOS.iter().find(|q| t.iter().any(|x| x == *q)).map(|q| q.to_string()));
            o.accept_mime_type(o.serial, tipo);
            o.set_actions(DndAction::Copy, DndAction::Copy);
        }
        let _ = self.a_render.send(ARender::Puntero(Some((x as f32, y as f32))));
    }
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_data_device::WlDataDevice) {
        let _ = self.a_render.send(ARender::Puntero(None));
    }
    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_data_device::WlDataDevice, x: f64, y: f64) {
        let _ = self.a_render.send(ARender::Puntero(Some((x as f32, y as f32))));
    }
    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_data_device::WlDataDevice) {}
    fn drop_performed(&mut self, conn: &Connection, _: &QueueHandle<Self>, d: &wayland_client::protocol::wl_data_device::WlDataDevice) {
        let Some(o) = oferta(d) else { return };
        let Some(tipo) = o.with_mime_types(|t| TIPOS.iter().find(|q| t.iter().any(|x| x == *q)).map(|q| q.to_string())) else { return };
        let Ok(mut tubo) = o.receive(tipo.clone()) else { return };
        let _ = conn.flush();
        // Leer el tubo puede tardar lo que tarde quien escribe: en otro hilo.
        let tx = self.a_render.clone();
        std::thread::spawn(move || {
            use std::io::Read;
            let mut datos = String::new();
            let _ = tubo.read_to_string(&mut datos);
            o.finish();
            o.destroy();
            let _ = tx.send(ARender::Soltado(tipo, datos.trim_end().to_owned()));
        });
    }
}

impl DataOfferHandler for Estado {
    fn source_actions(&mut self, _: &Connection, _: &QueueHandle<Self>, o: &mut DragOffer, _: DndAction) {
        o.set_actions(DndAction::Copy, DndAction::Copy);
    }
    fn selected_action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &mut DragOffer, _: DndAction) {}
}

/// No se arrastra nada HACIA fuera todavía; el rasgo hay que cumplirlo igual.
impl DataSourceHandler for Estado {
    fn accept_mime(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_data_source::WlDataSource, _: Option<String>) {}
    fn send_request(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_data_source::WlDataSource, _: String, _: WritePipe) {}
    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_data_source::WlDataSource) {}
    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_data_source::WlDataSource) {}
    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_data_source::WlDataSource) {}
    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_data_source::WlDataSource, _: DndAction) {}
}

fn nombre_de(e: &KeyEvent) -> String {
    // Como la llama xkb: `Escape`, `Return`, `BackSpace`, `a`.
    e.keysym.name().map(|n| n.trim_start_matches("XK_").to_owned()).unwrap_or_else(|| format!("{:#x}", e.keysym.raw()))
}

impl KeyboardHandler for Estado {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32, _: &[u32], _: &[Keysym]) {
        let _ = self.a_render.send(ARender::FocoTeclado(true));
    }
    /// Perder el teclado es, casi siempre, que han pulsado en otro sitio.
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {
        let _ = self.a_render.send(ARender::FocoTeclado(false));
    }
    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_keyboard::WlKeyboard, _: u32, e: KeyEvent) {
        let escribe = e.utf8.clone().filter(|t| !t.chars().any(char::is_control));
        let _ = self.a_render.send(ARender::Tecla(nombre_de(&e), escribe, self.mods));
    }
    fn update_repeat_info(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_keyboard::WlKeyboard, info: RepeatInfo) {
        let r = match info {
            RepeatInfo::Repeat { rate, delay } => Some((delay, (1000 / rate.get()).max(1))),
            RepeatInfo::Disable => None,
        };
        match r {
            Some((espera, cada)) => println!("keyboard · repeats after {espera} ms, then every {cada} ms"),
            None => println!("keyboard · no repeat"),
        }
        let _ = self.a_render.send(ARender::Repeticion(r));
    }
    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}
    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_keyboard::WlKeyboard, _: u32, e: KeyEvent) {
        let _ = self.a_render.send(ARender::TeclaSuelta(nombre_de(&e)));
    }
    fn update_modifiers(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wayland_client::protocol::wl_keyboard::WlKeyboard, _: u32, m: Modifiers, _: RawModifiers, _: u32) {
        self.mods = Mods { ctrl: m.ctrl, alt: m.alt, mayus: m.shift, logo: m.logo };
    }
}

impl SeatHandler for Estado {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.asientos
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, asiento: wl_seat::WlSeat, c: Capability) {
        if let Some(em) = EMERGENTES.get() {
            em.asiento.lock().unwrap().get_or_insert_with(|| asiento.clone());
        }
        if c == Capability::Pointer && self.puntero.is_none() {
            self.puntero = self.asientos.get_pointer(qh, &asiento).ok();
            if let (Some(p), Some(m)) = (&self.puntero, &self.formas_de_cursor) {
                *self.cursores.lock().unwrap() = Some(m.get_shape_device(p, qh));
            }
        }
        if self.dispositivo_de_datos.is_none() {
            self.dispositivo_de_datos = self.arrastres.as_ref().map(|m| m.get_data_device(qh, &asiento));
        }
        if c == Capability::Keyboard && self.teclado.is_none() && self.pide.iter().any(|s| s.teclado != Teclado::Nunca) {
            self.teclado = self.asientos.get_keyboard(qh, &asiento, None).ok();
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, _: Capability) {}
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl CompositorHandler for Estado {
    /// La escala entera de toda la vida. Solo cuenta si el compositor no da la
    /// fraccional, que llega por su propio camino.
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, wl: &wl_surface::WlSurface, escala: i32) {
        if self.escalas.is_some() && self.ventanillas.is_some() {
            return;
        }
        if let Some(p) = self.puestas.iter_mut().find(|p| p.capa.wl_surface() == wl) {
            wl.set_buffer_scale(escala);
            p.escala = escala as f32;
            let _ = self.a_render.send(ARender::Escala(p.id, escala as f32));
        }
    }
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for Estado {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.salidas
    }
    /// Un monitor que se enchufa con el programa en marcha.
    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, salida: wl_output::WlOutput) {
        self.poner_en(&salida, qh);
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, salida: wl_output::WlOutput) {
        self.quitar_de(&salida);
    }
}

delegate_registry!(Estado);
impl ProvidesRegistryState for Estado {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registro
    }
    registry_handlers![OutputState, SeatState];
}

smithay_client_toolkit::delegate_dispatch2!(Estado);

/// Un icono por su nombre, a la manera de freedesktop pero sin leer los
/// `index.theme`: se prueba en los temas de siempre, de lo vectorial a lo
/// pequeño. Basta para iconos de aplicaciones.
pub fn icono(nombre: &str) -> Option<std::path::PathBuf> {
    let casa = std::env::var("HOME").unwrap_or_default();
    let bases = [format!("{casa}/.local/share/icons"), format!("{casa}/.icons"), "/usr/share/icons".into()];
    let temas = ["hicolor", "Papirus", "Papirus-Dark", "Adwaita", "breeze", "breeze-dark"];
    let tallas = ["scalable", "512x512", "256x256", "128x128", "96x96", "64x64", "48x48", "32x32", "symbolic"];
    let clases = ["apps", "devices", "places", "status", "actions", "categories", "mimetypes"];
    for base in &bases {
        for tema in temas {
            for talla in tallas {
                for clase in clases {
                    for ext in ["svg", "png"] {
                        // Hay temas que ordenan por talla/clase y otros por clase/talla.
                        for ruta in [format!("{base}/{tema}/{talla}/{clase}/{nombre}.{ext}"), format!("{base}/{tema}/{clase}/{talla}/{nombre}.{ext}")] {
                            if std::path::Path::new(&ruta).is_file() {
                                return Some(ruta.into());
                            }
                        }
                    }
                }
            }
        }
    }
    ["svg", "png"].iter().map(|e| std::path::PathBuf::from(format!("/usr/share/pixmaps/{nombre}.{e}"))).find(|p| p.is_file())
}
