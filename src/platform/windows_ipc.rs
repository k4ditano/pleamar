//! Local command pipes. A scene owns its name until its last handle closes.
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HANDLE, ERROR_PIPE_CONNECTED};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Pipes::*;
use windows::core::PCWSTR;

const LIMIT: usize = 65536;
const DEFAULT_WAIT: Duration = Duration::from_secs(2);

fn prefix() -> String {
    // Keep different users' scenes and test namespaces separate. The default
    // process DACL also applies to the pipe; remote clients are rejected.
    let identity = format!("{}|{}", super::config_dir().display(), std::env::var("PLEAMAR_SOCKET_DIR").unwrap_or_default());
    let hash = identity.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    format!("pleamar-{hash:016x}-")
}

fn pipe_path(scene: &str) -> Result<String, String> {
    if scene.is_empty() || scene.len() > 120 || scene.chars().any(|c| c.is_control() || "\\/:".contains(c)) {
        return Err("invalid scene name for a command pipe".into());
    }
    Ok(format!(r"\\.\pipe\{}{scene}", prefix()))
}

fn read_line(file: &mut File, until: Instant) -> Result<String, String> {
    let mut data = Vec::new();
    loop {
        let mut available = 0;
        unsafe { PeekNamedPipe(HANDLE(file.as_raw_handle()), None, 0, None, Some(&mut available), None) }.map_err(|e| e.to_string())?;
        if available > 0 {
            let mut buf = [0u8; 4096];
            let n = file.read(&mut buf[..(available as usize).min(4096)]).map_err(|e| e.to_string())?;
            data.extend_from_slice(&buf[..n]);
            if data.len() > LIMIT { return Err("command is too long".into()); }
            if let Some(end) = data.iter().position(|b| *b == b'\n') {
                return String::from_utf8(data[..end].to_vec()).map_err(|e| e.to_string());
            }
        }
        if Instant::now() >= until { return Err("command pipe timed out".into()); }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn bind(scene: &str) -> Result<File, String> {
    let path = pipe_path(scene)?;
    let wide: Vec<u16> = path.encode_utf16().chain([0]).collect();
    let handle = unsafe {
        CreateNamedPipeW(PCWSTR(wide.as_ptr()), PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1, LIMIT as u32, LIMIT as u32, 2000, None)
    };
    if handle.is_invalid() { return Err(format!("{path}: {}", std::io::Error::last_os_error())); }
    Ok(unsafe { File::from_raw_handle(handle.0) })
}

pub fn listen_for_commands(scene: &str, receive: Box<dyn Fn(String) -> Option<String> + Send>) {
    let mut pipe = match bind(scene) {
        Ok(p) => p,
        Err(e) => { eprintln!("orders · {e}"); return; }
    };
    std::thread::spawn(move || loop {
        let handle = HANDLE(pipe.as_raw_handle());
        let connected = unsafe { ConnectNamedPipe(handle, None) };
        if connected.is_ok() || connected.as_ref().err().is_some_and(|e| e.code() == ERROR_PIPE_CONNECTED.to_hresult()) {
            if let Ok(line) = read_line(&mut pipe, Instant::now() + DEFAULT_WAIT) {
                // A quit must be acknowledged before the UI thread exits.
                let quitting = line.split_whitespace().next() == Some("quit");
                let answer = if quitting { String::new() } else { receive(line.clone()).unwrap_or_default() };
                let answer = serde_json::to_string(&answer).unwrap();
                if answer.len() < LIMIT { let _ = writeln!(pipe, "{answer}"); }
                // Wait until the client reads the reply, with a bounded wait.
                // Its acknowledgement prevents DisconnectNamedPipe discarding it.
                let _ = read_line(&mut pipe, Instant::now() + DEFAULT_WAIT);
                if quitting { receive(line); }
            }
            unsafe { let _ = DisconnectNamedPipe(handle); }
        }
        std::thread::sleep(Duration::from_millis(5));
    });
}

pub fn ask(scene: &str, command: &str, wait: Duration) -> Result<String, String> {
    if command.len() >= LIMIT || command.contains(['\n', '\r']) { return Err("send one command line at a time (less than 64 KiB)".into()); }
    let path = pipe_path(scene)?;
    let until = Instant::now() + wait;
    let mut pipe = loop {
        match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => break file,
            Err(e) if Instant::now() >= until => return Err(format!("{path}: {e}")),
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    writeln!(pipe, "{command}").map_err(|e| e.to_string())?;
    let reply = read_line(&mut pipe, until)?;
    let _ = writeln!(pipe, "ack");
    serde_json::from_str(&reply).map_err(|e| e.to_string())
}

pub fn running_scenes() -> Vec<String> {
    let prefix = prefix();
    let Ok(entries) = std::fs::read_dir(r"\\.\pipe\") else { return Vec::new() };
    let mut names: Vec<String> = entries.filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_string_lossy().strip_prefix(&prefix).map(str::to_owned)).collect();
    names.sort();
    names.dedup();
    names
}

pub fn send(scene: Option<&str>, command: &str) -> Result<(), String> {
    let name = match scene {
        Some(name) => name.to_owned(),
        None => {
            let names = running_scenes();
            match names.as_slice() {
                [name] => name.clone(),
                [] => return Err("there is no scene running".into()),
                _ => return Err(format!("there are several scenes running; say which: {}", names.join(", "))),
            }
        }
    };
    let answer = ask(&name, command, DEFAULT_WAIT)?;
    if !answer.is_empty() { println!("{answer}"); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_pipe_roundtrip_and_duplicate_owner() {
        let name = format!("pipe test ñ {}", std::process::id());
        listen_for_commands(&name, Box::new(|line| Some(if line == "multiline" { "first\nsecond 🚀".into() } else { format!("reply {line}") })));
        assert!(bind(&name).is_err(), "a second scene must not steal the first pipe");
        assert!(running_scenes().contains(&name));
        assert_eq!(ask(&name, "text greeting héllo 世界", DEFAULT_WAIT).unwrap(), "reply text greeting héllo 世界");
        assert_eq!(ask(&name, "get greeting", DEFAULT_WAIT).unwrap(), "reply get greeting");
        assert_eq!(ask(&name, "multiline", DEFAULT_WAIT).unwrap(), "first\nsecond 🚀");
    }
    #[test]
    fn caller_timeout_bounds_connection_and_reply() {
        let name = format!("slow pipe {}", std::process::id());
        listen_for_commands(&name, Box::new(|_| { std::thread::sleep(Duration::from_millis(150)); Some("late".into()) }));
        let start = Instant::now();
        assert!(ask(&name, "probe report", Duration::from_millis(25)).is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(ask(&format!("missing {}", std::process::id()), "probe", Duration::from_millis(25)).is_err());
    }
    #[test]
    fn rejects_pipe_path_injection() {
        for name in ["", "../scene", "x\\y", "C:scene", "line\n"] { assert!(pipe_path(name).is_err()); }
        assert!(pipe_path("scene ñ").is_ok());
    }
}
