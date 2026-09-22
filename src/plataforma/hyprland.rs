//! Hyprland, hablado por sus dos sockets: uno para preguntar y mandar, otro
//! por el que cuenta lo que pasa. Sin lanzar `hyprctl` ni una sola vez.

use super::Valor;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;

fn socket(cual: &str) -> Option<String> {
    let firma = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let base = std::env::var("XDG_RUNTIME_DIR").ok()?;
    Some(format!("{base}/hypr/{firma}/{cual}"))
}

pub fn esta() -> bool {
    socket(".socket.sock").is_some_and(|s| std::path::Path::new(&s).exists())
}

/// Una pregunta o una orden, y lo que contesta.
pub fn pedir(orden: &str) -> Result<String, String> {
    let ruta = socket(".socket.sock").ok_or("this does not look like Hyprland")?;
    let mut s = UnixStream::connect(ruta).map_err(|e| e.to_string())?;
    s.write_all(orden.as_bytes()).map_err(|e| e.to_string())?;
    let mut r = String::new();
    s.read_to_string(&mut r).map_err(|e| e.to_string())?;
    Ok(r)
}

fn json(orden: &str) -> Option<serde_json::Value> {
    serde_json::from_str(&pedir(orden).ok()?).ok()
}

fn escritorios() -> Valor {
    let activo = json("j/activeworkspace").and_then(|v| v["id"].as_f64()).unwrap_or(0.0);
    // Cada monitor tiene el suyo: es lo que necesita una barra por pantalla.
    let activos_por_monitor: Vec<f64> = json("j/monitors")
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|m| m["activeWorkspace"]["id"].as_f64())
        .collect();
    let mut lista: Vec<(i64, Valor)> = json("j/workspaces")
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|w| {
            let id = w["id"].as_i64()?;
            // Los negativos son los especiales (el cajón de notas y demás): no son sitios a los que ir.
            (id > 0).then(|| {
                (id, Valor::Mapa(vec![
                    ("id".into(), Valor::Num(id as f64)),
                    ("name".into(), Valor::Texto(w["name"].as_str().unwrap_or("").to_owned())),
                    ("windows".into(), Valor::Num(w["windows"].as_f64().unwrap_or(0.0))),
                    ("monitor".into(), Valor::Texto(w["monitor"].as_str().unwrap_or("").to_owned())),
                    // Si es el activo de su monitor, no solo el de todo el sistema.
                    ("active".into(), Valor::Si(activos_por_monitor.contains(&(id as f64)))),
                ]))
            })
        })
        .collect();
    lista.sort_by_key(|(id, _)| *id);
    Valor::Mapa(vec![("active".into(), Valor::Num(activo)), ("list".into(), Valor::Lista(lista.into_iter().map(|(_, v)| v).collect()))])
}

fn ventana() -> Valor {
    let v = json("j/activewindow").unwrap_or_default();
    let texto = |k: &str| Valor::Texto(v[k].as_str().unwrap_or("").to_owned());
    // Y en qué monitor está el FOCO. No el de la ventana activa: un monitor
    // sin ventanas también puede tener el foco, y es justo entonces cuando una
    // escena que te sigue tiene que ir allí. `j/monitors` lo dice siempre.
    let monitor = json("j/monitors")
        .and_then(|ms| ms.as_array()?.iter().find(|m| m["focused"].as_bool() == Some(true)).and_then(|m| m["name"].as_str().map(str::to_owned)))
        .unwrap_or_default();
    Valor::Mapa(vec![("title".into(), texto("title")), ("class".into(), texto("class")), ("monitor".into(), Valor::Texto(monitor))])
}

/// Escucha lo que cuenta Hyprland y, cuando algo de lo que interesa cambia,
/// vuelve a preguntar y avisa. Devuelve si el servicio existe.
pub fn servicio(nombre: &str, avisar: Box<dyn Fn(Valor) + Send>) -> bool {
    let (leer, interesan): (fn() -> Valor, &'static [&'static str]) = match nombre {
        "workspaces" => (escritorios, &["workspace", "createworkspace", "destroyworkspace", "openwindow", "closewindow", "movewindow", "focusedmon", "moveworkspace"]),
        "window" => (ventana, &["activewindow", "closewindow", "windowtitle", "focusedmon"]),
        _ => return false,
    };
    let Some(ruta) = socket(".socket2.sock") else { return false };
    let Ok(s) = UnixStream::connect(ruta) else { return false };
    std::thread::Builder::new()
        .name(format!("hyprland·{nombre}"))
        .spawn(move || {
            avisar(leer());
            for linea in BufReader::new(s).lines().map_while(Result::ok) {
                let suceso = linea.split(">>").next().unwrap_or("");
                // `workspacev2`, `activewindowv2`…: la misma noticia dos veces.
                if interesan.contains(&suceso) {
                    avisar(leer());
                }
            }
        })
        .is_ok()
}

pub fn orden(nombre: &str, args: &[Valor]) -> Result<(), String> {
    match (nombre, args) {
        // En esta casa la configuración de Hyprland es Lua, y sus órdenes también.
        ("workspaces.focus", [Valor::Num(n)]) => pedir(&format!("dispatch hl.dsp.focus({{ workspace = {} }})", *n as i64)).map(|_| ()),
        _ => Err(format!("I don't know how to do '{nombre}' with that")),
    }
}
