//! Lo que Linux cuenta de sí mismo sin compositor de por medio: el audio, la
//! batería y la red. Cada servicio es un hilo que avisa solo cuando algo cambia.
//!
//! Los nombres y las tablas son los de todos los sistemas; aquí solo está quién
//! contesta en Linux. En Windows serán WASAPI, `GetSystemPowerStatus` y
//! `INetworkListManager`; en macOS, CoreAudio, IOKit y `SCNetworkReachability`.

use super::Valor;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::Duration;

const SALIDA: &str = "@DEFAULT_AUDIO_SINK@";

fn hilo(nombre: &str, f: impl FnOnce() + Send + 'static) -> bool {
    std::thread::Builder::new().name(nombre.into()).spawn(f).is_ok()
}

fn salida_de(orden: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(orden).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Avisa solo si lo que hay que contar ha cambiado.
fn si_cambia(avisar: &dyn Fn(Valor), ultimo: &mut String, v: Valor) {
    let huella = format!("{v:?}");
    if huella != *ultimo {
        *ultimo = huella;
        avisar(v);
    }
}

// ── audio ─────────────────────────────────────────────────────────

/// `{ volume = 0.54, muted = false }`. PipeWire, por `wpctl`; y `pactl
/// subscribe` para enterarse de los cambios sin preguntar a cada rato.
fn audio_ahora() -> Option<Valor> {
    // «Volume: 0.54 [MUTED]»
    let s = salida_de("wpctl", &["get-volume", SALIDA])?;
    let volumen: f64 = s.split_whitespace().nth(1)?.parse().ok()?;
    Some(Valor::Mapa(vec![("volume".into(), Valor::Num(volumen)), ("muted".into(), Valor::Si(s.contains("MUTED")))]))
}

pub fn audio(avisar: Box<dyn Fn(Valor) + Send>) -> bool {
    let Some(ahora) = audio_ahora() else { return false };
    hilo("audio", move || {
        let mut ultimo = String::new();
        si_cambia(&*avisar, &mut ultimo, ahora);
        loop {
            let mut orden = Command::new("pactl");
            orden.arg("subscribe").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
            super::morir_con_el_padre(&mut orden);
            if let Ok(mut hijo) = orden.spawn() {
                for linea in BufReader::new(hijo.stdout.take().unwrap()).lines().map_while(Result::ok) {
                    // Un cambio en una salida, o en el servidor: puede haber cambiado cuál es la de por defecto.
                    if linea.contains("sink") || linea.contains("server") {
                        if let Some(v) = audio_ahora() { si_cambia(&*avisar, &mut ultimo, v) }
                    }
                }
                let _ = hijo.wait();
            }
            // Se cayó el servidor de audio (o no hay `pactl`): se pregunta de vez en cuando hasta que vuelva.
            std::thread::sleep(Duration::from_secs(2));
            if let Some(v) = audio_ahora() { si_cambia(&*avisar, &mut ultimo, v) }
        }
    })
}

pub fn audio_orden(que: &str, args: &[Valor]) -> Result<(), String> {
    let pedir = |a: &[&str]| salida_de("wpctl", a).map(|_| ()).ok_or_else(|| "wpctl refused".to_string());
    match (que, args) {
        ("audio.volume", [Valor::Num(v)]) => pedir(&["set-volume", SALIDA, &format!("{:.3}", v.clamp(0.0, 1.0))]),
        // Un paso, en tanto por uno: 0.05 sube, -0.05 baja. Nunca por encima del 100 %.
        ("audio.step", [Valor::Num(d)]) => pedir(&["set-volume", "-l", "1.0", SALIDA, &format!("{:.3}{}", d.abs(), if *d < 0.0 { "-" } else { "+" })]),
        ("audio.mute", []) => pedir(&["set-mute", SALIDA, "toggle"]),
        ("audio.mute", [Valor::Si(si)]) => pedir(&["set-mute", SALIDA, if *si { "1" } else { "0" }]),
        _ => Err(format!("'{que}' is not asked like that: audio.volume(0..1), audio.step(±0.05), audio.mute([true|false])")),
    }
}

// ── batería ───────────────────────────────────────────────────────

/// `{ present = true, percent = 83, charging = false }`. Un sobremesa contesta
/// `{ present = false }`: el servicio existe, lo que no hay es batería.
fn bateria_ahora() -> Valor {
    let leer = |ruta: std::path::PathBuf| std::fs::read_to_string(ruta).ok().map(|s| s.trim().to_owned());
    let pila = std::fs::read_dir("/sys/class/power_supply").ok().and_then(|d| {
        d.filter_map(Result::ok).map(|f| f.path()).find(|p| leer(p.join("type")).as_deref() == Some("Battery") && p.join("capacity").exists())
    });
    let Some(pila) = pila else { return Valor::Mapa(vec![("present".into(), Valor::Si(false))]) };
    let tanto = leer(pila.join("capacity")).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let estado = leer(pila.join("status")).unwrap_or_default();
    Valor::Mapa(vec![
        ("present".into(), Valor::Si(true)),
        ("percent".into(), Valor::Num(tanto)),
        ("charging".into(), Valor::Si(estado == "Charging" || estado == "Full")),
    ])
}

pub fn bateria(avisar: Box<dyn Fn(Valor) + Send>) -> bool {
    hilo("bateria", move || {
        let mut ultimo = String::new();
        loop {
            si_cambia(&*avisar, &mut ultimo, bateria_ahora());
            std::thread::sleep(Duration::from_secs(20));
        }
    })
}

// ── red ───────────────────────────────────────────────────────────

/// `{ online, kind = "wired" | "wifi" | "none", name, strength }`. Sale del
/// núcleo: la ruta por defecto dice por dónde se sale, y `/proc/net/wireless`
/// cómo de bien llega la wifi. El nombre es el de la red wifi si `iw` lo sabe
/// decir, y si no el de la interfaz.
fn red_ahora() -> Valor {
    let rutas = std::fs::read_to_string("/proc/net/route").unwrap_or_default();
    // Interfaz · destino · pasarela…: la ruta por defecto es la de destino 0.
    let interfaz = rutas.lines().skip(1).find_map(|l| {
        let mut c = l.split_whitespace();
        let (nombre, destino) = (c.next()?, c.next()?);
        (destino == "00000000").then(|| nombre.to_owned())
    });
    let Some(interfaz) = interfaz else {
        return Valor::Mapa(vec![("online".into(), Valor::Si(false)), ("kind".into(), Valor::Texto("none".into())), ("name".into(), Valor::Texto(String::new())), ("strength".into(), Valor::Num(0.0))]);
    };
    let wifi = std::path::Path::new(&format!("/sys/class/net/{interfaz}/wireless")).exists();
    let mut nombre = interfaz.clone();
    let mut fuerza = 1.0;
    if wifi {
        // «wlan0: 0000   54.  -56.  -256 …»: la calidad del enlace, sobre 70.
        if let Some(l) = std::fs::read_to_string("/proc/net/wireless").unwrap_or_default().lines().find(|l| l.trim_start().starts_with(&interfaz)) {
            if let Some(q) = l.split_whitespace().nth(2).and_then(|q| q.trim_end_matches('.').parse::<f64>().ok()) {
                // En décimas: la calidad baila sin parar y no es cosa de avisar cada tres segundos.
                fuerza = ((q / 70.0).clamp(0.0, 1.0) * 10.0).round() / 10.0;
            }
        }
        if let Some(s) = salida_de("iw", &["dev", &interfaz, "link"]) {
            if let Some(ssid) = s.lines().find_map(|l| l.trim().strip_prefix("SSID: ")) { nombre = ssid.to_owned() }
        }
    }
    Valor::Mapa(vec![
        ("online".into(), Valor::Si(true)),
        ("kind".into(), Valor::Texto(if wifi { "wifi" } else { "wired" }.into())),
        ("name".into(), Valor::Texto(nombre)),
        ("strength".into(), Valor::Num(fuerza)),
    ])
}

pub fn red(avisar: Box<dyn Fn(Valor) + Send>) -> bool {
    hilo("red", move || {
        let mut ultimo = String::new();
        loop {
            si_cambia(&*avisar, &mut ultimo, red_ahora());
            std::thread::sleep(Duration::from_secs(3));
        }
    })
}
