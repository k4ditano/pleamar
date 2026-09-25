//! What is playing, through MPRIS: the D-Bus agreement that players and browsers
//! follow on Linux. On Windows it will be SMTC; on macOS, MediaRemote.
//!
//! `{ playing, title, artist, album, player }`. If there are several, the one
//! that is playing is reported; if none is playing, the first. With no players, `player = ""`.

use super::SysValue;
use std::collections::HashMap;
use zbus::blocking::{fdo::DBusProxy, Connection, MessageIterator, Proxy};
use zbus::zvariant::OwnedValue;
use zbus::MatchRule;

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

fn players(c: &Connection) -> Vec<String> {
    let Ok(bus) = DBusProxy::new(c) else { return Vec::new() };
    let mut names: Vec<String> = bus.list_names().unwrap_or_default().into_iter().map(|n| n.to_string()).filter(|n| n.starts_with(PREFIX)).collect();
    names.sort();
    names
}

fn player(c: &Connection, name: &str) -> Option<Proxy<'static>> {
    Proxy::new(c, name.to_owned(), PATH, PLAYER).ok()
}

/// The one that is playing, or the first.
fn active_player(c: &Connection) -> Option<(String, Proxy<'static>, bool)> {
    let mut first = None;
    for n in players(c) {
        let Some(p) = player(c, &n) else { continue };
        let playing = p.get_property::<String>("PlaybackStatus").is_ok_and(|s| s == "Playing");
        if playing { return Some((n, p, true)) }
        first.get_or_insert((n, p, false));
    }
    first
}

fn text(v: &OwnedValue) -> String {
    if let Ok(s) = <&str>::try_from(v) { return s.to_owned() }
    // `xesam:artist` is a list: the artists, with commas.
    <Vec<String>>::try_from(v.clone()).map(|l| l.join(", ")).unwrap_or_default()
}

fn now(c: &Connection) -> SysValue {
    let field = |k: &str, v: SysValue| (k.to_owned(), v);
    let Some((name, p, playing)) = active_player(c) else {
        return SysValue::Map(vec![field("playing", SysValue::Bool(false)), field("title", SysValue::Text(String::new())), field("artist", SysValue::Text(String::new())), field("album", SysValue::Text(String::new())), field("player", SysValue::Text(String::new()))]);
    };
    let metadata: HashMap<String, OwnedValue> = p.get_property("Metadata").unwrap_or_default();
    let from_metadata = |k: &str| SysValue::Text(metadata.get(k).map(text).unwrap_or_default());
    SysValue::Map(vec![
        field("playing", SysValue::Bool(playing)),
        field("title", from_metadata("xesam:title")),
        field("artist", from_metadata("xesam:artist")),
        field("album", from_metadata("xesam:album")),
        field("player", SysValue::Text(name.trim_start_matches(PREFIX).split('.').next().unwrap_or_default().to_owned())),
    ])
}

pub fn service(dispatch: Box<dyn Fn(SysValue) + Send>) -> bool {
    let Ok(c) = Connection::session() else { return false };
    std::thread::Builder::new().name("media".into()).spawn(move || {
        let mut last = String::new();
        let mut report = |c: &Connection| {
            let v = now(c);
            let fingerprint = format!("{v:?}");
            if fingerprint != last { last = fingerprint; dispatch(v) }
        };
        report(&c);
        // Two things matter: a player changing something, and one appearing or going away.
        let rules = [
            MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.freedesktop.DBus.Properties").and_then(|b| b.member("PropertiesChanged")).and_then(|b| b.path(PATH)).map(|b| b.build()),
            MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.freedesktop.DBus").and_then(|b| b.member("NameOwnerChanged")).and_then(|b| b.arg0ns(PREFIX.trim_end_matches('.'))).map(|b| b.build()),
        ];
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        for rule in rules.into_iter().flatten() {
            let (c, tx) = (c.clone(), tx.clone());
            std::thread::spawn(move || {
                let Ok(messages) = MessageIterator::for_match_rule(rule, &c, Some(16)) else { return };
                for _ in messages { if tx.send(()).is_err() { break } }
            });
        }
        drop(tx);
        while rx.recv().is_ok() {
            // A song change is several signals in a row: it is reported once.
            std::thread::sleep(std::time::Duration::from_millis(40));
            while rx.try_recv().is_ok() {}
            report(&c);
        }
    }).is_ok()
}

/// `media.toggle`, `media.next`, `media.previous`: to the one `now` is reporting.
pub fn command(what: &str, _: &[SysValue]) -> Result<(), String> {
    let method = match what {
        "media.toggle" => "PlayPause",
        "media.next" => "Next",
        "media.previous" => "Previous",
        _ => return Err(format!("'{what}' does not exist: media.toggle, media.next, media.previous")),
    };
    let c = Connection::session().map_err(|e| e.to_string())?;
    let (_, p, _) = active_player(&c).ok_or("there is no player open")?;
    p.call_method(method, &()).map(|_| ()).map_err(|e| e.to_string())
}
