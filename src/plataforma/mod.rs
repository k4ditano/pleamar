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
//! Y una cuarta que no es de ventanas: encontrar un icono por su nombre.

#[cfg(not(target_os = "linux"))]
use crate::escena::{ARender, Superficie};
#[cfg(not(target_os = "linux"))]
use std::sync::mpsc::Sender;

/// Lo que el render le pide a una ventana del sistema.
pub trait Ventana: Send {
    /// Por dónde entra el ratón: solo por estas cajas, en píxeles lógicos.
    fn region_de_entrada(&self, cajas: &[[i32; 4]]);
}

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
