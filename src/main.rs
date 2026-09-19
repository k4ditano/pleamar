//! pleamar — prototipo de una shell donde animar no depende de la lógica.
//!
//! Tres hilos: este, que solo habla Wayland (una superficie por monitor, su
//! escala y el ratón); el de lógica, que decide; y el de render, que es el
//! único que anima.

mod escena;
mod escenas;
mod formas;
mod gpu;
mod logica;
mod render;
mod texto;

use escena::*;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
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
use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::reexports::protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::time::Duration;
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_seat, wl_surface},
    Connection, Proxy, QueueHandle,
};

const AYUDA: &str = "pleamar [opciones]
  --escena NOMBRE     marea (por defecto), isla, cara, muestrario o enjambre (PLEAMAR_N formas)
  --pantalla NOMBRES  «todas», o monitores separados por comas (por defecto, lo que pida la escena).
                      Un nombre repetido da dos superficies en el mismo monitor.
  --bloqueo MS        lo que se bloquea la lógica tras cada decisión (600)
  --ingenuo           la lógica bloquea el hilo que pinta, como en QtQuick
  --demo              abre y cierra sola, sin ratón
  --raton GUION       ratón de mentira: «360,90@1000 pulsa@2500 fuera@4000» (ms)
  --segundos N        salir sola al cabo de N segundos
  --margen PX         margen superior, en vez del de la escena
  --sin-hud           sin la gráfica de frames
  --sin-vsync         pintar sin esperar a la pantalla, para medir lo que cuesta un frame
  --movimiento-reducido  los muelles se posan y los gestos enseñan su cara quieta
Botón derecho sobre ella para cerrarla.";

struct Args {
    escena: String,
    pantalla: Option<String>,
    bloqueo: u64,
    ingenuo: bool,
    demo: bool,
    raton: Option<String>,
    segundos: Option<u64>,
    margen: Option<i32>,
    hud: bool,
    reducido: bool,
    sin_vsync: bool,
}

