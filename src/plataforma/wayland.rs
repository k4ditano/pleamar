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
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
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
use wayland_protocols::ext::background_effect::v1::client::{ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1, ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1};

/// Quien desenfoca lo de detrás de una superficie, si el compositor lo sabe
/// hacer (`ext-background-effect`): Hyprland y KWin, sí. Sin él, el cristal
/// es un tinte con su luz, sin nada borroso detrás.
static EFECTOS: std::sync::OnceLock<ExtBackgroundEffectManagerV1> = std::sync::OnceLock::new();

use wayland_protocols_wlr::screencopy::v1::client::{zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1}, zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1};
use wayland_client::protocol::{wl_buffer::WlBuffer, wl_shm::{self, WlShm}, wl_shm_pool::WlShmPool};

/// Quien fotografía la pantalla (`wlr-screencopy`), y la memoria compartida
/// donde deja la foto. Con los dos, pleamar puede ver lo que tiene detrás.
static CAPTURAS: std::sync::OnceLock<(ZwlrScreencopyManagerV1, WlShm)> = std::sync::OnceLock::new();

/// Lo que hace falta para capturar lo de detrás de una superficie: en qué
/// monitor está y dónde dentro de él —en píxeles lógicos—, y si ya hay una
/// captura en camino. El sitio lo pone el hilo de Wayland al configurarla.
struct Detras {
    id: u32,
    salida: wl_output::WlOutput,
    sitio: Mutex<Option<(i32, i32)>>,
    en_vuelo: std::sync::atomic::AtomicBool,
    /// La foto en camino, para poder olvidarla.
    marco: Mutex<Option<ZwlrScreencopyFrameV1>>,
}

/// Dónde cae una capa dentro de su monitor, por las reglas de layer-shell: el
/// margen solo cuenta del lado al que está pegada, y lo que no está pegado a
/// ningún lado va centrado. No sabe de las zonas que reserve otra capa (una
/// barra que aparta a las demás): con una de esas, se equivoca por su alto.
fn sitio_en_la_salida(ancla: Anchor, margen: [i32; 4], tam: (u32, u32), salida: (i32, i32)) -> (i32, i32) {
    let (w, h) = (tam.0 as i32, tam.1 as i32);
    let x = match (ancla.contains(Anchor::LEFT), ancla.contains(Anchor::RIGHT)) {
        (true, true) => margen[3] + (salida.0 - margen[3] - margen[1] - w) / 2,
        (true, false) => margen[3],
        (false, true) => salida.0 - w - margen[1],
        (false, false) => (salida.0 - w) / 2,
    };
    let y = match (ancla.contains(Anchor::TOP), ancla.contains(Anchor::BOTTOM)) {
        (true, true) => margen[0] + (salida.1 - margen[0] - margen[2] - h) / 2,
        (true, false) => margen[0],
        (false, true) => salida.1 - h - margen[2],
        (false, false) => (salida.1 - h) / 2,
    };
    (x, y)
}

/// Una foto en camino: de quién, y qué caja.
struct CapturaDe {
    detras: Arc<Detras>,
    al_cambiar: bool,
    formato: Mutex<Option<(u32, u32, u32)>>,
}

/// La memoria donde el compositor deja las fotos de una superficie. Se
/// reutiliza mientras mida lo mismo.
struct Foto {
    mapa: Arc<Mapa>,
    tam: (u32, u32, u32),
    _pool: WlShmPool,
    bufer: WlBuffer,
    _fd: std::os::fd::OwnedFd,
}

impl Drop for Foto {
    fn drop(&mut self) {
        self.bufer.destroy();
    }
}

/// La memoria de una foto, vista desde este proceso. Viaja al render sin
/// copiarse: el compositor no vuelve a escribir en ella hasta que se le pide
/// otra foto, y el render solo la pide después de haberla subido a la tarjeta.
struct Mapa {
    ptr: *mut u8,
    largo: usize,
}
unsafe impl Send for Mapa {}
unsafe impl Sync for Mapa {}
impl AsRef<[u8]> for Mapa {
    fn as_ref(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.largo) }
    }
}
impl Drop for Mapa {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.ptr as *mut libc::c_void, self.largo) };
    }
}

