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
const ENTRADA: &str = "@DEFAULT_AUDIO_SOURCE@";

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

/// Los aparatos de sonido que hay, con el que está puesto marcado. Salen de
/// `wpctl status`, que los lista por secciones:
///
/// ```text
///  ├─ Sinks:
///  │      52. TU106 HDMI                    [vol: 0.46]
///  │  *  105. CMF Buds Pro 2                [vol: 0.54]
///  ├─ Sources:
/// ```
///
/// Un panel de sonido de escritorio necesita poder elegir por dónde suena y por
/// dónde escucha; sin esto solo se puede mover el volumen de lo que ya había.
fn aparatos(seccion: &str) -> Valor {
    let Some(texto) = salida_de("wpctl", &["status"]) else { return Valor::Lista(Vec::new()) };
    let mut fuera = Vec::new();
    let mut dentro = false;
    for linea in texto.lines() {
        let limpia = linea.trim_start_matches(|c: char| c == '│' || c == '├' || c == '└' || c == '─' || c.is_whitespace());
        if linea.contains("Sinks:") || linea.contains("Sources:") || linea.contains("Filters:") || linea.contains("Streams:") {
            // Las secciones se repiten (Audio y Video): vale la primera.
            dentro = linea.contains(seccion) && fuera.is_empty();
            continue;
        }
        if !dentro {
            continue;
        }
        let puesto = limpia.starts_with('*');
        let resto = limpia.trim_start_matches('*').trim_start();
        let Some((numero, nombre)) = resto.split_once('.') else { continue };
        let Ok(id) = numero.trim().parse::<u32>() else { continue };
        // «CMF Buds Pro 2      [vol: 0.54]»: el volumen ya lo dice el servicio.
        let nombre = nombre.split('[').next().unwrap_or(nombre).trim();
        if nombre.is_empty() {
            continue;
        }
        fuera.push(Valor::Mapa(vec![
            ("id".into(), Valor::Num(id as f64)),
            ("name".into(), Valor::Texto(nombre.to_owned())),
            ("default".into(), Valor::Si(puesto)),
        ]));
    }
    Valor::Lista(fuera)
}

/// `{ volume = 0.54, muted = false, input = 0.4, input_muted = true,
/// outputs = [...], inputs = [...] }`: la salida y la entrada, que en un panel
/// de sonido van siempre juntas, y los aparatos que hay de cada una. PipeWire,
/// por `wpctl`; y `pactl subscribe` para enterarse de los cambios sin preguntar
/// a cada rato.
fn audio_ahora() -> Option<Valor> {
    // «Volume: 0.54 [MUTED]»
    let leer = |que: &str| -> Option<(f64, bool)> {
        let s = salida_de("wpctl", &["get-volume", que])?;
        Some((s.split_whitespace().nth(1)?.parse().ok()?, s.contains("MUTED")))
    };
    let (volumen, mudo) = leer(SALIDA)?;
    // Sin micrófono el servicio existe igual: lo que no hay es entrada.
    let (entrada, entrada_muda) = leer(ENTRADA).unwrap_or((0.0, true));
    Some(Valor::Mapa(vec![
        ("volume".into(), Valor::Num(volumen)),
        ("muted".into(), Valor::Si(mudo)),
        ("input".into(), Valor::Num(entrada)),
        ("input_muted".into(), Valor::Si(entrada_muda)),
        ("outputs".into(), aparatos("Sinks:")),
        ("inputs".into(), aparatos("Sources:")),
    ]))
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
        ("audio.input", [Valor::Num(v)]) => pedir(&["set-volume", ENTRADA, &format!("{:.3}", v.clamp(0.0, 1.0))]),
        ("audio.input_mute", []) => pedir(&["set-mute", ENTRADA, "toggle"]),
        ("audio.input_mute", [Valor::Si(si)]) => pedir(&["set-mute", ENTRADA, if *si { "1" } else { "0" }]),
        // Por dónde suena y por dónde escucha: el número que trae la lista.
        ("audio.default", [Valor::Num(id)]) => pedir(&["set-default", &format!("{}", *id as u32)]),
        _ => Err(format!("'{que}' is not asked like that: audio.volume(0..1), audio.step(±0.05), audio.mute([true|false]), audio.input(0..1), audio.input_mute([true|false]), audio.default(id)")),
    }
}

