//! Game controllers: the kernel's joystick devices, `/dev/input/js0`…, read
//! as they are. Nothing is spawned and no library is asked: each one is a
//! file that gives eight bytes for each stick that moves or button that goes
//! down, and three questions tell its name and which stick and which button
//! each number is.
//!
//! What is kept of each: its name, its sticks from −1 to 1, its triggers from
//! 0 to 1, and its buttons, by the names every pad shares —`a`, `b`, `x`, `y`,
//! `lb`, `rb`, `back`, `start`, `guide`, `ls`, `rs`, and the cross as `up`,
//! `down`, `left`, `right`—. A logic that plays asks how they are at each
//! frame (`pad(1)`); one that only wants to know of a press is told of each.

use super::SysValue;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How many are looked for: js0 to js7.
const SLOTS: usize = 8;

#[derive(Clone, Default)]
struct Pad {
    name: String,
    /// By the kernel's own code for each (ABS_X is 0, BTN_SOUTH is 0x130…), in the order the device numbers them.
    axis_codes: Vec<u8>,
    button_codes: Vec<u16>,
    axes: Vec<f32>,
    buttons: Vec<bool>,
}

struct Pads {
    /// By slot: `None` where there is none plugged in.
    list: Vec<Option<Pad>>,
    /// Whoever wants to hear of a press, a release, one arriving or one going.
    listeners: Vec<Arc<dyn Fn(SysValue) + Send + Sync>>,
    watching: bool,
}

static PADS: Mutex<Pads> = Mutex::new(Pads { list: Vec::new(), listeners: Vec::new(), watching: false });

const AXES: &[(u8, &str)] = &[(0x00, "lx"), (0x01, "ly"), (0x02, "lt"), (0x03, "rx"), (0x04, "ry"), (0x05, "rt"), (0x10, "dx"), (0x11, "dy")];
const BUTTONS: &[(u16, &str)] = &[
    (0x130, "a"), (0x131, "b"), (0x133, "x"), (0x134, "y"), (0x136, "lb"), (0x137, "rb"), (0x138, "lt_button"), (0x139, "rt_button"),
    (0x13a, "back"), (0x13b, "start"), (0x13c, "guide"), (0x13d, "ls"), (0x13e, "rs"), (0x220, "up"), (0x221, "down"), (0x222, "left"), (0x223, "right"),
];

impl Pad {
    /// As a logic reads it: `{ name, lx, ly, rx, ry, lt, rt, a, b, … }`.
    fn told(&self, slot: usize) -> SysValue {
        let mut m: Vec<(String, SysValue)> = vec![("id".into(), SysValue::Num(slot as f64 + 1.0)), ("name".into(), SysValue::Text(self.name.clone()))];
        let axis = |code: u8| self.axis_codes.iter().position(|c| *c == code).and_then(|k| self.axes.get(k)).copied();
        for (code, name) in AXES {
            let v = match (axis(*code), *name) {
                // A trigger rests at −1 and ends at 1: from 0 to 1 it reads better.
                (Some(v), "lt" | "rt") => (v + 1.0) / 2.0,
                (Some(v), _) => v,
                (None, _) => 0.0,
            };
            m.push(((*name).into(), SysValue::Num((v as f64 * 1000.0).round() / 1000.0)));
        }
        let mut down: Vec<(&str, bool)> = BUTTONS.iter().map(|(code, name)| (*name, self.button_codes.iter().position(|c| c == code).is_some_and(|k| self.buttons.get(k).copied().unwrap_or(false)))).collect();
        // The cross is a stick on most pads: it reads as four buttons all the same.
        for (name, code, side) in [("left", 0x10, -1.0), ("right", 0x10, 1.0), ("up", 0x11, -1.0), ("down", 0x11, 1.0)] {
            if axis(code).is_some_and(|v| v * side > 0.5) {
                down.iter_mut().filter(|d| d.0 == name).for_each(|d| d.1 = true);
            }
        }
        m.extend(down.into_iter().map(|(n, v)| (n.to_owned(), SysValue::Bool(v))));
        // And every button by its number, for a pad with more than those.
        m.push(("buttons".into(), SysValue::List(self.buttons.iter().map(|b| SysValue::Bool(*b)).collect())));
        m.push(("axes".into(), SysValue::List(self.axes.iter().map(|a| SysValue::Num(*a as f64)).collect())));
        SysValue::Map(m)
    }
}

/// How pad number `n` (from 1) is right now, or nothing if there is none.
pub fn state(n: usize) -> Option<SysValue> {
    watch();
    let p = PADS.lock().unwrap();
    p.list.get(n.checked_sub(1)?)?.as_ref().map(|pad| pad.told(n - 1))
}

/// Every pad plugged in, as `sys.watch("gamepad", …)` hears them: told now,
/// and again whenever a button goes down or up, or one arrives or goes.
pub fn service(notify: Box<dyn Fn(SysValue) + Send>) -> bool {
    watch();
    // (Only ever called from one thread at a time: the lock is what makes it shareable.)
    let notify = Mutex::new(notify);
    let listener: Arc<dyn Fn(SysValue) + Send + Sync> = Arc::new(move |v| (notify.lock().unwrap())(v));
    let now = {
        let mut p = PADS.lock().unwrap();
        p.listeners.push(listener.clone());
        all(&p)
    };
    listener(now);
    true
}