impl Dispatch2<ZwlrScreencopyFrameV1, Estado> for CapturaDe {
    fn event(&self, e: &mut Estado, marco: &ZwlrScreencopyFrameV1, ev: zwlr_screencopy_frame_v1::Event, _: &Connection, qh: &QueueHandle<Estado>) {
        use zwlr_screencopy_frame_v1::Event as E;
        let acabar = |marco: &ZwlrScreencopyFrameV1| {
            marco.destroy();
            *self.detras.marco.lock().unwrap() = None;
            self.detras.en_vuelo.store(false, Ordering::Relaxed);
        };
        match ev {
            // Solo en el formato de siempre: cuatro bytes, azul primero.
            E::Buffer { format: wayland_client::WEnum::Value(f), width, height, stride } if matches!(f, wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888) => {
                *self.formato.lock().unwrap() = Some((width, height, stride));
            }
            E::BufferDone => {
                let (Some(tam), Some((_, shm))) = (*self.formato.lock().unwrap(), CAPTURAS.get()) else { return acabar(marco) };
                let id = self.detras.id;
                if e.fotos.get(&id).is_none_or(|f| f.tam != tam) {
                    e.fotos.remove(&id);
                    let largo = (tam.2 * tam.1) as usize;
                    let Some(fd) = memfd(largo) else { return acabar(marco) };
                    let ptr = unsafe { libc::mmap(std::ptr::null_mut(), largo, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, std::os::fd::AsRawFd::as_raw_fd(&fd), 0) };
                    if ptr == libc::MAP_FAILED {
                        return acabar(marco);
                    }
                    let pool = shm.create_pool(std::os::fd::AsFd::as_fd(&fd), largo as i32, qh, Mudo);
                    let bufer = pool.create_buffer(0, tam.0 as i32, tam.1 as i32, tam.2 as i32, wl_shm::Format::Argb8888, qh, Mudo);
                    e.fotos.insert(id, Foto { mapa: Arc::new(Mapa { ptr: ptr as *mut u8, largo }), tam, _pool: pool, bufer, _fd: fd });
                }
                // Vigilando, espera a que algo cambie: con lo de detrás quieto, no
                // llega nada. Tras presentar, la trae la próxima vez que se pinte.
                if self.al_cambiar {
                    marco.copy_with_damage(&e.fotos[&id].bufer);
                } else {
                    marco.copy(&e.fotos[&id].bufer);
                }
            }
            E::Ready { .. } => {
                let id = self.detras.id;
                if let Some(f) = e.fotos.get(&id) {
                    let datos: Arc<dyn AsRef<[u8]> + Send + Sync> = f.mapa.clone();
                    let _ = e.a_render.send(ARender::Detras(Box::new(super::Detras { lamina: id, ancho: f.tam.0, alto: f.tam.1, zancada: f.tam.2, datos: Some(datos) })));
                }
                acabar(marco);
            }
            // Que el render no se quede esperándola.
            E::Failed => {
                let _ = e.a_render.send(ARender::Detras(Box::new(super::Detras { lamina: self.detras.id, ancho: 0, alto: 0, zancada: 0, datos: None })));
                acabar(marco)
            }
            _ => {}
        }
    }
}

