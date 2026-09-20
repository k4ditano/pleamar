//! Guardar y recordar: lo que una escena necesita para que sus ajustes sobrevivan a
//! cerrarla. Cada escena —y cada plugin— tiene **su propia carpeta**, y no puede salir
//! de ella: sin rutas absolutas, sin `..`, sin leer el `.ssh` de nadie. Es lo mismo que
//! hace `require` con los módulos.
//!
//! En Linux, `~/.local/share/pleamar/<quién>/`; en Windows, `%APPDATA%`; en macOS,
//! `~/Library/Application Support`.

use super::Valor;
use std::path::{Path, PathBuf};

/// Dónde guarda sus cosas cada escena o plugin.
fn carpeta(de: &str) -> PathBuf {
    let var = |v: &str| std::env::var_os(v).filter(|x| !x.is_empty()).map(PathBuf::from);
    let base = if cfg!(target_os = "windows") {
        var("APPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        var("XDG_DATA_HOME").or_else(|| var("HOME").map(|h| h.join(".local/share")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("pleamar").join(de)
}

/// Un nombre de fichero, no una ruta: letras, cifras, `_`, `-`, `.` y carpetas de dentro.
fn dentro(de: &str, nombre: &str) -> Result<PathBuf, String> {
    let limpio = !nombre.is_empty()
        && !nombre.starts_with('/')
        && nombre.split('/').all(|t| !t.is_empty() && t != ".." && t != "." && t.chars().all(|c| c.is_alphanumeric() || "_-.".contains(c)));
    if !limpio {
        return Err(format!("'{nombre}' is not a name inside this scene's folder: no `..`, no full paths"));
    }
    Ok(carpeta(de).join(nombre))
}

/// `sys.ask("files.read", "settings.json")` · `("files.list")` · `("files.exists", n)`
pub fn consulta(de: &str, que: &str, args: &[Valor]) -> Result<Valor, String> {
    match (que, args) {
        ("files.read", [Valor::Texto(nombre)]) => match std::fs::read_to_string(dentro(de, nombre)?) {
            Ok(t) => Ok(Valor::Texto(t)),
            // Que no esté todavía no es un fallo: es la primera vez.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Valor::Nulo),
            Err(e) => Err(format!("cannot read '{nombre}': {e}")),
        },
        ("files.exists", [Valor::Texto(nombre)]) => Ok(Valor::Si(dentro(de, nombre)?.exists())),
        ("files.list", []) => {
            let mut nombres: Vec<String> = std::fs::read_dir(carpeta(de))
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter(|e| e.path().is_file())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            nombres.sort();
            Ok(Valor::Lista(nombres.into_iter().map(Valor::Texto).collect()))
        }
        ("files.folder", []) => Ok(Valor::Texto(carpeta(de).to_string_lossy().into_owned())),
        _ => Err(format!("'{que}' is not asked like that: files.read(name), files.exists(name), files.list(), files.folder()")),
    }
}

/// `sys.call("files.write", "settings.json", text)` · `("files.remove", name)`
pub fn orden(de: &str, que: &str, args: &[Valor]) -> Result<(), String> {
    match (que, args) {
        ("files.write", [Valor::Texto(nombre), contenido]) => {
            let ruta = dentro(de, nombre)?;
            let texto = match contenido {
                Valor::Texto(t) => t.clone(),
                Valor::Num(n) => n.to_string(),
                Valor::Si(b) => b.to_string(),
                otro => return Err(format!("what gets written is a text, not a {otro:?}")),
            };
            if let Some(padre) = ruta.parent() {
                std::fs::create_dir_all(padre).map_err(|e| format!("cannot create {}: {e}", padre.display()))?;
            }
            // Primero al lado, luego en su sitio: si se corta la luz a medias, lo de antes sigue.
            let medias = PathBuf::from(format!("{}.nuevo", ruta.display()));
            std::fs::write(&medias, texto).map_err(|e| format!("cannot write '{nombre}': {e}"))?;
            std::fs::rename(&medias, &ruta).map_err(|e| format!("cannot write '{nombre}': {e}"))
        }
        ("files.remove", [Valor::Texto(nombre)]) => match std::fs::remove_file(dentro(de, nombre)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("cannot remove '{nombre}': {e}")),
        },
        _ => Err(format!("'{que}' is not asked like that: files.write(name, text), files.remove(name)")),
    }
}

/// Lo que se vigila: que el fichero cambie por debajo (otra aplicación lo ha tocado).
/// Mira su fecha cuatro veces por segundo, que vale igual en los tres sistemas.
pub fn vigilar(de: &str, nombre: &str, avisar: Box<dyn Fn(Valor) + Send>) -> bool {
    let Ok(ruta) = dentro(de, nombre) else { return false };
    std::thread::Builder::new()
        .name(format!("file {nombre}"))
        .spawn(move || {
            let fecha = |r: &Path| std::fs::metadata(r).and_then(|m| m.modified()).ok();
            let mut ultima = None;
            loop {
                let ahora = fecha(&ruta);
                if ahora != ultima {
                    ultima = ahora;
                    avisar(match std::fs::read_to_string(&ruta) {
                        Ok(t) => Valor::Texto(t),
                        Err(_) => Valor::Nulo,
                    });
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        })
        .is_ok()
}
