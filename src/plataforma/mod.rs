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
///  · `apps`       → `{ { name, exec, icon }, … }`, una vez
///  · `audio`      → `{ volume = 0.54, muted = false }`
///  · `battery`    → `{ present, percent, charging }`
///  · `network`    → `{ online, kind = "wired" | "wifi" | "none", name, strength }`
///  · `media`      → `{ playing, title, artist, album, player }`, o `{ player = "" }` si no suena nada
pub fn servicio(nombre: &str, avisar: Box<dyn Fn(Valor) + Send>) -> bool {
    #[cfg(target_os = "linux")]
    if nombre == "apps" {
        // Leer cientos de ficheros no es cosa de un instante: en su hilo.
        return std::thread::Builder::new().name("apps".into()).spawn(move || avisar(escritorio::aplicaciones())).is_ok();
    }
    #[cfg(target_os = "linux")]
    match nombre {
        "audio" => return sistema::audio(avisar),
        "battery" => return sistema::bateria(avisar),
        "network" => return sistema::red(avisar),
        "media" => return mpris::servicio(avisar),
        _ => {}
    }
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
    if let ("apps.launch", [Valor::Texto(o)]) = (nombre, args) {
        return escritorio::lanzar(o);
    }
    #[cfg(target_os = "linux")]
    if nombre.starts_with("audio.") {
        return sistema::audio_orden(nombre, args);
    }
    #[cfg(target_os = "linux")]
    if nombre.starts_with("media.") {
        return mpris::orden(nombre, args);
    }
    #[cfg(target_os = "linux")]
    if hyprland::esta() {
        return hyprland::orden(nombre, args);
    }
    let _ = args;
    Err(format!("este sistema no sabe hacer «{nombre}» todavía"))
}

/// Que un proceso que lanzamos no nos sobreviva, ni aunque nos maten a la
/// fuerza. En Linux se lo pedimos al núcleo; en Windows será un Job Object.
pub fn morir_con_el_padre(orden: &mut std::process::Command) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // Solo llama a `prctl`, que es de las que se pueden usar entre `fork` y `exec`.
        unsafe {
            orden.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
    }
    let _ = orden;
}

/// Órdenes desde fuera: un atajo global del compositor, un script, otra
/// aplicación. Una línea de texto por orden, por un socket con el nombre de la
/// escena. `pleamar --decir "emit toggle"` es el otro extremo.
///
/// En Linux y macOS, un socket de Unix. En Windows será una tubería con nombre.
#[cfg(unix)]
pub fn escuchar_ordenes(escena: &str, recibir: Box<dyn Fn(String) -> Option<String> + Send>) {
    use std::io::{BufRead, BufReader, Write};
    let Some(ruta) = ruta_de_ordenes(escena) else { return };
    let _ = std::fs::create_dir_all(ruta.parent().unwrap());
    let _ = std::fs::remove_file(&ruta);
    let Ok(escucha) = std::os::unix::net::UnixListener::bind(&ruta) else { return };
    std::thread::Builder::new()
        .name("órdenes".into())
        .spawn(move || {
            for mut c in escucha.incoming().map_while(Result::ok) {
                let Ok(lectura) = c.try_clone() else { continue };
                for linea in BufReader::new(lectura).lines().map_while(Result::ok) {
                    // Una pregunta (`get open`) se contesta por donde vino.
                    if let Some(respuesta) = recibir(linea) {
                        let _ = writeln!(c, "{respuesta}");
                    }
                }
            }
        })
        .ok();
}

#[cfg(unix)]
fn ruta_de_ordenes(escena: &str) -> Option<std::path::PathBuf> {
    let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
    Some(std::path::Path::new(&base).join("pleamar").join(format!("{escena}.sock")))
}

/// Decirle algo a una escena en marcha. Sin nombre, a la única que haya.
#[cfg(unix)]
pub fn decir(escena: Option<&str>, orden: &str) -> Result<(), String> {
    use std::io::Write;
    let ruta = match escena {
        Some(e) => ruta_de_ordenes(e).ok_or("no sé dónde están los sockets")?,
        None => {
            let dir = ruta_de_ordenes("x").and_then(|r| r.parent().map(|p| p.to_owned())).ok_or("no sé dónde están los sockets")?;
            let vivas: Vec<_> = std::fs::read_dir(&dir).map_err(|_| "no hay ninguna escena en marcha")?.filter_map(Result::ok).map(|e| e.path()).filter(|p| std::os::unix::net::UnixStream::connect(p).is_ok()).collect();
            match vivas.as_slice() {
                [una] => una.clone(),
                [] => return Err("no hay ninguna escena en marcha".into()),
                varias => return Err(format!("hay varias escenas en marcha; di cuál: {}", varias.iter().filter_map(|p| p.file_stem()).map(|n| n.to_string_lossy()).collect::<Vec<_>>().join(", "))),
            }
        }
    };
    let mut s = std::os::unix::net::UnixStream::connect(&ruta).map_err(|e| format!("{}: {e}", ruta.display()))?;
    writeln!(s, "{orden}").map_err(|e| e.to_string())?;
    // Ya está dicho todo: si era una pregunta, ahora llega lo que contesten.
    let _ = s.shutdown(std::net::Shutdown::Write);
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(2)));
    let mut respuesta = String::new();
    let _ = std::io::Read::read_to_string(&mut s, &mut respuesta);
    if !respuesta.is_empty() {
        print!("{respuesta}");
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn escuchar_ordenes(_: &str, _: Box<dyn Fn(String) -> Option<String> + Send>) {}
#[cfg(not(unix))]
pub fn decir(_: Option<&str>, _: &str) -> Result<(), String> {
    Err("este sistema no tiene todavía por dónde recibir órdenes: falta la tubería con nombre".into())
}

/// El portapapeles del sistema. `arboard` lo habla en los tres sistemas; vive
/// aquí para que el núcleo siga sin saber de ninguno.
pub fn portapapeles_leer() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

pub fn portapapeles_escribir(texto: &str) {
    // En Wayland el que copia tiene que seguir vivo para servir lo copiado: un
    // hilo que se queda con ello hasta que otro copie otra cosa.
    let texto = texto.to_owned();
    std::thread::spawn(move || {
        if let Ok(mut c) = arboard::Clipboard::new() {
            #[cfg(target_os = "linux")]
            {
                use arboard::SetExtLinux;
                let _ = c.set().wait().text(texto);
            }
            #[cfg(not(target_os = "linux"))]
            let _ = c.set_text(texto);
        }
    });
}

/// Lo que el render le pide a una ventana del sistema.
pub trait Ventana: Send {
    /// Por dónde entra el ratón: solo por estas cajas, en píxeles lógicos.
    fn region_de_entrada(&self, cajas: &[[i32; 4]]);
    /// Qué cursor se ve mientras el ratón esté encima.
    fn cursor(&self, c: crate::escena::Cursor);
    /// Pedir o soltar el teclado con la escena en marcha: un lanzador lo quiere
    /// entero mientras está abierto, y nada cuando no.
    fn teclado(&self, t: crate::escena::Teclado);
}

#[cfg(target_os = "linux")]
mod escritorio;
#[cfg(target_os = "linux")]
mod hyprland;
#[cfg(target_os = "linux")]
mod mpris;
#[cfg(target_os = "linux")]
mod sistema;
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
