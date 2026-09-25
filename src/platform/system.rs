//! What Linux reports about itself with no compositor in between: audio,
//! battery and network. Each service is a thread that notifies only when something changes.
//!
//! The names and tables are the same on every system; here is only who
//! answers on Linux. On Windows they will be WASAPI, `GetSystemPowerStatus` and
//! `INetworkListManager`; on macOS, CoreAudio, IOKit and `SCNetworkReachability`.

use super::SysValue;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::Duration;

const SINK: &str = "@DEFAULT_AUDIO_SINK@";
const SOURCE: &str = "@DEFAULT_AUDIO_SOURCE@";

fn spawn_thread(name: &str, f: impl FnOnce() + Send + 'static) -> bool {
    std::thread::Builder::new().name(name.into()).spawn(f).is_ok()
}

fn output_of(program: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(program).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Notifies only if what has to be reported has changed.
fn notify_if_changed(dispatch: &dyn Fn(SysValue), last: &mut String, v: SysValue) {
    let fingerprint = format!("{v:?}");
    if fingerprint != *last {
        *last = fingerprint;
        dispatch(v);
    }
}

// ── audio ─────────────────────────────────────────────────────────

/// The sound devices there are, with the one in use marked. They come from
/// `wpctl status`, which lists them by sections:
///
/// ```text
///  ├─ Sinks:
///  │      52. TU106 HDMI                    [vol: 0.46]
///  │  *  105. CMF Buds Pro 2                [vol: 0.54]
///  ├─ Sources:
/// ```
///
/// A desktop sound panel needs to be able to choose where it plays and where
/// it listens; without this you can only move the volume of whatever was already set.
fn devices(section: &str) -> SysValue {
    let Some(text) = output_of("wpctl", &["status"]) else { return SysValue::List(Vec::new()) };
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let clean = line.trim_start_matches(|c: char| c == '│' || c == '├' || c == '└' || c == '─' || c.is_whitespace());
        if line.contains("Sinks:") || line.contains("Sources:") || line.contains("Filters:") || line.contains("Streams:") {
            // The sections repeat (Audio and Video): the first one counts.
            inside = line.contains(section) && out.is_empty();
            continue;
        }
        if !inside {
            continue;
        }
        let selected = clean.starts_with('*');
        let rest = clean.trim_start_matches('*').trim_start();
        let Some((number, name)) = rest.split_once('.') else { continue };
        let Ok(id) = number.trim().parse::<u32>() else { continue };
        // "CMF Buds Pro 2      [vol: 0.54]": the service already reports the volume.
        let name = name.split('[').next().unwrap_or(name).trim();
        if name.is_empty() {
            continue;
        }
        out.push(SysValue::Map(vec![
            ("id".into(), SysValue::Num(id as f64)),
            ("name".into(), SysValue::Text(name.to_owned())),
            ("default".into(), SysValue::Bool(selected)),
        ]));
    }
    SysValue::List(out)
}

/// `{ volume = 0.54, muted = false, input = 0.4, input_muted = true,
/// outputs = [...], inputs = [...] }`: the output and the input, which in a sound
/// panel always go together, and the devices there are for each. PipeWire,
/// through `wpctl`; and `pactl subscribe` to hear about changes without asking
/// every so often.
fn audio_now() -> Option<SysValue> {
    // "Volume: 0.54 [MUTED]"
    let read = |which: &str| -> Option<(f64, bool)> {
        let s = output_of("wpctl", &["get-volume", which])?;
        Some((s.split_whitespace().nth(1)?.parse().ok()?, s.contains("MUTED")))
    };
    let (volume, muted) = read(SINK)?;
    // With no microphone the service exists all the same: what is missing is the input.
    let (input, input_muted) = read(SOURCE).unwrap_or((0.0, true));
    Some(SysValue::Map(vec![
        ("volume".into(), SysValue::Num(volume)),
        ("muted".into(), SysValue::Bool(muted)),
        ("input".into(), SysValue::Num(input)),
        ("input_muted".into(), SysValue::Bool(input_muted)),
        ("outputs".into(), devices("Sinks:")),
        ("inputs".into(), devices("Sources:")),
    ]))
}

pub fn audio(dispatch: Box<dyn Fn(SysValue) + Send>) -> bool {
    let Some(now) = audio_now() else { return false };
    spawn_thread("audio", move || {
        let mut last = String::new();
        notify_if_changed(&*dispatch, &mut last, now);
        loop {
            let mut cmd = Command::new("pactl");
            cmd.arg("subscribe").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
            super::die_with_parent(&mut cmd);
            if let Ok(mut child) = cmd.spawn() {
                for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
                    // A change in an output, or in the server: which one is the default may have changed.
                    if line.contains("sink") || line.contains("server") {
                        if let Some(v) = audio_now() { notify_if_changed(&*dispatch, &mut last, v) }
                    }
                }
                let _ = child.wait();
            }
            // The audio server went down (or there is no `pactl`): ask every now and then until it comes back.
            std::thread::sleep(Duration::from_secs(2));
            if let Some(v) = audio_now() { notify_if_changed(&*dispatch, &mut last, v) }
        }
    })
}