fn all(p: &Pads) -> SysValue {
    SysValue::List(p.list.iter().enumerate().filter_map(|(k, pad)| pad.as_ref().map(|pad| pad.told(k))).collect())
}

fn tell() {
    let (listeners, now) = {
        let p = PADS.lock().unwrap();
        (p.listeners.clone(), all(&p))
    };
    for l in listeners {
        l(now.clone());
    }
}

/// Starts looking for pads, the first time anyone asks: one thread that
/// looks every two seconds for the ones that arrive, and one for each that is there.
fn watch() {
    {
        let mut p = PADS.lock().unwrap();
        if std::mem::replace(&mut p.watching, true) {
            return;
        }
        p.list = vec![None; SLOTS];
    }
    // The ones already there are known before whoever asked reads them.
    look();
    let _ = std::thread::Builder::new().name("gamepads".into()).spawn(|| loop {
        std::thread::sleep(Duration::from_secs(2));
        look();
    });
}

fn folder() -> String {
    // (`PLEAMAR_PADS`: another folder with js0…, to rehearse without a pad.)
    std::env::var("PLEAMAR_PADS").unwrap_or_else(|_| "/dev/input".into())
}

fn look() {
    for slot in 0..SLOTS {
        if PADS.lock().unwrap().list[slot].is_some() {
            continue;
        }
        let Ok(file) = std::fs::File::open(format!("{}/js{slot}", folder())) else { continue };
        let pad = describe(&file);
        PADS.lock().unwrap().list[slot] = Some(pad);
        tell();
        let _ = std::thread::Builder::new().name(format!("gamepad {slot}")).spawn(move || {
            read(slot, file);
            PADS.lock().unwrap().list[slot] = None;
            tell();
        });
    }
}

/// Its name and which stick and button each of its numbers is, asked of the kernel.
fn describe(file: &std::fs::File) -> Pad {
    // _IOR('j', nr, size): what joystick.h calls JSIOCGAXES, JSIOCGBUTTONS,
    // JSIOCGNAME, JSIOCGAXMAP and JSIOCGBTNMAP.
    let ask = |nr: u64, size: usize| (2u64 << 30) | ((size as u64) << 16) | (0x6a << 8) | nr;
    let fd = file.as_raw_fd();
    let (mut axes, mut buttons) = (0u8, 0u8);
    let mut name = [0u8; 128];
    let mut axis_map = [0u8; 64];
    let mut button_map = [0u16; 512];
    // SAFETY: each call hands the kernel a buffer of exactly the size its request names.
    unsafe {
        libc::ioctl(fd, ask(0x11, 1) as _, &mut axes);
        libc::ioctl(fd, ask(0x12, 1) as _, &mut buttons);
        libc::ioctl(fd, ask(0x13, name.len()) as _, name.as_mut_ptr());
        libc::ioctl(fd, ask(0x32, axis_map.len()) as _, axis_map.as_mut_ptr());
        libc::ioctl(fd, ask(0x34, button_map.len() * 2) as _, button_map.as_mut_ptr());
    }
    let end = name.iter().position(|b| *b == 0).unwrap_or(name.len());
    let name = match String::from_utf8_lossy(&name[..end]).trim() {
        "" => "Game controller".to_owned(),
        n => n.to_owned(),
    };
    // A file that is not a device (the rehearsal) says nothing: the usual pad's layout.
    let (axis_codes, button_codes) = if axes == 0 && buttons == 0 {
        (vec![0, 1, 2, 3, 4, 5, 0x10, 0x11], vec![0x130, 0x131, 0x133, 0x134, 0x136, 0x137, 0x13a, 0x13b, 0x13c, 0x13d, 0x13e])
    } else {
        (axis_map[..axes as usize].to_vec(), button_map[..buttons as usize].to_vec())
    };
    Pad { name, axes: vec![0.0; axis_codes.len()], buttons: vec![false; button_codes.len()], axis_codes, button_codes }
}

/// Its events, until it is unplugged: time (4 bytes), value (2), kind (1), number (1).
fn read(slot: usize, mut file: std::fs::File) {
    let mut e = [0u8; 8];
    while file.read_exact(&mut e).is_ok() {
        let value = i16::from_le_bytes([e[4], e[5]]);
        // (0x80 marks what it says of itself on being opened: taken the same way.)
        let (kind, number) = (e[6] & 0x7f, e[7] as usize);
        let mut changed = false;
        {
            let mut p = PADS.lock().unwrap();
            let Some(pad) = p.list[slot].as_mut() else { return };
            match kind {
                1 => {
                    if let Some(b) = pad.buttons.get_mut(number) {
                        changed = std::mem::replace(b, value != 0) != (value != 0);
                    }
                }
                2 => {
                    if let Some(a) = pad.axes.get_mut(number) {
                        let v = (value as f32 / 32767.0).clamp(-1.0, 1.0);
                        // The cross is told like a button: when it crosses the middle.
                        changed = matches!(pad.axis_codes.get(number), Some(0x10 | 0x11)) && (*a > 0.5) != (v > 0.5) || (*a < -0.5) != (v < -0.5) && matches!(pad.axis_codes.get(number), Some(0x10 | 0x11));
                        *a = v;
                    }
                }
                _ => {}
            }
        }
        // A press is told at once; a stick is read when it is asked for.
        if changed && e[6] & 0x80 == 0 {
            tell();
        }
    }
}
