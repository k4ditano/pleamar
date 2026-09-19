//! La frontera con el sistema. Todo lo que sabe de Wayland —y, el día que
//! toque, de Win32 o de AppKit— vive aquí debajo; el resto del programa solo
//! conoce esto.
//!
//! Una plataforma tiene que saber hacer tres cosas:
//!  · poner las superficies que pide una escena (`Superficie`: tamaño lógico,
//!    ancla, nivel, reserva, pantallas) y entregárselas al render como láminas,
//!    también cuando un monitor llega o se va, o cambia de escala;
//!  · contarle al render dónde está el ratón y cuándo pulsa;
//!  · decirle al sistema por dónde entra el ratón (`Ventana::region_de_entrada`),
//!    para que lo transparente deje pasar el clic.
//!
//! Y dos que no son de ventanas: encontrar un icono por su nombre, y los
//! **servicios** —lo que pasa en el sistema: escritorios, ventana activa…—, que
//! la lógica pide por un nombre que es el mismo en todos los sistemas. Lo que un
//! sistema no tenga, dice que no lo tiene, y la escena decide qué hacer sin ello.

#[cfg(not(target_os = "linux"))]
use crate::escena::{ARender, Superficie};
#[cfg(not(target_os = "linux"))]
use std::sync::mpsc::Sender;

/// Un dato del sistema, con la forma de un JSON: es lo que un servicio le
/// cuenta a la lógica, que lo recibe como una tabla.
#[derive(Clone, Debug)]
pub enum Valor {
    Nulo,
    Si(bool),
    Num(f64),
    Texto(String),
    Lista(Vec<Valor>),
    Mapa(Vec<(String, Valor)>),
}

/// Empieza a escuchar un servicio. `avisar` se llama con el estado de ahora y
/// luego cada vez que cambie. Devuelve si este sistema lo tiene.
///
/// Los nombres son los mismos en todas partes:
///  · `workspaces` → `{ active = 3, list = { { id, name, windows, monitor }, … } }`
///  · `window`     → `{ title, class }`
pub fn servicio(nombre: &str, avisar: Box<dyn Fn(Valor) + Send>) -> bool {
    #[cfg(target_os = "linux")]
    if hyprland::esta() {
        return hyprland::servicio(nombre, avisar);
    }
    let _ = (nombre, avisar);
    false
}

/// Pedirle algo a un servicio: `workspaces.focus`, 3.
pub fn orden(nombre: &str, args: &[Valor]) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if hyprland::esta() {
        return hyprland::orden(nombre, args);
    }
    let _ = args;
    Err(format!("este sistema no sabe hacer «{nombre}» todavía"))
}

/// Lo que el render le pide a una ventana del sistema.
pub trait Ventana: Send {
    /// Por dónde entra el ratón: solo por estas cajas, en píxeles lógicos.
    fn region_de_entrada(&self, cajas: &[[i32; 4]]);
}

#[cfg(target_os = "linux")]
mod hyprland;
#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
pub use wayland::atender;

/// Sin plataforma todavía: el núcleo compila —es la guarda de que sigue siendo
/// portable— pero no hay dónde pintar.
#[cfg(not(target_os = "linux"))]
pub fn atender(_: Superficie, _: u32, _: wgpu::Instance, _: Sender<ARender>) {
    eprintln!("pleamar todavía no sabe poner ventanas en este sistema: falta src/plataforma/ para él");
    std::process::exit(1);
}

/// Dónde está el fichero de un icono, por su nombre.
#[cfg(target_os = "linux")]
pub use wayland::icono;
#[cfg(not(target_os = "linux"))]
pub fn icono(_: &str) -> Option<std::path::PathBuf> {
    None
}
