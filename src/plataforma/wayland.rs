//! Wayland: una superficie layer-shell por monitor, su escala —también la
//! fraccional— y el ratón. Es lo único del programa que sabe qué es Wayland.

use super::Ventana;
use crate::escena::*;
use crate::gpu;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
        WaylandSurface,
    },
};
use std::ptr::NonNull;
use std::sync::mpsc::Sender;
use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::reexports::protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_seat, wl_surface},
    Connection, Proxy, QueueHandle,
};


/// Lo que el render necesita de una superficie de Wayland, y nada más.
struct VentanaWayland {
    wl: wl_surface::WlSurface,
    compositor: CompositorState,
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
}

/// Una superficie puesta en un monitor.
struct Puesta {
    id: u32,
    capa: LayerSurface,
    salida: wl_output::WlOutput,
    /// Para pintar a una escala que no sea entera.
    _ventanilla: Option<WpViewport>,
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
            });
            // La segunda en el mismo monitor, debajo de la primera: es para ensayar.
            let m = p.margen;
            capa.set_margin(m[0] + k as i32 * (alto as i32 + 12), m[1], m[2], m[3]);
            capa.set_size(p.ancho, alto);
            capa.set_exclusive_zone(p.reserva);
            capa.set_keyboard_interactivity(KeyboardInteractivity::None);
            let id = self.siguiente_id;
            self.siguiente_id += 1;
            // Con ventanilla, el tamaño lógico es fijo y los píxeles de verdad los
            // decide la escala: así vale también una que no sea entera.
            let ventanilla = self.ventanillas.as_ref().map(|v| {
                let w = v.get_viewport(capa.wl_surface(), qh, Mudo);
                w.set_destination(p.ancho as i32, alto as i32);
                w
            });
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
            self.puestas.push(Puesta { id, capa, salida: salida.clone(), _ventanilla: ventanilla, _escala: escala, escala: 1.0, pendiente: Some((superficie, nombre.clone(), mhz)) });
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
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, capa: &LayerSurface, _: LayerSurfaceConfigure, _: u32) {
        let Some(p) = self.puestas.iter_mut().find(|p| &p.capa == capa) else { return };
        if let Some((superficie, nombre, mhz)) = p.pendiente.take() {
            let _ = self.a_render.send(ARender::Lamina(Box::new(gpu::NuevaLamina {
                id: p.id,
                superficie,
                ventana: Box::new(VentanaWayland { wl: p.capa.wl_surface().clone(), compositor: self.compositor.clone() }),
                escala: p.escala,
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
                    // El ratón va al render, que es quien sabe qué hay debajo;
                    // a la lógica le llega ya con nombre.
                    let _ = self.a_render.send(ARender::Puntero(Some((x, y))));
                }
                PointerEventKind::Leave { .. } => {
                    let _ = self.a_render.send(ARender::Puntero(None));
                }
                PointerEventKind::Press { button, .. } => {
                    if button == 0x111 {
                        self.salir = true;
                    } else {
                        let _ = self.a_render.send(ARender::Puntero(Some((x, y))));
                        let _ = self.a_render.send(ARender::Pulsar);
                    }
                }
                _ => {}
            }
        }
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
