//! What is playing, through MPRIS: the D-Bus agreement that players and browsers
//! follow on Linux. On Windows it will be SMTC; on macOS, MediaRemote.
//!
//! `{ playing, title, artist, album, length, position, rate, art, player, players }`. If there are
//! several, the one chosen with `media.choose` is reported, while it is there; otherwise the one
//! that is playing; if none is, the one that played last; and before anything has played, the
//! first. With no players, `player = ""`. `players` is all of them, `{ id, name, playing,
//! chosen }`, to choose from.
//!
//! MPRIS does not signal `Position` as it moves, so it is read whenever something
//! else is reported, and on `Seeked`. Between reports the scene carries it on with
//! its own clock, at `rate`, while `playing`: nothing here polls.

use super::SysValue;
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use zbus::blocking::{fdo::DBusProxy, Connection, MessageIterator, Proxy};
use zbus::zvariant::OwnedValue;
use zbus::MatchRule;

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

/// The player `media.choose` pinned, by its bus name without the prefix; "" is none. It is the
/// process's, like the default output is the system's: every scene reports the same one.
static CHOSEN: Mutex<String> = Mutex::new(String::new());
/// The last player seen playing, by its whole bus name; "" is none yet. A pause leaves `media` on
/// it: without this, nothing playing meant the first on the bus, and play after a pause went
/// there —a phone through KDE Connect sorts before most players—.
static LAST: Mutex<String> = Mutex::new(String::new());
/// The running services, to report again at once when the choice changes.
static WAKE: Mutex<Vec<Sender<()>>> = Mutex::new(Vec::new());

fn chosen() -> String {
    CHOSEN.lock().map(|c| c.clone()).unwrap_or_default()
}

/// Every player on the bus but `playerctld`, which is no player of its own: it mirrors whichever
/// one was active last, and would show up twice.
fn players(c: &Connection) -> Vec<String> {
    let Ok(bus) = DBusProxy::new(c) else { return Vec::new() };
    let mut names: Vec<String> = bus.list_names().unwrap_or_default().into_iter().map(|n| n.to_string()).filter(|n| n.starts_with(PREFIX) && n[PREFIX.len()..] != *"playerctld").collect();
    names.sort();
    names
}

fn player(c: &Connection, name: &str) -> Option<Proxy<'static>> {
    Proxy::new(c, name.to_owned(), PATH, PLAYER).ok()
}

fn is_playing(p: &Proxy) -> bool {
    p.get_property::<String>("PlaybackStatus").is_ok_and(|s| s == "Playing")
}

/// What a pause falls back to: see `LAST`.
fn remember(name: &str) {
    if let Ok(mut l) = LAST.lock() { name.clone_into(&mut l) }
}

/// The chosen one if it is there; otherwise the one that is playing, the one that played last, or
/// the first.
fn active_player(c: &Connection) -> Option<(String, Proxy<'static>, bool)> {
    let pinned = chosen();
    let names = players(c);
    if !pinned.is_empty() {
        if let Some(n) = names.iter().find(|n| n.strip_prefix(PREFIX) == Some(pinned.as_str())) {
            if let Some(p) = player(c, n) {
                let playing = is_playing(&p);
                if playing { remember(n) }
                return Some((n.clone(), p, playing));
            }
        }
    }
    let last = LAST.lock().map(|l| l.clone()).unwrap_or_default();
    let (mut first, mut previous) = (None, None);
    for n in names {
        let Some(p) = player(c, &n) else { continue };
        if is_playing(&p) {
            remember(&n);
            return Some((n, p, true));
        }
        if n == last { previous = Some((n, p, false)) } else { first.get_or_insert((n, p, false)); }
    }
    previous.or(first)
}

/// Every player, to choose from: `id` is what `media.choose` takes, `name` what the player
/// calls itself (`Identity`), which tells two of the same program apart where `player` cannot
/// —two phones through KDE Connect are both `kdeconnect`—.
fn all_players(c: &Connection) -> SysValue {
    let pinned = chosen();
    SysValue::List(players(c).into_iter().map(|n| {
        let id = n.trim_start_matches(PREFIX).to_owned();
        let name = Proxy::new(c, n.clone(), PATH, "org.mpris.MediaPlayer2").ok().and_then(|r| r.get_property::<String>("Identity").ok()).unwrap_or_else(|| id.clone());
        let playing = player(c, &n).is_some_and(|p| is_playing(&p));
        SysValue::Map(vec![("id".into(), SysValue::Text(id.clone())), ("name".into(), SysValue::Text(name)), ("playing".into(), SysValue::Bool(playing)), ("chosen".into(), SysValue::Bool(id == pinned))])
    }).collect())
}

