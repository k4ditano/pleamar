//! pleamar — prototipo de una shell donde animar no depende de la lógica.
//!
//! Tres hilos: este, que solo habla Wayland (superficie layer-shell y ratón);
//! el de lógica, que decide; y el de render, que es el único que anima.

mod escena;
mod escenas;
mod logica;
mod render;
mod texto;

use escena::*;
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
  --escena NOMBRE     marea (por defecto) o isla
  --pantalla NOMBRE   monitor donde aparecer (por defecto HDMI-A-1)
  --bloqueo MS        lo que se bloquea la lógica tras cada decisión (600)
  --ingenuo           la lógica bloquea el hilo que pinta, como en QtQuick
  --demo              abre y cierra sola, sin ratón
  --raton GUION       ratón de mentira: «360,90@1000 pulsa@2500 fuera@4000» (ms)
  --segundos N        salir sola al cabo de N segundos
  --margen PX         margen superior (40)
  --sin-hud           sin la gráfica de frames
Botón derecho sobre ella para cerrarla.";

struct Args {
    escena: String,
    pantalla: String,
    bloqueo: u64,
    ingenuo: bool,
    demo: bool,
    raton: Option<String>,
    segundos: Option<u64>,
    margen: i32,
    hud: bool,
}

fn args() -> Args {
    let mut a = Args { escena: "marea".into(), pantalla: "HDMI-A-1".into(), bloqueo: 600, ingenuo: false, demo: false, raton: None, segundos: None, margen: 40, hud: true };
    let mut it = std::env::args().skip(1);
    while let Some(op) = it.next() {
        let mut valor = || it.next().unwrap_or_else(|| { eprintln!("{AYUDA}"); std::process::exit(2) });
        match op.as_str() {
            "--escena" => a.escena = valor(),
            "--pantalla" => a.pantalla = valor(),
            "--bloqueo" => a.bloqueo = valor().parse().expect("--bloqueo quiere milisegundos"),
            "--raton" => a.raton = Some(valor()),
            "--segundos" => a.segundos = Some(valor().parse().expect("--segundos quiere un número")),
            "--margen" => a.margen = valor().parse().expect("--margen quiere píxeles"),
            "--ingenuo" => a.ingenuo = true,
            "--demo" => a.demo = true,
            "--sin-hud" => a.hud = false,
            _ => { eprintln!("{AYUDA}"); std::process::exit(2) }
        }
    }
    a
}

struct Estado {
    registro: RegistryState,
    asientos: SeatState,
    salidas: OutputState,
    capa: Option<LayerSurface>,
    puntero: Option<wl_pointer::WlPointer>,
    configurada: bool,
    salir: bool,
    a_render: Sender<ARender>,
}

fn main() {
    let a = args();
    let conexion = Connection::connect_to_env().expect("no hay sesión Wayland");
    let (globales, mut eventos) = registry_queue_init::<Estado>(&conexion).unwrap();
    let qh = eventos.handle();
    let compositor = CompositorState::bind(&globales, &qh).expect("sin wl_compositor");
    let capas = LayerShell::bind(&globales, &qh).expect("el compositor no tiene layer-shell");

    let (a_render, de_render) = channel();
    let (a_logica, de_logica) = channel();
    let mut estado = Estado {
        registro: RegistryState::new(&globales),
        asientos: SeatState::new(&globales, &qh),
        salidas: OutputState::new(&globales, &qh),
        capa: None,
        puntero: None,
        configurada: false,
        salir: false,
        a_render: a_render.clone(),
    };
    // Una vuelta para que lleguen los nombres de los monitores.
    eventos.roundtrip(&mut estado).unwrap();
    eventos.roundtrip(&mut estado).unwrap();
    let salida = estado.salidas.outputs().find(|o| {
        estado.salidas.info(o).and_then(|i| i.name).as_deref() == Some(a.pantalla.as_str())
    });
    if salida.is_none() {
        eprintln!("aviso: no encuentro el monitor {}; que decida el compositor", a.pantalla);
    }

    let superficie_wl = compositor.create_surface(&qh);
    let capa = capas.create_layer_surface(&qh, superficie_wl, Layer::Top, Some("pleamar"), salida.as_ref());
    capa.set_anchor(Anchor::TOP);
    capa.set_margin(a.margen, 0, 0, 0);
    capa.set_size(ANCHO, ALTO);
    capa.set_exclusive_zone(0);
    capa.set_keyboard_interactivity(KeyboardInteractivity::None);
    // La gráfica no recibe ratón: lo de debajo sigue siendo tuyo.
    if let Ok(region) = Region::new(&compositor) {
        region.add(0, 0, ANCHO as i32, 224);
        capa.wl_surface().set_input_region(Some(region.wl_region()));
    }
    capa.commit();

    let instancia = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let superficie = unsafe {
        instancia
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
                    NonNull::new(conexion.backend().display_ptr() as *mut _).unwrap(),
                ))),
                raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(
                    NonNull::new(capa.wl_surface().id().as_ptr() as *mut _).unwrap(),
                )),
            })
            .expect("no se pudo crear la superficie gráfica")
    };
    estado.capa = Some(capa);

    // Hasta que el compositor no configura la capa no se le puede pegar nada.
    while !estado.configurada {
        eventos.blocking_dispatch(&mut estado).unwrap();
    }

    let bloqueada = Arc::new(AtomicBool::new(false));
    let guion: Box<dyn logica::Guion> = match a.escena.as_str() {
        "marea" => Box::<escenas::marea::Marea>::default(),
        "isla" => Box::<escenas::isla::Isla>::default(),
        otra => {
            eprintln!("no conozco la escena «{otra}»\n{AYUDA}");
            std::process::exit(2)
        }
    };
    println!(
        "pleamar · modo {} · la lógica se bloquea {} ms tras cada decisión",
        if a.ingenuo { "INGENUO (un solo hilo)" } else { "separado (render independiente)" },
        a.bloqueo
    );
    let render = {
        let bloqueada = bloqueada.clone();
        let op = render::Opciones { hud: a.hud, ingenuo: a.ingenuo };
        std::thread::Builder::new()
            .name("render".into())
            .spawn(move || render::hilo(instancia, superficie, de_render, a_logica, bloqueada, op))
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
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.salir = true;
    }
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface, _: LayerSurfaceConfigure, _: u32) {
        self.configurada = true;
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
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for Estado {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.salidas
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

delegate_registry!(Estado);
impl ProvidesRegistryState for Estado {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registro
    }
    registry_handlers![OutputState, SeatState];
}

smithay_client_toolkit::delegate_dispatch2!(Estado);
