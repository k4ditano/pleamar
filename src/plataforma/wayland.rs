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
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{cursor_shape::CursorShapeManager, PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
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
    capa: LayerSurface,
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
        self.capa.set_keyboard_interactivity(interactividad(t));
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
    pide: Superficie,
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
    fn quiere(&self, nombre: &str) -> usize {
        match &self.pide.pantallas {
            Pantallas::Todas => 1,
            Pantallas::Estas(n) => n.iter().filter(|x| x.as_str() == nombre).count(),
        }
    }

    /// Pone en un monitor las superficies que la escena pida para él.
    fn poner_en(&mut self, salida: &wl_output::WlOutput, qh: &QueueHandle<Estado>) {
        let Some(info) = self.salidas.info(salida) else { return };
        let nombre = info.name.clone().unwrap_or_default();
        let mhz = info.modes.iter().find(|m| m.current).map_or(0, |m| m.refresh_rate);
        let ya = self.puestas.iter().filter(|p| &p.salida == salida).count();
        for k in ya..self.quiere(&nombre) {
            let p = &self.pide;
            let alto = p.alto + self.alto_extra;
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
                    .expect("no se pudo crear la superficie gráfica")
            };
            self.puestas.push(Puesta { id, capa, salida: salida.clone(), ventanilla, _escala: escala, escala: 1.0, pendiente: Some((superficie, nombre.clone(), mhz)) });
        }
    }

    fn quitar_de(&mut self, salida: &wl_output::WlOutput) {
        for p in self.puestas.iter().filter(|p| &p.salida == salida) {
            let _ = self.a_render.send(ARender::LaminaFuera(p.id));
        }
        self.puestas.retain(|p| &p.salida != salida);
    }
}

/// Pone las superficies que pida la escena y atiende a Wayland hasta que
/// alguien cierre. Se queda con el hilo que la llama.
pub fn atender(pide: Superficie, alto_extra: u32, instancia: wgpu::Instance, a_render: Sender<ARender>) {
    let conexion = Connection::connect_to_env().expect("no hay sesión Wayland");
    let (globales, mut eventos) = registry_queue_init::<Estado>(&conexion).unwrap();
    let qh = eventos.handle();
    let compositor = CompositorState::bind(&globales, &qh).expect("sin wl_compositor");
    let mut estado = Estado {
        registro: RegistryState::new(&globales),
        asientos: SeatState::new(&globales, &qh),
        salidas: OutputState::new(&globales, &qh),
        capas: LayerShell::bind(&globales, &qh).expect("el compositor no tiene layer-shell"),
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
    if estado.ventanillas.is_none() || estado.escalas.is_none() {
        eprintln!("aviso: el compositor no da escala fraccional; se pintará a la escala entera que diga");
    }
    // Los monitores que ya están llegan con las primeras vueltas; los que se
    // enchufen después, por `new_output`.
    eventos.roundtrip(&mut estado).unwrap();
    eventos.roundtrip(&mut estado).unwrap();
    for salida in estado.salidas.outputs().collect::<Vec<_>>() {
        estado.poner_en(&salida, &qh);
    }
    if estado.puestas.is_empty() {
        eprintln!("aviso: ningún monitor de los pedidos ({:?}) está enchufado; espero a que aparezca", estado.pide.pantallas);
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
        let pedido = (self.pide.ancho, self.pide.alto + self.alto_extra);
        let Some(p) = self.puestas.iter_mut().find(|p| &p.capa == capa) else { return };
        // Lo que el compositor haya dado; si dice 0, lo que se pidió.
        let tam = (if conf.new_size.0 > 0 { conf.new_size.0 } else { pedido.0 }, if conf.new_size.1 > 0 { conf.new_size.1 } else { pedido.1 });
        if let Some(v) = &p.ventanilla {
            v.set_destination(tam.0 as i32, tam.1 as i32);
        }
        if let Some((superficie, nombre, mhz)) = p.pendiente.take() {
            let _ = self.a_render.send(ARender::Lamina(Box::new(gpu::NuevaLamina {
                id: p.id,
                superficie,
                ventana: Box::new(VentanaWayland { wl: p.capa.wl_surface().clone(), compositor: self.compositor.clone(), cursores: self.cursores.clone(), serie: self.serie.clone(), capa: p.capa.clone() }),
                escala: p.escala,
                tam,
                mhz,
                nombre,
            })));
        }
    }
}

impl PointerHandler for Estado {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, eventos: &[PointerEvent]) {
        for e in eventos {
            let (x, y) = (e.position.0 as f32, e.position.1 as f32);
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
                    if boton == 1 && abajo && self.pide.derecho_cierra {
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
        if c == Capability::Pointer && self.puntero.is_none() {
            self.puntero = self.asientos.get_pointer(qh, &asiento).ok();
            if let (Some(p), Some(m)) = (&self.puntero, &self.formas_de_cursor) {
                *self.cursores.lock().unwrap() = Some(m.get_shape_device(p, qh));
            }
        }
        if self.dispositivo_de_datos.is_none() {
            self.dispositivo_de_datos = self.arrastres.as_ref().map(|m| m.get_data_device(qh, &asiento));
        }
        if c == Capability::Keyboard && self.teclado.is_none() && self.pide.teclado != Teclado::Nunca {
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
