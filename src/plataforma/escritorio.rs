//! Lo que un escritorio de freedesktop sabe de sí mismo sin preguntarle a ningún
//! compositor: qué aplicaciones hay instaladas, y cómo lanzar una.

use super::Valor;
use std::collections::BTreeMap;

/// Los `.desktop` de todas las carpetas de aplicaciones, sin los escondidos.
pub fn aplicaciones() -> Valor {
    let casa = std::env::var("HOME").unwrap_or_default();
    let datos = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    let mut carpetas = vec![format!("{casa}/.local/share/applications")];
    carpetas.extend(datos.split(':').map(|d| format!("{d}/applications")));
    // Por nombre, y gana la primera carpeta: la del usuario pisa a la del sistema.
    let mut apps: BTreeMap<String, (String, String)> = BTreeMap::new();
    for carpeta in carpetas {
        let Ok(dir) = std::fs::read_dir(&carpeta) else { continue };
        for f in dir.filter_map(Result::ok).filter(|f| f.path().extension().is_some_and(|e| e == "desktop")) {
            let Ok(texto) = std::fs::read_to_string(f.path()) else { continue };
            let (mut nombre, mut orden, mut icono, mut oculta, mut dentro) = (None, None, String::new(), false, false);
            for l in texto.lines() {
                if l.starts_with('[') {
                    dentro = l == "[Desktop Entry]";
                } else if dentro {
                    match l.split_once('=') {
                        Some(("Name", v)) => nombre = nombre.or(Some(v.to_owned())),
                        Some(("Exec", v)) => orden = Some(v.split_whitespace().filter(|p| !p.starts_with('%')).collect::<Vec<_>>().join(" ")),
                        Some(("Icon", v)) => icono = v.to_owned(),
                        Some(("NoDisplay" | "Hidden", "true")) => oculta = true,
                        Some(("Type", v)) if v != "Application" => oculta = true,
                        _ => {}
                    }
                }
            }
            if let (Some(n), Some(o), false) = (nombre, orden, oculta) {
                apps.entry(n).or_insert((o, icono));
            }
        }
    }
    Valor::Lista(apps.into_iter().map(|(n, (o, i))| Valor::Mapa(vec![("name".into(), Valor::Texto(n)), ("exec".into(), Valor::Texto(o)), ("icon".into(), Valor::Texto(i))])).collect())
}

/// Lanza una orden y se desentiende de ella: no es hija de la lógica, y no muere con nosotros.
pub fn lanzar(orden: &str) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let mut hijo = Command::new("setsid").args(["-f", "sh", "-c", orden]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        let _ = hijo.wait();
    });
    Ok(())
}