fn memfd(largo: usize) -> Option<std::os::fd::OwnedFd> {
    unsafe {
        let fd = libc::memfd_create(c"pleamar-detras".as_ptr(), libc::MFD_CLOEXEC);
        if fd < 0 || libc::ftruncate(fd, largo as i64) != 0 {
            return None;
        }
        Some(std::os::fd::FromRawFd::from_raw_fd(fd))
    }
}
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
    /// Para pedir el aviso de «ya puedes pintar otro»: con qué cola, y de quién es.
    qh: QueueHandle<Estado>,
    id: u32,
    /// Su desenfoque, que se pide la primera vez que tiene cristal: pedirlo dos
    /// veces para la misma superficie es un error del protocolo.
    efecto: Mutex<Option<ExtBackgroundEffectSurfaceV1>>,
    /// Para ver lo que tiene detrás. Solo las capas: de una ventana no se sabe dónde está.
    detras: Option<Arc<Detras>>,
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

    /// Se aplica con el `commit` que hace el propio driver al presentar, como
    /// la región de entrada: el aviso llega cuando el compositor quiera otro.
    fn pedir_frame(&self) {
        self.wl.frame(&self.qh, FrameDe(self.id));
    }

    fn capturar_detras(&self, caja: [i32; 4], al_cambiar: bool) -> bool {
        let (Some(d), Some((gestor, _))) = (&self.detras, CAPTURAS.get()) else { return false };
        let Some((x, y)) = *d.sitio.lock().unwrap() else { return false };
        if d.en_vuelo.swap(true, Ordering::Relaxed) {
            return false;
        }
        // Sin el cursor: el cristal no lo refracta, va encima.
        let marco = gestor.capture_output_region(0, &d.salida, x + caja[0], y + caja[1], caja[2], caja[3], &self.qh, CapturaDe { detras: d.clone(), al_cambiar, formato: Mutex::new(None) });
        *d.marco.lock().unwrap() = Some(marco);
        true
    }

    fn cancelar_detras(&self) {
        let Some(d) = &self.detras else { return };
        if let Some(m) = d.marco.lock().unwrap().take() {
            m.destroy();
        }
        d.en_vuelo.store(false, Ordering::Relaxed);
    }

    /// Como la región de entrada, vale con el siguiente frame que se presente.
    fn region_de_desenfoque(&self, cajas: &[[i32; 4]]) {
        let Some(efectos) = EFECTOS.get() else { return };
        let mut efecto = self.efecto.lock().unwrap();
        if efecto.is_none() && cajas.is_empty() {
            return;
        }
        let efecto = efecto.get_or_insert_with(|| efectos.get_background_effect(&self.wl, &self.qh, Mudo));
        if cajas.is_empty() {
            efecto.set_blur_region(None);
        } else if let Ok(region) = Region::new(&self.compositor) {
            for c in cajas {
                region.add(c[0], c[1], c[2] - c[0], c[3] - c[1]);
            }
            efecto.set_blur_region(Some(region.wl_region()));
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

/// Cómo pide una superficie su sitio: pegada a un borde del monitor, o como una
/// ventana normal, que el compositor decora y coloca.
#[derive(Clone)]
enum Concha {
    Capa(LayerSurface),
    Ventana(Window),
}

impl Concha {
    fn wl(&self) -> &wl_surface::WlSurface {
        match self {
            Concha::Capa(c) => c.wl_surface(),
            Concha::Ventana(v) => v.wl_surface(),
        }
    }
    fn capa(&self) -> Option<&LayerSurface> {
        match self {
            Concha::Capa(c) => Some(c),
            Concha::Ventana(_) => None,
        }
    }
}

/// Una superficie puesta en un monitor.
struct Puesta {
    id: u32,
    /// Qué superficie de la escena es.
    cual: usize,
    concha: Concha,
    salida: wl_output::WlOutput,
    /// Para pintar a una escala que no sea entera.
    ventanilla: Option<WpViewport>,
    _escala: Option<WpFractionalScaleV1>,
    /// La última escala que dijo el compositor. Suele llegar ANTES de que la
    /// superficie esté configurada, cuando el render aún no la conoce.
    escala: f32,
    /// Se le entrega al render cuando el compositor la configura.
    pendiente: Option<(wgpu::Surface<'static>, String, i32)>,
    /// Lo que mide ahora: una ventana la estira quien quiera.
    tam: (u32, u32),
    /// Una capa: a qué bordes está pegada y con qué márgenes, para saber
    /// dónde cae en su monitor; y lo que hace falta para ver lo de detrás.
    capa_sitio: Option<(Anchor, [i32; 4])>,
    detras: Option<Arc<Detras>>,
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
    qh: QueueHandle<Estado>,
    /// La memoria de las fotos de lo de detrás, una por superficie.
    fotos: std::collections::HashMap<u32, Foto>,
}

/// Los objetos de Wayland que no nos cuentan nada.
struct Mudo;
impl<I: Proxy> Dispatch2<I, Estado> for Mudo {
    fn event(&self, _: &mut Estado, _: &I, _: I::Event, _: &Connection, _: &QueueHandle<Estado>) {}
}

/// El compositor ya ha enseñado el último frame de esa lámina y quiere otro.
/// A lo que no se ve —un monitor apagado— no se le avisa: ese silencio es lo
/// que deja al render dejar de pintar para nadie.
struct FrameDe(u32);
impl Dispatch2<wayland_client::protocol::wl_callback::WlCallback, Estado> for FrameDe {
    fn event(&self, e: &mut Estado, _: &wayland_client::protocol::wl_callback::WlCallback, ev: wayland_client::protocol::wl_callback::Event, _: &Connection, _: &QueueHandle<Estado>) {
        if let wayland_client::protocol::wl_callback::Event::Done { .. } = ev {
            let _ = e.a_render.send(ARender::Frame(self.0));
        }
    }
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
    /// Cuántas superficies de esta clase van en este monitor. `k` es el número del
    /// monitor, por orden de aparición: es lo que reparte `screens: each`.
    /// La superficie de wgpu que pinta sobre una de Wayland.
    fn superficie_de(&self, wl: &wl_surface::WlSurface) -> wgpu::Surface<'static> {
        unsafe {
            self.instancia
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
                        NonNull::new(self.conexion.backend().display_ptr() as *mut _).unwrap(),
                    ))),
                    raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(NonNull::new(wl.id().as_ptr() as *mut _).unwrap())),
                })
                .expect("the graphics surface could not be created")
        }
    }

    fn quiere_en(s: &Superficie, nombre: &str, k: usize) -> usize {
        match &s.pantallas {
            Pantallas::Todas => 1,
            Pantallas::Estas(n) => n.iter().filter(|x| x.as_str() == nombre).count(),
            Pantallas::Numero(x) => (*x == k) as usize,
        }
    }

    /// Pone en un monitor las superficies que la escena pida para él.
    fn poner_en(&mut self, salida: &wl_output::WlOutput, qh: &QueueHandle<Estado>) {
        let Some(info) = self.salidas.info(salida) else { return };
        let nombre = info.name.clone().unwrap_or_default();
        // Qué número de monitor es: el orden en que el compositor los cuenta.
        let numero = self.salidas.outputs().position(|o| &o == salida).unwrap_or(0);
        let mhz = info.modes.iter().find(|m| m.current).map_or(0, |m| m.refresh_rate);
        if let Some(c) = CERROJOS.get() {
            let mut m = c.monitores.lock().unwrap();
            if !m.iter().any(|(s, _, _)| s == salida) {
                m.push((salida.clone(), nombre.clone(), mhz));
            }
        }
        for cual in 0..self.pide.len() {
            // La de bloqueo no se pone: se echa, cuando la escena lo diga.
            if self.pide[cual].cerrojo {
                continue;
            }
            let ya = self.puestas.iter().filter(|p| &p.salida == salida && p.cual == cual).count();
            let quiere = Self::quiere_en(&self.pide[cual], &nombre, numero);
            for k in ya..quiere {
            let p = &self.pide[cual];
            // El HUD solo va debajo de la principal.
            let alto = p.alto + if cual == 0 { self.alto_extra } else { 0 };
            // Una ventana normal no se pone por monitor: la coloca el compositor.
            if p.ventana.is_some() && self.puestas.iter().any(|x| x.cual == cual) {
                continue;
            }
            let wl = self.compositor.create_surface(qh);
            let nivel = match p.nivel {
                Nivel::Fondo => Layer::Background,
                Nivel::Debajo => Layer::Bottom,
                Nivel::Encima => Layer::Top,
                Nivel::SobreTodo => Layer::Overlay,
            };
            // `kind: window`: una ventana de las normales, con su título y su marco.
            if let (Some(titulo), Some(em)) = (&p.ventana, EMERGENTES.get()) {
                let ventana = em.xdg.create_window(wl, WindowDecorations::RequestServer, qh);
                ventana.set_title(if titulo.is_empty() { "pleamar" } else { titulo });
                ventana.set_app_id("pleamar");
                ventana.set_min_size(Some((p.ancho, alto)));
                let id = self.siguiente_id;
                self.siguiente_id += 1;
                ventana.commit();
                let concha = Concha::Ventana(ventana);
                let ventanilla = self.ventanillas.as_ref().map(|v| v.get_viewport(concha.wl(), qh, Mudo));
                let escala = self.escalas.as_ref().map(|m| m.get_fractional_scale(concha.wl(), qh, EscalaDe(id)));
                let superficie = self.superficie_de(concha.wl());
                self.puestas.push(Puesta { id, cual, concha, salida: salida.clone(), ventanilla, _escala: escala, escala: 1.0, tam: (0, 0), pendiente: Some((superficie, nombre.clone(), mhz)), capa_sitio: None, detras: None });
                continue;
            }
            let capa = self.capas.create_layer_surface(qh, wl, nivel, Some("pleamar"), Some(salida));
            capa.set_anchor(bordes(p.ancla, p.ancho == 0));
            // La segunda en el mismo monitor, debajo de la primera: es para ensayar.
            let m = p.margen;
            let margen = [m[0] + k as i32 * (alto as i32 + 12), m[1], m[2], m[3]];
            capa.set_margin(margen[0], margen[1], margen[2], margen[3]);
            // Apuntada, por si la escena decide moverla de borde en marcha.
            if p.ancla_de.is_some() {
                if let Some(c) = CAPAS.get() {
                    c.puestas.lock().unwrap().push((cual, capa.clone(), margen, p.ancho == 0));
                }
            }
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
            let superficie = self.superficie_de(capa.wl_surface());
            let detras = Some(Arc::new(Detras { id, salida: salida.clone(), sitio: Mutex::new(None), en_vuelo: std::sync::atomic::AtomicBool::new(false), marco: Mutex::new(None) }));
            let capa_sitio = Some((bordes(p.ancla, p.ancho == 0), margen));
            self.puestas.push(Puesta { id, cual, concha: Concha::Capa(capa), salida: salida.clone(), ventanilla, _escala: escala, escala: 1.0, tam: (0, 0), pendiente: Some((superficie, nombre.clone(), mhz)), capa_sitio, detras });
            }
        }
    }

    fn quitar_de(&mut self, salida: &wl_output::WlOutput) {
        if let Some(c) = CERROJOS.get() {
            c.monitores.lock().unwrap().retain(|(s, _, _)| s != salida);
        }
        for p in self.puestas.iter().filter(|p| &p.salida == salida) {
            let _ = self.a_render.send(ARender::LaminaFuera(p.id));
            if let Some(c) = CAPAS.get() {
                c.puestas.lock().unwrap().retain(|(_, capa, _, _)| capa.wl_surface() != p.concha.wl());
            }
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

// ── el cerrojo ────────────────────────────────────────────────────

/// La pantalla de bloqueo (`kind: lock`). No es una superficie que se pinta
/// encima de todo: es el protocolo `ext-session-lock`, con el que el COMPOSITOR
/// garantiza que mientras dure no se ve ni se toca nada más, en ningún
/// monitor. Por eso no existe hasta que se echa, y por eso la abre el render
/// —que es quien sabe cuándo el `open:` de la escena se hace verdad— igual que
/// una emergente. Lo que pase después llega al hilo de siempre.
///
/// Y una cosa que hay que saber antes de usarlo: si quien bloquea se muere con
/// el cerrojo echado, la sesión SIGUE bloqueada. Es a propósito —lo contrario
/// sería que matar al bloqueador desbloquease— y es lo que obliga a que esto
/// no pueda fallar a medias.
struct Cerrojos {
    qh: QueueHandle<Estado>,
    compositor: CompositorState,
    gestor: smithay_client_toolkit::session_lock::SessionLockState,
    conexion: Connection,
    instancia: wgpu::Instance,
    ventanillas: Option<WpViewporter>,
    escalas: Option<WpFractionalScaleManagerV1>,
    /// Los monitores que hay ahora, con su nombre y su refresco: los mantiene
    /// al día el hilo de Wayland, y hace falta uno por cara.
    monitores: Mutex<Vec<(wl_output::WlOutput, String, i32)>>,
    echado: Mutex<Option<Echado>>,
    siguiente_id: AtomicU32,
}

struct Echado {
    cual: usize,
    /// La caja que la escena declara (`size:`), que se centra en cada monitor,
    /// y dónde cae su trozo del plano.
    caja: (u32, u32),
    origen: (f32, f32),
    cerrojo: smithay_client_toolkit::session_lock::SessionLock,
    caras: Vec<Cara>,
}

/// La superficie de bloqueo de UN monitor.
struct Cara {
    id: u32,
    // Por orden: primero se suelta lo que pinta, luego la superficie.
    pendiente: Option<wgpu::Surface<'static>>,
    ventanilla: Option<WpViewport>,
    _escala: Option<WpFractionalScaleV1>,
    nombre: String,
    mhz: i32,
    /// Dónde mira en el plano: el origen de la escena menos lo que haga falta
    /// para que su caja quede centrada. Se sabe al configurarla.
    mira: (f32, f32),
    superficie: smithay_client_toolkit::session_lock::SessionLockSurface,
}

static CERROJOS: std::sync::OnceLock<Cerrojos> = std::sync::OnceLock::new();

/// `Some((caja, origen))` echa el cerrojo de la superficie `cual`; `None` lo
/// quita. Lo llama el render, que antes de quitarlo ya ha soltado lo que pintaba.
pub fn cerrojo(cual: usize, que: Option<((u32, u32), (f32, f32))>) {
    let Some(c) = CERROJOS.get() else { return };
    let mut echado = c.echado.lock().unwrap();
    let Some((caja, origen)) = que else {
        if let Some(e) = echado.take() {
            e.cerrojo.unlock();
            drop(e);
            let _ = c.conexion.flush();
            println!("lock   · unlocked");
        }
        return;
    };
    if echado.is_some() {
        return;
    }
    let cerrojo = match c.gestor.lock(&c.qh) {
        Ok(l) => l,
        Err(_) => {
            eprintln!("lock   · this compositor has no ext-session-lock: the session cannot be locked from here");
            return;
        }
    };
    // Una cara por monitor, y todas a la vez: hasta que no están todas, el
    // compositor no da la sesión por bloqueada.
    let caras = c
        .monitores
        .lock()
        .unwrap()
        .iter()
        .map(|(salida, nombre, mhz)| {
            let wl = c.compositor.create_surface(&c.qh);
            let id = 2000 + c.siguiente_id.fetch_add(1, Ordering::Relaxed);
            let superficie = cerrojo.create_lock_surface(wl, salida, &c.qh);
            let ventanilla = c.ventanillas.as_ref().map(|v| v.get_viewport(superficie.wl_surface(), &c.qh, Mudo));
            let escala = c.escalas.as_ref().map(|m| m.get_fractional_scale(superficie.wl_surface(), &c.qh, EscalaDe(id)));
            let pinta = unsafe {
                c.instancia
                    .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                        raw_display_handle: Some(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(NonNull::new(c.conexion.backend().display_ptr() as *mut _).unwrap()))),
                        raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(NonNull::new(superficie.wl_surface().id().as_ptr() as *mut _).unwrap())),
                    })
                    .ok()
            };
            Cara { id, pendiente: pinta, ventanilla, _escala: escala, nombre: nombre.clone(), mhz: *mhz, mira: origen, superficie }
        })
        .collect();
    *echado = Some(Echado { cual, caja, origen, cerrojo, caras });
    let _ = c.conexion.flush();
}

