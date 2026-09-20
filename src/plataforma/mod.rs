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
///  · `clock`      → `{ hour, minute, second, day, month, year, weekday, time, date }`, al cambiar el minuto
///  · `clock.seconds` → lo mismo, cada segundo
///  · `files:x.json` → el texto de ese fichero cuando cambie
///
/// Y dos que solo se preguntan, con `sys.ask`: `env` (una variable del entorno) y
/// `clipboard` (lo que haya copiado); `clipboard.set` escribe en él.
pub fn servicio(de: &str, nombre: &str, avisar: Box<dyn Fn(Valor) + Send>) -> bool {
    // La hora, sin llamar a nadie.
    if nombre == "clock" || nombre == "clock.seconds" {
        return reloj::servicio(nombre, avisar);
    }
    // `files:ajustes.json`: avisa cuando ese fichero cambie, también si lo toca otro.
    if let Some(cual) = nombre.strip_prefix("files:") {
        return ficheros::vigilar(de, cual, avisar);
    }
    #[cfg(target_os = "linux")]
    if nombre == "apps" {
        // Leer cientos de ficheros no es cosa de un instante: en su hilo.
        return std::thread::Builder::new().name("apps".into()).spawn(move || avisar(escritorio::aplicaciones())).is_ok();
    }
    #[cfg(target_os = "linux")]
    match nombre {
        "audio" => return sistema::audio(avisar),
        "battery" => return sistema::bateria(avisar),
        "brightness" => return sistema::brillo(avisar),
        "network" => return sistema::red(avisar),
        "media" => return mpris::servicio(avisar),
        "notifications" => return avisos::servicio(avisar),
        "tray" => return bandeja::servicio(avisar),
        _ => {}
    }
    // `PLEAMAR_GENERICO=1` salta el camino de Hyprland: es como se prueba aquí lo que
    // verán los demás compositores.
    #[cfg(target_os = "linux")]
    if hyprland::esta() && std::env::var_os("PLEAMAR_GENERICO").is_none() {
        return hyprland::servicio(nombre, avisar);
    }
    // Cualquier otro Wayland: los protocolos que entienden todos.
    #[cfg(target_os = "linux")]
    if matches!(nombre, "window" | "workspaces") {
        return compositor::servicio(nombre, avisar);
    }
    let _ = (nombre, avisar);
    false
}

