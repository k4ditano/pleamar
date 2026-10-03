//! Registered shortcuts, without a keyboard hook or polling held keys.
use super::SysValue;
use std::collections::HashMap;
use std::sync::{OnceLock, mpsc};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*};

const WAKE: u32 = WM_APP + 17;
type Reply = mpsc::SyncSender<Result<(), String>>;
enum Request {
    Watch(Box<dyn Fn(SysValue) + Send>),
    Bind(String, Option<(HOT_KEY_MODIFIERS, u32)>, Reply),
}
struct Manager { sender: mpsc::SyncSender<Request>, thread: u32 }
static MANAGER: OnceLock<Result<Manager, String>> = OnceLock::new();

fn manager() -> Result<&'static Manager, String> {
    MANAGER.get_or_init(|| {
        let (sender, requests) = mpsc::sync_channel::<Request>(32);
        let (ready, started) = mpsc::sync_channel(1);
        std::thread::Builder::new().name("hotkeys".into()).spawn(move || unsafe {
            let mut message = MSG::default();
            // PostThreadMessage requires the receiving thread to have a queue.
            let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
            let _ = ready.send(GetCurrentThreadId());
            let mut bindings = HashMap::<String, (i32, HOT_KEY_MODIFIERS, u32)>::new();
            let mut listeners = Vec::<Box<dyn Fn(SysValue) + Send>>::new();
            let mut sequence = 0u64;
            while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                if message.message == WM_HOTKEY {
                    if let Some((name, _)) = bindings.iter().find(|(_, (id, _, _))| *id == message.wParam.0 as i32) {
                        sequence += 1;
                        let value = event(name, sequence);
                        for listener in &listeners { listener(value.clone()); }
                    }
                } else if message.message == WAKE {
                    for request in requests.try_iter() {
                        match request {
                            Request::Watch(notify) => { notify(event("", sequence)); listeners.push(notify); }
                            Request::Bind(name, binding, reply) => {
                                let result = match binding {
                                    None => {
                                        if let Some((id, _, _)) = bindings.remove(&name) { let _ = UnregisterHotKey(None, id); }
                                        Ok(())
                                    }
                                    Some((modifiers, key)) => {
                                        if bindings.get(&name).is_some_and(|(_, m, k)| m.0 == modifiers.0 && *k == key) {
                                            Ok(())
                                        } else if bindings.len() >= 16 && !bindings.contains_key(&name) {
                                            Err("at most 16 global shortcuts can be registered".into())
                                        } else {
                                            let id = (1..=32).find(|id| !bindings.values().any(|(used, _, _)| used == id)).unwrap();
                                            match RegisterHotKey(None, id, modifiers | MOD_NOREPEAT, key) {
                                                Ok(()) => {
                                                    if let Some((old, _, _)) = bindings.insert(name, (id, modifiers, key)) { let _ = UnregisterHotKey(None, old); }
                                                    Ok(())
                                                }
                                                Err(e) => Err(format!("shortcut unavailable or already used: {e}")),
                                            }
                                        }
                                    }
                                };
                                let _ = reply.send(result);
                            }
                        }
                    }
                }
            }
            for (id, _, _) in bindings.into_values() { let _ = UnregisterHotKey(None, id); }
        }).map_err(|e| e.to_string())?;
        let thread = started.recv().map_err(|e| e.to_string())?;
        Ok(Manager { sender, thread })
    }).as_ref().map_err(Clone::clone)
}

fn event(name: &str, sequence: u64) -> SysValue {
    SysValue::Map(vec![("event".into(), SysValue::Text(name.into())), ("sequence".into(), SysValue::Num(sequence as f64))])
}
fn send(request: Request) -> Result<(), String> {
    let manager = manager()?;
    manager.sender.try_send(request).map_err(|e| e.to_string())?;
    unsafe { PostThreadMessageW(manager.thread, WAKE, WPARAM(0), LPARAM(0)).map_err(|e| e.to_string()) }
}
pub fn service(notify: Box<dyn Fn(SysValue) + Send>) -> bool { send(Request::Watch(notify)).is_ok() }

fn parse(chord: &str) -> Result<(HOT_KEY_MODIFIERS, u32), String> {
    let mut modifiers = HOT_KEY_MODIFIERS(0);
    let mut key = None;
    for part in chord.split('+').map(|p| p.trim().to_ascii_lowercase()) {
        let flag = match part.as_str() {
            "ctrl" | "control" => Some(MOD_CONTROL), "alt" => Some(MOD_ALT), "shift" => Some(MOD_SHIFT),
            "win" | "windows" | "super" => return Err("Windows-logo shortcuts are reserved by the operating system".into()),
            _ => None,
        };
        if let Some(flag) = flag {
            if modifiers.0 & flag.0 != 0 { return Err("duplicate shortcut modifier".into()); }
            modifiers |= flag;
        } else {
            let code = if part == "space" { 0x20 }
                else if part.len() == 1 && part.as_bytes()[0].is_ascii_alphanumeric() { part.as_bytes()[0].to_ascii_uppercase() as u32 }
                else if let Some(n) = part.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=24).contains(n) && *n != 12) { 0x70 + n - 1 }
                else { return Err("shortcut key must be a letter, digit, Space or F1-F24 (except F12)".into()); };
            if key.replace(code).is_some() { return Err("a shortcut has exactly one non-modifier key".into()); }
        }
    }
    if modifiers.0 == 0 { return Err("a global shortcut needs Ctrl, Alt or Shift".into()); }
    Ok((modifiers, key.ok_or("shortcut key is missing")?))
}

pub fn command(name: &str, args: &[SysValue]) -> Result<(), String> {
    let (event, binding) = match (name, args) {
        ("hotkeys.bind", [SysValue::Text(event), SysValue::Text(chord)]) => (event, Some(parse(chord)?)),
        ("hotkeys.unbind", [SysValue::Text(event)]) => (event, None),
        _ => return Err("hotkeys.bind takes an event and chord; hotkeys.unbind takes the event".into()),
    };
    if event.is_empty() || event.len() > 64 || !event.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')) {
        return Err("invalid shortcut event name".into());
    }
    let (reply, response) = mpsc::sync_channel(1);
    send(Request::Bind(event.clone(), binding, reply))?;
    response.recv().map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcuts_reject_ambiguous_and_reserved_keys() {
        let (modifiers, key) = parse("Ctrl+Alt+Space").unwrap();
        assert_eq!(modifiers.0, (MOD_CONTROL | MOD_ALT).0);
        assert_eq!(key, 0x20);
        for chord in ["Win+Space", "F12", "Ctrl+F12", "Ctrl+Ctrl+A", "Ctrl+A+B", "Alt+", "Space"] {
            assert!(parse(chord).is_err(), "{chord}");
        }
    }
}
