//! Lo que suena, por MPRIS: el acuerdo de D-Bus que cumplen los reproductores y
//! los navegadores en Linux. En Windows será SMTC; en macOS, MediaRemote.
//!
//! `{ playing, title, artist, album, player }`. Si hay varios, se cuenta el que
//! esté sonando; si ninguno suena, el primero. Sin reproductores, `player = ""`.

use super::Valor;
use std::collections::HashMap;
use zbus::blocking::{fdo::DBusProxy, Connection, MessageIterator, Proxy};
use zbus::zvariant::OwnedValue;
use zbus::MatchRule;

const PREFIJO: &str = "org.mpris.MediaPlayer2.";
const RUTA: &str = "/org/mpris/MediaPlayer2";
const REPRODUCTOR: &str = "org.mpris.MediaPlayer2.Player";

fn reproductores(c: &Connection) -> Vec<String> {
    let Ok(bus) = DBusProxy::new(c) else { return Vec::new() };
    let mut nombres: Vec<String> = bus.list_names().unwrap_or_default().into_iter().map(|n| n.to_string()).filter(|n| n.starts_with(PREFIJO)).collect();
    nombres.sort();
    nombres
}

fn de(c: &Connection, nombre: &str) -> Option<Proxy<'static>> {
    Proxy::new(c, nombre.to_owned(), RUTA, REPRODUCTOR).ok()
}

/// El que suena, o el primero.
fn elegido(c: &Connection) -> Option<(String, Proxy<'static>, bool)> {
    let mut primero = None;
    for n in reproductores(c) {
        let Some(p) = de(c, &n) else { continue };
        let sonando = p.get_property::<String>("PlaybackStatus").is_ok_and(|s| s == "Playing");
        if sonando { return Some((n, p, true)) }
        primero.get_or_insert((n, p, false));
    }
    primero
}

fn texto(v: &OwnedValue) -> String {
    if let Ok(s) = <&str>::try_from(v) { return s.to_owned() }
    // `xesam:artist` es una lista: los artistas, con comas.
    <Vec<String>>::try_from(v.clone()).map(|l| l.join(", ")).unwrap_or_default()
}

fn ahora(c: &Connection) -> Valor {
    let campo = |k: &str, v: Valor| (k.to_owned(), v);
    let Some((nombre, p, sonando)) = elegido(c) else {
        return Valor::Mapa(vec![campo("playing", Valor::Si(false)), campo("title", Valor::Texto(String::new())), campo("artist", Valor::Texto(String::new())), campo("album", Valor::Texto(String::new())), campo("player", Valor::Texto(String::new()))]);
    };
    let datos: HashMap<String, OwnedValue> = p.get_property("Metadata").unwrap_or_default();
    let de_datos = |k: &str| Valor::Texto(datos.get(k).map(texto).unwrap_or_default());
    Valor::Mapa(vec![
        campo("playing", Valor::Si(sonando)),
        campo("title", de_datos("xesam:title")),
        campo("artist", de_datos("xesam:artist")),
        campo("album", de_datos("xesam:album")),
        campo("player", Valor::Texto(nombre.trim_start_matches(PREFIJO).split('.').next().unwrap_or_default().to_owned())),
    ])
}

pub fn servicio(avisar: Box<dyn Fn(Valor) + Send>) -> bool {
    let Ok(c) = Connection::session() else { return false };
    std::thread::Builder::new().name("media".into()).spawn(move || {
        let mut ultimo = String::new();
        let mut contar = |c: &Connection| {
            let v = ahora(c);
            let huella = format!("{v:?}");
            if huella != ultimo { ultimo = huella; avisar(v) }
        };
        contar(&c);
        // Dos cosas interesan: que un reproductor cambie algo, y que aparezca o se vaya uno.
        let reglas = [
            MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.freedesktop.DBus.Properties").and_then(|b| b.member("PropertiesChanged")).and_then(|b| b.path(RUTA)).map(|b| b.build()),
            MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.freedesktop.DBus").and_then(|b| b.member("NameOwnerChanged")).and_then(|b| b.arg0ns(PREFIJO.trim_end_matches('.'))).map(|b| b.build()),
        ];
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        for regla in reglas.into_iter().flatten() {
            let (c, tx) = (c.clone(), tx.clone());
            std::thread::spawn(move || {
                let Ok(mensajes) = MessageIterator::for_match_rule(regla, &c, Some(16)) else { return };
                for _ in mensajes { if tx.send(()).is_err() { break } }
            });
        }
        drop(tx);
        while rx.recv().is_ok() {
            // Un cambio de canción son varias señales seguidas: se cuenta una vez.
            std::thread::sleep(std::time::Duration::from_millis(40));
            while rx.try_recv().is_ok() {}
            contar(&c);
        }
    }).is_ok()
}

/// `media.toggle`, `media.next`, `media.previous`: al que `ahora` está contando.
pub fn orden(que: &str, _: &[Valor]) -> Result<(), String> {
    let metodo = match que {
        "media.toggle" => "PlayPause",
        "media.next" => "Next",
        "media.previous" => "Previous",
        _ => return Err(format!("'{que}' does not exist: media.toggle, media.next, media.previous")),
    };
    let c = Connection::session().map_err(|e| e.to_string())?;
    let (_, p, _) = elegido(&c).ok_or("no hay ningún reproductor abierto")?;
    p.call_method(metodo, &()).map(|_| ()).map_err(|e| e.to_string())
}