/// Pedirle algo a un servicio: `workspaces.focus`, 3.
pub fn orden(de: &str, nombre: &str, args: &[Valor]) -> Result<(), String> {
    if nombre.starts_with("files.") {
        return ficheros::orden(de, nombre, args);
    }
    if let ("clipboard.set", [Valor::Texto(t)]) = (nombre, args) {
        portapapeles_escribir(t);
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    if let ("apps.launch", [Valor::Texto(o)]) = (nombre, args) {
        return escritorio::lanzar(o);
    }
    #[cfg(target_os = "linux")]
    if nombre.starts_with("brightness.") {
        return sistema::brillo_orden(nombre, args);
    }
    #[cfg(target_os = "linux")]
    if nombre.starts_with("audio.") {
        return sistema::audio_orden(nombre, args);
    }
    #[cfg(target_os = "linux")]
    if nombre.starts_with("tray.") {
        return bandeja::orden(nombre, args);
    }
    #[cfg(target_os = "linux")]
    if nombre.starts_with("notifications.") {
        return avisos::orden(nombre, args);
    }
    #[cfg(target_os = "linux")]
    if nombre.starts_with("media.") {
        return mpris::orden(nombre, args);
    }
    #[cfg(target_os = "linux")]
    if hyprland::esta() && std::env::var_os("PLEAMAR_GENERICO").is_none() {
        return hyprland::orden(nombre, args);
    }
    #[cfg(target_os = "linux")]
    if nombre.starts_with("workspaces.") {
        return compositor::orden(nombre, args);
    }
    let _ = args;
    Err(format!("this system cannot do '{nombre}' yet"))
}

/// Dónde guarda pleamar lo que tiene que recordar de una vez para otra —qué plugins se
/// han aprobado—: la carpeta de ajustes de cada sistema.
pub fn carpeta_de_ajustes() -> std::path::PathBuf {
    let de = |v: &str| std::env::var_os(v).filter(|x| !x.is_empty()).map(std::path::PathBuf::from);
    let base = if cfg!(target_os = "windows") {
        de("APPDATA")
    } else if cfg!(target_os = "macos") {
        de("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        de("XDG_CONFIG_HOME").or_else(|| de("HOME").map(|h| h.join(".config")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("pleamar")
}

/// Preguntarle algo a un servicio y esperar la respuesta: `tray.menu`, de quién.
/// Puede tardar —hay otra aplicación al otro lado—, y por eso es cosa de la
/// lógica, que puede esperar sin que se note.
pub fn consulta(de: &str, nombre: &str, args: &[Valor]) -> Result<Valor, String> {
    if nombre.starts_with("files.") {
        return ficheros::consulta(de, nombre, args);
    }
    match (nombre, args) {
        // El entorno: lo que la sesión le dijo a este programa al arrancar.
        ("env", [Valor::Texto(cual)]) => {
            return Ok(std::env::var_os(cual).map_or(Valor::Nulo, |v| Valor::Texto(v.to_string_lossy().into_owned())));
        }
        ("env", _) => return Err("`env` takes the name of one variable: sys.ask(\"env\", \"HOME\")".into()),
        ("clipboard", []) => return Ok(portapapeles_leer().map_or(Valor::Nulo, Valor::Texto)),
        _ => {}
    }
    #[cfg(target_os = "linux")]
    if nombre.starts_with("tray.") {
        return bandeja::consulta(nombre, args);
    }
    let _ = args;
    Err(format!("this system cannot answer '{nombre}' yet"))
}

/// Abre o cierra la emergente número `k` de la escena: una superficie hija de
/// la principal, en `[x, y, ancho, alto]` dentro de ella, que enseña la escena
/// desde `origen`. La lámina le llega al render como las demás; si el sistema la
/// cierra (han pulsado fuera), avisa con `ARender::EmergenteCerrada`.
///
/// En Wayland, un `xdg_popup`. En Windows será una ventana sin marco con
/// `WS_EX_NOACTIVATE`; en macOS, un `NSPanel`.
pub fn emergente(k: usize, que: Option<([i32; 4], (f32, f32))>) {
    #[cfg(target_os = "linux")]
    wayland::emergente(k, que);
    let _ = (k, que);
}

/// Pega la superficie `cual` a otro borde, ya en marcha. En Wayland es una
/// petición de layer-shell —`set_anchor` y `set_margin` valen sobre una
/// superficie viva, sin volver a crearla—; en Windows será mover la ventana y
/// recolocar su AppBar, y en macOS cambiar de sitio el `NSPanel`.
pub fn anclar(cual: usize, ancla: crate::escena::Ancla) {
    #[cfg(target_os = "linux")]
    wayland::anclar(cual, ancla);
    let _ = (cual, ancla);
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
        Some(e) => ruta_de_ordenes(e).ok_or("I don't know where the sockets are")?,
        None => {
            let dir = ruta_de_ordenes("x").and_then(|r| r.parent().map(|p| p.to_owned())).ok_or("I don't know where the sockets are")?;
            let vivas: Vec<_> = std::fs::read_dir(&dir).map_err(|_| "there is no scene running")?.filter_map(Result::ok).map(|e| e.path()).filter(|p| std::os::unix::net::UnixStream::connect(p).is_ok()).collect();
            match vivas.as_slice() {
                [una] => una.clone(),
                [] => return Err("there is no scene running".into()),
                varias => return Err(format!("there are several scenes running; say which: {}", varias.iter().filter_map(|p| p.file_stem()).map(|n| n.to_string_lossy()).collect::<Vec<_>>().join(", "))),
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
    Err("this system has nowhere to receive commands yet: the named pipe is missing".into())
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
mod avisos;
#[cfg(target_os = "linux")]
mod bandeja;
#[cfg(target_os = "linux")]
mod compositor;
mod ficheros;
mod reloj;
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
pub fn atender(_: Vec<Superficie>, _: u32, _: wgpu::Instance, _: Sender<ARender>) {
    eprintln!("pleamar cannot put windows on this system yet: its src/plataforma/ is missing");
    std::process::exit(1);
}

/// Dónde está el fichero de un icono, por su nombre.
#[cfg(target_os = "linux")]
pub use wayland::icono;
#[cfg(not(target_os = "linux"))]
pub fn icono(_: &str) -> Option<std::path::PathBuf> {
    None
}