/// Los bordes a los que se pega, como banderas del protocolo. Con ancho 0 se
/// pega también a izquierda y derecha: es lo que lo estira de lado a lado.
fn bordes(ancla: Ancla, todo_el_ancho: bool) -> Anchor {
    (match ancla {
        Ancla::Arriba => Anchor::TOP,
        Ancla::Abajo => Anchor::BOTTOM,
        Ancla::Izquierda => Anchor::LEFT,
        Ancla::Derecha => Anchor::RIGHT,
        Ancla::ArribaIzquierda => Anchor::TOP | Anchor::LEFT,
        Ancla::ArribaDerecha => Anchor::TOP | Anchor::RIGHT,
        Ancla::AbajoIzquierda => Anchor::BOTTOM | Anchor::LEFT,
        Ancla::AbajoDerecha => Anchor::BOTTOM | Anchor::RIGHT,
        Ancla::Centro => Anchor::empty(),
    }) | if todo_el_ancho { Anchor::LEFT | Anchor::RIGHT } else { Anchor::empty() }
}

/// Las capas que pueden cambiar de borde, para llegar a ellas sin pasar por el
/// hilo de Wayland. `set_anchor` y `set_margin` son peticiones y valen en
/// marcha: no hace falta volver a crear la superficie, que es lo que dejaba a
/// Marea sin poder elegir esquina mientras graba.
struct Capas {
    conexion: Connection,
    puestas: Mutex<Vec<(usize, LayerSurface, [i32; 4], bool)>>,
}
static CAPAS: std::sync::OnceLock<Capas> = std::sync::OnceLock::new();