fn text(v: &OwnedValue) -> String {
    if let Ok(s) = <&str>::try_from(v) { return s.to_owned() }
    // `xesam:artist` is a list: the artists, with commas.
    <Vec<String>>::try_from(v.clone()).map(|l| l.join(", ")).unwrap_or_default()
}

/// `mpris:length` is in microseconds, and the spec says `x`, but some players send `t`.
fn seconds(v: &OwnedValue) -> f64 {
    let us = i64::try_from(v).ok().or_else(|| u64::try_from(v).ok().map(|n| n as i64)).unwrap_or(0);
    us.max(0) as f64 / 1e6
}

/// `Position` is not in the metadata: it is a property of its own, and the one that
/// never says it changed. A player that does not know it answers with an error: 0.
fn position(p: &Proxy) -> f64 {
    p.get_property::<OwnedValue>("Position").map(|v| seconds(&v)).unwrap_or(0.0)
}

fn now(c: &Connection) -> SysValue {
    let field = |k: &str, v: SysValue| (k.to_owned(), v);
    let Some((name, p, playing)) = active_player(c) else {
        return SysValue::Map(vec![field("playing", SysValue::Bool(false)), field("title", SysValue::Text(String::new())), field("artist", SysValue::Text(String::new())), field("album", SysValue::Text(String::new())), field("length", SysValue::Num(0.0)), field("position", SysValue::Num(0.0)), field("rate", SysValue::Num(1.0)), field("art", SysValue::Text(String::new())), field("player", SysValue::Text(String::new())), field("players", SysValue::List(Vec::new()))]);
    };
    let metadata: HashMap<String, OwnedValue> = p.get_property("Metadata").unwrap_or_default();
    let from_metadata = |k: &str| SysValue::Text(metadata.get(k).map(text).unwrap_or_default());
    SysValue::Map(vec![
        field("playing", SysValue::Bool(playing)),
        field("title", from_metadata("xesam:title")),
        field("artist", from_metadata("xesam:artist")),
        field("album", from_metadata("xesam:album")),
        field("length", SysValue::Num(metadata.get("mpris:length").map(seconds).unwrap_or(0.0))),
        field("position", SysValue::Num(position(&p))),
        // The spec's default; a player that cannot change it may leave it out.
        field("rate", SysValue::Num(p.get_property::<f64>("Rate").unwrap_or(1.0))),
        field("art", from_metadata("mpris:artUrl")),
        field("player", SysValue::Text(name.trim_start_matches(PREFIX).split('.').next().unwrap_or_default().to_owned())),
        field("players", all_players(c)),
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
        // Three things matter: a player changing something, one jumping to another
        // point of the song (`Seeked`, the only time the position says it moved),
        // and one appearing or going away.
        let rules = [
            MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.freedesktop.DBus.Properties").and_then(|b| b.member("PropertiesChanged")).and_then(|b| b.path(PATH)).map(|b| b.build()),
            MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(PLAYER).and_then(|b| b.member("Seeked")).and_then(|b| b.path(PATH)).map(|b| b.build()),
            MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.freedesktop.DBus").and_then(|b| b.member("NameOwnerChanged")).and_then(|b| b.arg0ns(PREFIX.trim_end_matches('.'))).map(|b| b.build()),
        ];
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        if let Ok(mut w) = WAKE.lock() { w.push(tx.clone()) }
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
/// `media.choose(id)`: report that one, and command it, while it is there; `""` goes back to
/// the one playing, else the one that played last, else the first.
pub fn command(what: &str, args: &[SysValue]) -> Result<(), String> {
    let method = match (what, args) {
        ("media.toggle", _) => "PlayPause",
        ("media.next", _) => "Next",
        ("media.previous", _) => "Previous",
        ("media.choose", [SysValue::Text(id)]) => {
            if let Ok(mut c) = CHOSEN.lock() { c.clone_from(id) }
            // A service whose thread has gone is dropped here.
            if let Ok(mut w) = WAKE.lock() { w.retain(|tx| tx.send(()).is_ok()) }
            return Ok(());
        }
        ("media.choose", _) => return Err("media.choose takes the `id` of one of `players`, or \"\" to let it choose".into()),
        _ => return Err(format!("'{what}' does not exist: media.toggle, media.next, media.previous, media.choose(id)")),
    };
    let c = Connection::session().map_err(|e| e.to_string())?;
    let (_, p, _) = active_player(&c).ok_or("there is no player open")?;
    p.call_method(method, &()).map(|_| ()).map_err(|e| e.to_string())
}