pub fn audio_command(what: &str, args: &[SysValue]) -> Result<(), String> {
    let request = |a: &[&str]| output_of("wpctl", a).map(|_| ()).ok_or_else(|| "wpctl refused".to_string());
    match (what, args) {
        ("audio.volume", [SysValue::Num(v)]) => request(&["set-volume", SINK, &format!("{:.3}", v.clamp(0.0, 1.0))]),
        // A step, as a fraction of one: 0.05 goes up, -0.05 goes down. Never above 100 %.
        ("audio.step", [SysValue::Num(d)]) => request(&["set-volume", "-l", "1.0", SINK, &format!("{:.3}{}", d.abs(), if *d < 0.0 { "-" } else { "+" })]),
        ("audio.mute", []) => request(&["set-mute", SINK, "toggle"]),
        ("audio.mute", [SysValue::Bool(yes)]) => request(&["set-mute", SINK, if *yes { "1" } else { "0" }]),
        ("audio.input", [SysValue::Num(v)]) => request(&["set-volume", SOURCE, &format!("{:.3}", v.clamp(0.0, 1.0))]),
        ("audio.input_mute", []) => request(&["set-mute", SOURCE, "toggle"]),
        ("audio.input_mute", [SysValue::Bool(yes)]) => request(&["set-mute", SOURCE, if *yes { "1" } else { "0" }]),
        // Where it plays and where it listens: the number that comes in the list.
        ("audio.default", [SysValue::Num(id)]) => request(&["set-default", &format!("{}", *id as u32)]),
        _ => Err(format!("'{what}' is not asked like that: audio.volume(0..1), audio.step(±0.05), audio.mute([true|false]), audio.input(0..1), audio.input_mute([true|false]), audio.default(id)")),
    }
}

// ── the session ───────────────────────────────────────────────────

/// Lock, suspend, log out, reboot and power off. A desktop has to be able
/// to say goodbye, and a bar cannot do that on its own: `systemd` does it,
/// through `loginctl` and `systemctl`.
///
/// They sit behind their permission —`services: "session.*"`— and for a different
/// reason than the rest: **they cannot be undone**. The worst an `audio.volume`
/// can do is leave you deaf for a second; the worst this can do is close your
/// session with unsaved work. Whoever writes the scene decides whether its logic
/// may ask for it, and it is visible, written in the scene.
pub fn session_command(what: &str, _args: &[SysValue]) -> Result<(), String> {
    let run = |program: &str, args: &[&str]| -> Result<(), String> {
        Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("{program}: {e}"))
    };
    match what {
        "session.lock" => run("loginctl", &["lock-session"]),
        "session.suspend" => run("systemctl", &["suspend"]),
        "session.reboot" => run("systemctl", &["reboot"]),
        "session.poweroff" => run("systemctl", &["poweroff"]),
        // Logging out is ending **the session**, not killing the compositor: that
        // way whatever you had started separately goes too.
        "session.logout" => match std::env::var("XDG_SESSION_ID") {
            Ok(id) => run("loginctl", &["terminate-session", &id]),
            Err(_) => run("loginctl", &["terminate-user", &std::env::var("USER").unwrap_or_default()]),
        },
        _ => Err(format!("'{what}' is not one of: session.lock, session.suspend, session.logout, session.reboot, session.poweroff")),
    }
}

// ── battery ───────────────────────────────────────────────────────

/// `{ present = true, percent = 83, charging = false }`. A desktop machine answers
/// `{ present = false }`: the service exists, what is missing is a battery.
fn battery_now() -> SysValue {
    let read = |path: std::path::PathBuf| std::fs::read_to_string(path).ok().map(|s| s.trim().to_owned());
    let cell = std::fs::read_dir("/sys/class/power_supply").ok().and_then(|d| {
        d.filter_map(Result::ok).map(|f| f.path()).find(|p| read(p.join("type")).as_deref() == Some("Battery") && p.join("capacity").exists())
    });
    let Some(cell) = cell else { return SysValue::Map(vec![("present".into(), SysValue::Bool(false))]) };
    let percent = read(cell.join("capacity")).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let status = read(cell.join("status")).unwrap_or_default();
    SysValue::Map(vec![
        ("present".into(), SysValue::Bool(true)),
        ("percent".into(), SysValue::Num(percent)),
        ("charging".into(), SysValue::Bool(status == "Charging" || status == "Full")),
    ])
}

pub fn battery(dispatch: Box<dyn Fn(SysValue) + Send>) -> bool {
    spawn_thread("battery", move || {
        let mut last = String::new();
        loop {
            notify_if_changed(&*dispatch, &mut last, battery_now());
            std::thread::sleep(Duration::from_secs(20));
        }
    })
}

// ── brightness ────────────────────────────────────────────────────