pub fn anclar(cual: usize, ancla: Ancla) {
    let Some(c) = CAPAS.get() else { return };
    let mut alguna = false;
    for (k, capa, margen, ancho_cero) in c.puestas.lock().unwrap().iter() {
        if *k != cual {
            continue;
        }
        capa.set_anchor(bordes(ancla, *ancho_cero));
        capa.set_margin(margen[0], margen[1], margen[2], margen[3]);
        capa.commit();
        alguna = true;
    }
    if alguna {
        let _ = c.conexion.flush();
    }
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
impl WindowHandler for Estado {
    /// Han pulsado la cruz. Se cierra esa; si era la última, se acaba.
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, v: &Window) {
        self.se_fue(&v.wl_surface().clone());
        if self.puestas.is_empty() {
            self.salir = true;
        }
    }
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, v: &Window, conf: WindowConfigure, _: u32) {
        let tam = (conf.new_size.0.map_or(0, |x| x.get()), conf.new_size.1.map_or(0, |x| x.get()));
        self.configurada(&v.wl_surface().clone(), tam);
    }
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
                ventana: Box::new(VentanaWayland { wl: a.popup.wl_surface().clone(), compositor: self.compositor.clone(), cursores: e.cursores.clone(), serie: e.serie.clone(), capa: None, qh: e.qh.clone(), id: a.id, efecto: Mutex::new(None), detras: None }),
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
    if let Ok(m) = globales.bind::<ExtBackgroundEffectManagerV1, _, _>(&qh, 1..=1, Mudo) {
        let _ = EFECTOS.set(m);
    }
    if let (Ok(c), Ok(s)) = (globales.bind::<ZwlrScreencopyManagerV1, _, _>(&qh, 1..=3, Mudo), globales.bind::<WlShm, _, _>(&qh, 1..=1, Mudo)) {
        let _ = CAPTURAS.set((c, s));
    }
    let mut estado = Estado {
        registro: RegistryState::new(&globales),
        asientos: SeatState::new(&globales, &qh),
        salidas: OutputState::new(&globales, &qh),
        capas: LayerShell::bind(&globales, &qh).expect("the compositor has no layer-shell"),
        ventanillas: globales.bind(&qh, 1..=1, Mudo).ok(),
        escalas: globales.bind(&qh, 1..=1, Mudo).ok(),
        compositor,
        conexion: conexion.clone(),
        qh: qh.clone(),
        fotos: std::collections::HashMap::new(),
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
            let _ = CAPAS.set(Capas { conexion: conexion.clone(), puestas: Mutex::default() });
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
    let _ = CERROJOS.set(Cerrojos {
        qh: qh.clone(),
        compositor: estado.compositor.clone(),
        gestor: smithay_client_toolkit::session_lock::SessionLockState::new(&globales, &qh),
        conexion: conexion.clone(),
        instancia: estado.instancia.clone(),
        ventanillas: estado.ventanillas.clone(),
        escalas: estado.escalas.clone(),
        monitores: Mutex::default(),
        echado: Mutex::default(),
        siguiente_id: AtomicU32::new(0),
    });
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
    while !estado.salir && !SALIDA.load(std::sync::atomic::Ordering::Relaxed) {
        eventos.blocking_dispatch(&mut estado).unwrap();
    }
}