fn args() -> Args {
    let mut a = Args { escena: "marea".into(), pantalla: None, bloqueo: 600, ingenuo: false, demo: false, raton: None, segundos: None, margen: None, hud: true, reducido: false, sin_vsync: false };
    let mut it = std::env::args().skip(1);
    while let Some(op) = it.next() {
        let mut valor = || it.next().unwrap_or_else(|| { eprintln!("{AYUDA}"); std::process::exit(2) });
        match op.as_str() {
            "--escena" => a.escena = valor(),
            "--pantalla" => a.pantalla = Some(valor()),
            "--bloqueo" => a.bloqueo = valor().parse().expect("--bloqueo quiere milisegundos"),
            "--raton" => a.raton = Some(valor()),
            "--segundos" => a.segundos = Some(valor().parse().expect("--segundos quiere un número")),
            "--margen" => a.margen = Some(valor().parse().expect("--margen quiere píxeles")),
            "--ingenuo" => a.ingenuo = true,
            "--demo" => a.demo = true,
            "--sin-hud" => a.hud = false,
            "--movimiento-reducido" => a.reducido = true,
            "--sin-vsync" => a.sin_vsync = true,
            _ => { eprintln!("{AYUDA}"); std::process::exit(2) }
        }
    }
    a
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
    hud: bool,
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
            let alto = p.alto + if self.hud { gpu::ALTO_INSTRUMENTOS as u32 } else { 0 };
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

fn main() {
    let a = args();
    let bloqueada = Arc::new(AtomicBool::new(false));
    let mut guion: Box<dyn logica::Guion> = match a.escena.as_str() {
        "marea" => Box::<escenas::marea::Marea>::default(),
        "isla" => Box::<escenas::isla::Isla>::default(),
        "cara" => Box::<escenas::cara::Cara>::default(),
        "muestrario" => Box::<escenas::muestrario::Muestrario>::default(),
        "enjambre" => Box::<escenas::enjambre::Enjambre>::default(),
        otra => {
            eprintln!("no conozco la escena «{otra}»\n{AYUDA}");
            std::process::exit(2)
        }
    };
    // La escena dice qué superficie quiere; la línea de órdenes puede llevarle
    // la contraria.
    let mut escena = guion.escena();
    if let Some(p) = &a.pantalla {
        escena.superficie.pantallas = if p == "todas" { Pantallas::Todas } else { Pantallas::Estas(p.split(',').map(str::to_owned).collect()) };
    }
    if let Some(m) = a.margen {
        escena.superficie.margen[0] = m;
    }

    let conexion = Connection::connect_to_env().expect("no hay sesión Wayland");
    let (globales, mut eventos) = registry_queue_init::<Estado>(&conexion).unwrap();
    let qh = eventos.handle();
    let compositor = CompositorState::bind(&globales, &qh).expect("sin wl_compositor");
    let (a_render, de_render) = channel();
    let (a_logica, de_logica) = channel();
    let mut estado = Estado {
        registro: RegistryState::new(&globales),
        asientos: SeatState::new(&globales, &qh),
        salidas: OutputState::new(&globales, &qh),
        capas: LayerShell::bind(&globales, &qh).expect("el compositor no tiene layer-shell"),
        ventanillas: globales.bind(&qh, 1..=1, Mudo).ok(),
        escalas: globales.bind(&qh, 1..=1, Mudo).ok(),
        compositor: compositor.clone(),
        conexion: conexion.clone(),
        instancia: wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::VULKAN, ..wgpu::InstanceDescriptor::new_without_display_handle() }),
        pide: escena.superficie.clone(),
        hud: a.hud,
        puestas: Vec::new(),
        siguiente_id: 0,
        puntero: None,
        salir: false,
        a_render: a_render.clone(),
    };
    if estado.ventanillas.is_none() || estado.escalas.is_none() {
        eprintln!("aviso: el compositor no da escala fraccional; se pintará a la escala entera que diga");
    }

    println!(
        "pleamar · modo {} · la lógica se bloquea {} ms tras cada decisión",
        if a.ingenuo { "INGENUO (un solo hilo)" } else { "separado (render independiente)" },
        a.bloqueo
    );
    let _ = a_render.send(ARender::Escena(escena));
    let render = {
        let bloqueada = bloqueada.clone();
        let op = render::Opciones { hud: a.hud, ingenuo: a.ingenuo, reducido: a.reducido, sin_vsync: a.sin_vsync };
        let instancia = estado.instancia.clone();
        std::thread::Builder::new()
            .name("render".into())
            .spawn(move || render::hilo(instancia, compositor, de_render, a_logica, bloqueada, op))
            .unwrap()
    };
    {
        let op = logica::Opciones { bloqueo: Duration::from_millis(a.bloqueo), demo: a.demo, ingenuo: a.ingenuo, eco: a.raton.is_some() };
        let tx = a_render.clone();
        std::thread::Builder::new()
            .name("logica".into())
            .spawn(move || logica::hilo(guion, de_logica, tx, bloqueada, op))
            .unwrap();
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
    if let Some(guion) = a.raton.clone() {
        // Para ensayar las zonas sin quitarle el ratón a nadie.
        let tx = a_render.clone();
        std::thread::spawn(move || {
            let inicio = std::time::Instant::now();
            for paso in guion.split_whitespace() {
                let (que, cuando) = paso.split_once('@').expect("--raton: falta @ms");
                let cuando = Duration::from_millis(cuando.parse().expect("--raton: ms"));
                std::thread::sleep(cuando.saturating_sub(inicio.elapsed()));
                let _ = match que {
                    "pulsa" => tx.send(ARender::Pulsar),
                    "fuera" => tx.send(ARender::Puntero(None)),
                    xy => {
                        let (x, y) = xy.split_once(',').expect("--raton: x,y");
                        tx.send(ARender::Puntero(Some((x.parse().unwrap(), y.parse().unwrap()))))
                    }
                };
            }
        });
    }
    if let Some(s) = a.segundos {
        let tx = a_render.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(s));
            // Salir pasa por el render para que cierre su último ciclo de medidas.
            let _ = tx.send(ARender::Salir);
            std::thread::sleep(Duration::from_millis(400));
            std::process::exit(0);
        });
    }

    while !estado.salir && !render.is_finished() {
        eventos.blocking_dispatch(&mut estado).unwrap();
    }
    let _ = a_render.send(ARender::Salir);
    let _ = render.join();
    // El proceso se va entero: el orden de destrucción entre Vulkan y Wayland
    // no merece código en un prototipo.
    std::process::exit(0);
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
                wl: p.capa.wl_surface().clone(),
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