// ── la sesión ─────────────────────────────────────────────────────

/// Bloquear, suspender, cerrar sesión, reiniciar y apagar. Un escritorio tiene
/// que poder despedirse, y eso no lo sabe hacer una barra sola: lo hace
/// `systemd`, por `loginctl` y `systemctl`.
///
/// Van detrás de su permiso —`services: "session.*"`— y por una razón distinta
/// que las demás: **no se deshacen**. Lo peor que puede hacer un `audio.volume`
/// es dejarte sordo un segundo; lo peor que puede hacer esto es cerrarte la
/// sesión con cosas sin guardar. Quien escribe la escena decide si su lógica
/// puede pedirlo, y se ve escrito en la escena.
pub fn sesion_orden(que: &str, _args: &[Valor]) -> Result<(), String> {
    let hacer = |programa: &str, args: &[&str]| -> Result<(), String> {
        Command::new(programa)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("{programa}: {e}"))
    };
    match que {
        "session.lock" => hacer("loginctl", &["lock-session"]),
        "session.suspend" => hacer("systemctl", &["suspend"]),
        "session.reboot" => hacer("systemctl", &["reboot"]),
        "session.poweroff" => hacer("systemctl", &["poweroff"]),
        // Cerrar sesión es terminar **la sesión**, no matar el compositor: así
        // se va también lo que hubieras arrancado aparte.
        "session.logout" => match std::env::var("XDG_SESSION_ID") {
            Ok(id) => hacer("loginctl", &["terminate-session", &id]),
            Err(_) => hacer("loginctl", &["terminate-user", &std::env::var("USER").unwrap_or_default()]),
        },
        _ => Err(format!("'{que}' is not one of: session.lock, session.suspend, session.logout, session.reboot, session.poweroff")),
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

// ── brillo ────────────────────────────────────────────────────────

/// La primera retroiluminación que haya, la que manda `brightnessctl` por
/// defecto. Un sobremesa contesta `{ present = false }`: el servicio existe, lo
/// que no hay es pantalla que se pueda atenuar.
fn pantalla() -> Option<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir("/sys/class/backlight").ok()?.filter_map(Result::ok).map(|f| f.path()).filter(|p| p.join("max_brightness").exists()).collect();
    v.sort();
    v.into_iter().next()
}

fn brillo_ahora() -> Valor {
    let leer = |r: std::path::PathBuf| std::fs::read_to_string(r).ok().and_then(|s| s.trim().parse::<f64>().ok());
    let Some(p) = pantalla() else { return Valor::Mapa(vec![("present".into(), Valor::Si(false)), ("level".into(), Valor::Num(0.0))]) };
    let (ahora, tope) = (leer(p.join("brightness")), leer(p.join("max_brightness")));
    match (ahora, tope) {
        (Some(a), Some(t)) if t > 0.0 => Valor::Mapa(vec![("present".into(), Valor::Si(true)), ("level".into(), Valor::Num(a / t))]),
        _ => Valor::Mapa(vec![("present".into(), Valor::Si(false)), ("level".into(), Valor::Num(0.0))]),
    }
}

pub fn brillo(avisar: Box<dyn Fn(Valor) + Send>) -> bool {
    hilo("brillo", move || {
        let mut ultimo = String::new();
        loop {
            si_cambia(&*avisar, &mut ultimo, brillo_ahora());
            // Se mira a menudo porque lo puede cambiar una tecla del teclado.
            std::thread::sleep(Duration::from_millis(700));
        }
    })
}

pub fn brillo_orden(que: &str, args: &[Valor]) -> Result<(), String> {
    let ("brightness.level", [Valor::Num(v)]) = (que, args) else {
        return Err(format!("'{que}' is not asked like that: brightness.level(0..1)"));
    };
    if pantalla().is_none() {
        return Err("there is no backlight on this machine".into());
    }
    // Por `brightnessctl`, que es quien tiene el permiso: escribir en `sysfs`
    // pide ser root, y pleamar no lo es ni debe serlo.
    let tanto = format!("{}%", (v.clamp(0.0, 1.0) * 100.0).round() as i32);
    salida_de("brightnessctl", &["-q", "set", &tanto]).map(|_| ()).ok_or_else(|| "brightnessctl refused".to_string())
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