/// Lo pide el render cuando un clic derecho no lo ha querido nadie.
static SALIDA: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn pedir_salir() {
    SALIDA.store(true, std::sync::atomic::Ordering::Relaxed);
}

impl Estado {
    /// Una superficie se cerró: su monitor se fue, o alguien cerró la ventana.
    fn se_fue(&mut self, wl: &wl_surface::WlSurface) {
        if let Some(p) = self.puestas.iter().find(|p| p.concha.wl() == wl) {
            let _ = self.a_render.send(ARender::LaminaFuera(p.id));
        }
        self.puestas.retain(|p| p.concha.wl() != wl);
        if let Some(c) = CAPAS.get() {
            c.puestas.lock().unwrap().retain(|(_, capa, _, _)| capa.wl_surface() != wl);
        }
    }

    /// Hasta que el compositor no la configura no se le puede pegar nada: es ahora
    /// cuando pasa a manos del render. Vale igual para un panel y para una ventana.
    fn configurada(&mut self, wl: &wl_surface::WlSurface, nuevo: (u32, u32)) {
        let Some(p) = self.puestas.iter_mut().find(|p| p.concha.wl() == wl) else { return };
        let suya = &self.pide[p.cual];
        let pedido = (suya.ancho, suya.alto + if p.cual == 0 { self.alto_extra } else { 0 });
        let origen = suya.origen;
        // Lo que el compositor haya dado; si dice 0, lo que se pidió.
        let tam = (if nuevo.0 > 0 { nuevo.0 } else { pedido.0 }, if nuevo.1 > 0 { nuevo.1 } else { pedido.1 });
        if let Some(v) = &p.ventanilla {
            v.set_destination(tam.0 as i32, tam.1 as i32);
        }
        // Dónde cae dentro de su monitor: lo que hace falta para fotografiar lo de detrás.
        if let (Some((ancla, margen)), Some(d)) = (p.capa_sitio, &p.detras) {
            if let Some((w, h)) = self.salidas.info(&p.salida).and_then(|i| i.logical_size) {
                *d.sitio.lock().unwrap() = Some(sitio_en_la_salida(ancla, margen, tam, (w, h)));
            }
        }
        // Ya estaba entregada y ha cambiado de tamaño: alguien ha estirado la ventana.
        if p.pendiente.is_none() && p.tam != tam {
            let _ = self.a_render.send(ARender::TamLamina(p.id, (tam.0 as f32, tam.1 as f32)));
        }
        p.tam = tam;
        if let Some((superficie, nombre, mhz)) = p.pendiente.take() {
            if let (Some(em), Some(capa)) = (EMERGENTES.get(), p.concha.capa()) {
                // Mientras no se vea el ratón en ninguna, las emergentes cuelgan de la primera.
                em.madre.lock().unwrap().get_or_insert_with(|| Madre { capa: capa.clone(), escala: p.escala, nombre: nombre.clone(), mhz });
            }
            let _ = self.a_render.send(ARender::Lamina(Box::new(gpu::NuevaLamina {
                id: p.id,
                superficie,
                ventana: Box::new(VentanaWayland { wl: p.concha.wl().clone(), compositor: self.compositor.clone(), cursores: self.cursores.clone(), serie: self.serie.clone(), capa: p.concha.capa().cloned(), qh: self.qh.clone(), id: p.id, efecto: Mutex::new(None), detras: p.detras.clone() }),
                escala: p.escala,
                tam,
                mhz,
                nombre,
                vista: gpu::Vista { superficie: p.cual, emergente: None, origen, tam: (tam.0 as f32, tam.1 as f32) },
            })));
        }
    }
}