/// The first backlight there is, the one `brightnessctl` drives by
/// default. A desktop machine answers `{ present = false }`: the service exists, what
/// is missing is a screen that can be dimmed.
fn backlight() -> Option<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir("/sys/class/backlight").ok()?.filter_map(Result::ok).map(|f| f.path()).filter(|p| p.join("max_brightness").exists()).collect();
    v.sort();
    v.into_iter().next()
}

fn brightness_now() -> SysValue {
    let read = |r: std::path::PathBuf| std::fs::read_to_string(r).ok().and_then(|s| s.trim().parse::<f64>().ok());
    let Some(p) = backlight() else { return SysValue::Map(vec![("present".into(), SysValue::Bool(false)), ("level".into(), SysValue::Num(0.0))]) };
    let (current, max) = (read(p.join("brightness")), read(p.join("max_brightness")));
    match (current, max) {
        (Some(a), Some(t)) if t > 0.0 => SysValue::Map(vec![("present".into(), SysValue::Bool(true)), ("level".into(), SysValue::Num(a / t))]),
        _ => SysValue::Map(vec![("present".into(), SysValue::Bool(false)), ("level".into(), SysValue::Num(0.0))]),
    }
}

pub fn brightness(dispatch: Box<dyn Fn(SysValue) + Send>) -> bool {
    spawn_thread("brightness", move || {
        let mut last = String::new();
        loop {
            notify_if_changed(&*dispatch, &mut last, brightness_now());
            // Checked often because a key on the keyboard can change it.
            std::thread::sleep(Duration::from_millis(700));
        }
    })
}

pub fn brightness_command(what: &str, args: &[SysValue]) -> Result<(), String> {
    let ("brightness.level", [SysValue::Num(v)]) = (what, args) else {
        return Err(format!("'{what}' is not asked like that: brightness.level(0..1)"));
    };
    if backlight().is_none() {
        return Err("there is no backlight on this machine".into());
    }
    // Through `brightnessctl`, which is who has the permission: writing to `sysfs`
    // requires being root, and pleamar is not, nor should it be.
    let percent = format!("{}%", (v.clamp(0.0, 1.0) * 100.0).round() as i32);
    output_of("brightnessctl", &["-q", "set", &percent]).map(|_| ()).ok_or_else(|| "brightnessctl refused".to_string())
}

// ── network ───────────────────────────────────────────────────────

/// `{ online, kind = "wired" | "wifi" | "none", name, strength }`. It comes from the
/// kernel: the default route says which way traffic leaves, and `/proc/net/wireless`
/// how well the wifi is getting through. The name is the wifi network's if `iw` can
/// tell, and otherwise the interface's.
fn network_now() -> SysValue {
    let routes = std::fs::read_to_string("/proc/net/route").unwrap_or_default();
    // Interface · destination · gateway…: the default route is the one with destination 0.
    let interface = routes.lines().skip(1).find_map(|l| {
        let mut c = l.split_whitespace();
        let (name, destination) = (c.next()?, c.next()?);
        (destination == "00000000").then(|| name.to_owned())
    });
    let Some(interface) = interface else {
        return SysValue::Map(vec![("online".into(), SysValue::Bool(false)), ("kind".into(), SysValue::Text("none".into())), ("name".into(), SysValue::Text(String::new())), ("strength".into(), SysValue::Num(0.0))]);
    };
    let wifi = std::path::Path::new(&format!("/sys/class/net/{interface}/wireless")).exists();
    let mut name = interface.clone();
    let mut strength = 1.0;
    if wifi {
        // "wlan0: 0000   54.  -56.  -256 …": the link quality, out of 70.
        if let Some(l) = std::fs::read_to_string("/proc/net/wireless").unwrap_or_default().lines().find(|l| l.trim_start().starts_with(&interface)) {
            if let Some(q) = l.split_whitespace().nth(2).and_then(|q| q.trim_end_matches('.').parse::<f64>().ok()) {
                // In tenths: the quality jitters nonstop and it is not worth notifying every three seconds.
                strength = ((q / 70.0).clamp(0.0, 1.0) * 10.0).round() / 10.0;
            }
        }
        if let Some(s) = output_of("iw", &["dev", &interface, "link"]) {
            if let Some(ssid) = s.lines().find_map(|l| l.trim().strip_prefix("SSID: ")) { name = ssid.to_owned() }
        }
    }
    SysValue::Map(vec![
        ("online".into(), SysValue::Bool(true)),
        ("kind".into(), SysValue::Text(if wifi { "wifi" } else { "wired" }.into())),
        ("name".into(), SysValue::Text(name)),
        ("strength".into(), SysValue::Num(strength)),
    ])
}

pub fn network(dispatch: Box<dyn Fn(SysValue) + Send>) -> bool {
    spawn_thread("network", move || {
        let mut last = String::new();
        loop {
            notify_if_changed(&*dispatch, &mut last, network_now());
            std::thread::sleep(Duration::from_secs(3));
        }
    })
}