impl LayerShellHandler for Estado {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, capa: &LayerSurface) {
        self.se_fue(capa.wl_surface());
    }
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, capa: &LayerSurface, conf: LayerSurfaceConfigure, _: u32) {
        self.configurada(&capa.wl_surface().clone(), conf.new_size);
    }
}

impl smithay_client_toolkit::session_lock::SessionLockHandler for Estado {
    /// El compositor lo confirma: ahora sí, y no antes, la sesión está bloqueada.
    fn locked(&mut self, _: &Connection, _: &QueueHandle<Self>, _: smithay_client_toolkit::session_lock::SessionLock) {
        println!("lock   · the session is locked");
        let _ = self.a_render.send(ARender::Cerrojo(true));
    }
    /// No lo ha dado —ya hay otro bloqueador—, o lo ha dado por terminado.
    fn finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: smithay_client_toolkit::session_lock::SessionLock) {
        eprintln!("lock   · the compositor did not grant the lock, or ended it: is another locker running?");
        let _ = self.a_render.send(ARender::Cerrojo(false));
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        superficie: smithay_client_toolkit::session_lock::SessionLockSurface,
        conf: smithay_client_toolkit::session_lock::SessionLockSurfaceConfigure,
        _: u32,
    ) {
        let Some(c) = CERROJOS.get() else { return };
        let mut echado = c.echado.lock().unwrap();
        let Some(e) = echado.as_mut() else { return };
        let (cual, caja, origen) = (e.cual, e.caja, e.origen);
        let Some(cara) = e.caras.iter_mut().find(|k| k.superficie.wl_surface() == superficie.wl_surface()) else { return };
        let tam = conf.new_size;
        if let Some(v) = &cara.ventanilla {
            v.set_destination(tam.0 as i32, tam.1 as i32);
        }
        // Su caja, centrada en ESTE monitor: lo que sobra alrededor también se
        // ve, así que el fondo se pinta grande y cada monitor enseña lo suyo.
        cara.mira = (origen.0 - (tam.0 as f32 - caja.0 as f32) / 2.0, origen.1 - (tam.1 as f32 - caja.1 as f32) / 2.0);
        if let Some(pinta) = cara.pendiente.take() {
            let _ = self.a_render.send(ARender::Lamina(Box::new(gpu::NuevaLamina {
                id: cara.id,
                superficie: pinta,
                ventana: Box::new(VentanaWayland { wl: cara.superficie.wl_surface().clone(), compositor: self.compositor.clone(), cursores: self.cursores.clone(), serie: self.serie.clone(), capa: None, qh: self.qh.clone(), id: cara.id, efecto: Mutex::new(None), detras: None }),
                escala: 1.0,
                tam,
                mhz: cara.mhz,
                nombre: format!("{} (lock)", cara.nombre),
                vista: gpu::Vista { superficie: cual, emergente: None, origen: cara.mira, tam: (tam.0 as f32, tam.1 as f32) },
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
                let en_cerrojo = CERROJOS.get().and_then(|c| c.echado.lock().unwrap().as_ref().and_then(|ec| ec.caras.iter().find(|k| k.superficie.wl_surface() == &e.surface).map(|k| k.mira)));
                if let Some(mira) = en_cerrojo {
                    (x, y) = (x + mira.0, y + mira.1);
                } else if let Some(a) = em.abiertas.lock().unwrap().iter().find(|a| a.popup.wl_surface() == &e.surface) {
                    (x, y) = (x + a.origen.0, y + a.origen.1);
                } else if let Some(p) = self.puestas.iter().find(|p| p.concha.wl() == &e.surface) {
                    let info = self.salidas.info(&p.salida);
                    // El ratón sobre una superficie está en el trozo del plano que ella enseña.
                    let origen = self.pide[p.cual].origen;
                    (x, y) = (x + origen.0, y + origen.1);
                    // Las emergentes cuelgan de una capa: de una ventana normal, todavía no.
                    if let Some(capa) = p.concha.capa() {
                        *em.madre.lock().unwrap() = Some(Madre {
                            capa: capa.clone(),
                            escala: p.escala,
                            nombre: info.as_ref().and_then(|i| i.name.clone()).unwrap_or_default(),
                            mhz: info.as_ref().and_then(|i| i.modes.iter().find(|m| m.current).map(|m| m.refresh_rate)).unwrap_or(0),
                        });
                    }
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
                    // salida de emergencia de un prototipo sin teclado. Si alguna
                    // superficie sí lo usa, la decisión es del render —él sabe si el
                    // clic ha caído encima de algo—, y vuelve por `pedir_salir`.
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
        if let Some(p) = self.puestas.iter_mut().find(|p| p.concha.wl() == wl) {
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
