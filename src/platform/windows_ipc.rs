//! Local command pipes. One logon-scoped listener, bounded concurrent requests.
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::sync::{Arc, Mutex, Weak, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HANDLE, ERROR_PIPE_CONNECTED, ERROR_NO_DATA};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Pipes::*;
use windows::core::PCWSTR;

#[path = "windows_ipc_security.rs"]
mod security;

const LIMIT: usize = 65536;
const DEFAULT_WAIT: Duration = Duration::from_secs(2);
const MAX_COMMANDS: usize = 8;
const STREAM: &str = "@stream-v1 ";

fn prefix() -> Result<String, String> {
    // A second logon of the same account must have its own scene names.
    let identity = format!("{}|{}", super::config_dir().display(), std::env::var("PLEAMAR_SOCKET_DIR").unwrap_or_default());
    let hash = identity.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    let hash = security::Logon::current()?.hash(hash);
    Ok(format!("pleamar-{hash:016x}-"))
}

fn pipe_path(scene: &str) -> Result<String, String> {
    if scene.is_empty() || scene.len() > 120 || scene.chars().any(|c| c.is_control() || "\\/:".contains(c)) {
        return Err("invalid scene name for a command pipe".into());
    }
    Ok(format!(r"\\.\pipe\{}{scene}", prefix()?))
}

pub(super) fn read_line(file: &mut File, until: Instant) -> Result<String, String> {
    read_until(file, until, &AtomicBool::new(false))
}
fn read_until(file: &mut File, until: Instant, stopped: &AtomicBool) -> Result<String, String> {
    let mut data = Vec::new();
    loop {
        if stopped.load(Ordering::Acquire) { return Err("command listener stopped".into()); }
        let mut available = 0;
        unsafe { PeekNamedPipe(HANDLE(file.as_raw_handle()), None, 0, None, Some(&mut available), None) }.map_err(|e| e.to_string())?;
        if available > 0 {
            let mut buf = [0u8; 4096];
            let n = file.read(&mut buf[..(available as usize).min(4096)]).map_err(|e| e.to_string())?;
            data.extend_from_slice(&buf[..n]);
            if data.len() > LIMIT { return Err("command frame is too long".into()); }
            if let Some(end) = data.iter().position(|b| *b == b'\n') {
                return String::from_utf8(data[..end].to_vec()).map_err(|e| e.to_string());
            }
        }
        if Instant::now() >= until { return Err("command pipe timed out".into()); }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn write_until(file: &mut File, mut bytes: &[u8], until: Instant, stopped: &AtomicBool) -> Result<(), String> {
    while !bytes.is_empty() {
        if stopped.load(Ordering::Acquire) { return Err("command listener stopped".into()); }
        if Instant::now() >= until { return Err("command pipe write timed out".into()); }
        match file.write(bytes) {
            Ok(0) => std::thread::sleep(Duration::from_millis(5)),
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            },
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

fn bind(scene: &str) -> Result<File, String> { bind_instance(&pipe_path(scene)?, true, MAX_COMMANDS as u32 + 1) }
pub(super) fn bind_path(path: &str) -> Result<File, String> { bind_instance(path, true, 1) }
fn bind_instance(path: &str, first: bool, count: u32) -> Result<File, String> {
    let wide: Vec<u16> = path.encode_utf16().chain([0]).collect();
    let mut security = security::Security::new()?;
    let attributes = security.attributes();
    let access = if first { PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE } else { PIPE_ACCESS_DUPLEX };
    let handle = unsafe {
        CreateNamedPipeW(PCWSTR(wide.as_ptr()), access,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            count, LIMIT as u32, LIMIT as u32, 2000, Some(&attributes))
    };
    if handle.is_invalid() { return Err(format!("{path}: {}", std::io::Error::last_os_error())); }
    Ok(unsafe { File::from_raw_handle(handle.0) })
}

pub(super) fn accept_ready(pipe: &File) -> bool {
    let handle = HANDLE(pipe.as_raw_handle());
    let status = unsafe { ConnectNamedPipe(handle, None) };
    if status.as_ref().err().is_some_and(|e| e.code() == ERROR_PIPE_CONNECTED.to_hresult()) { return true; }
    if status.as_ref().err().is_some_and(|e| e.code() == ERROR_NO_DATA.to_hresult()) {
        unsafe { let _ = DisconnectNamedPipe(handle); }
    }
    false
}

fn wait_for_client(pipe: &File) -> Result<(), String> {
    let handle = HANDLE(pipe.as_raw_handle());
    // Idle listeners sleep in the kernel; only exchanges have deadlines.
    unsafe { SetNamedPipeHandleState(handle, Some(&PIPE_WAIT), None, None) }.map_err(|e| e.to_string())?;
    loop {
        match unsafe { ConnectNamedPipe(handle, None) } {
            Ok(()) => break,
            Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => break,
            Err(e) if e.code() == ERROR_NO_DATA.to_hresult() => {
                unsafe { DisconnectNamedPipe(handle) }.map_err(|e| e.to_string())?;
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    unsafe { SetNamedPipeHandleState(handle, Some(&PIPE_NOWAIT), None, None) }.map_err(|e| e.to_string())
}

struct Connection { pipe: Mutex<Option<File>>, stopped: Arc<AtomicBool> }
impl Connection {
    fn close(&self) {
        if let Some(pipe) = self.pipe.lock().unwrap().take() {
            unsafe { let _ = DisconnectNamedPipe(HANDLE(pipe.as_raw_handle())); }
        }
    }
    fn frame(&self, answer: Option<&str>) -> bool {
        let mut frame = serde_json::to_string(&answer).unwrap();
        if frame.len() >= LIMIT { frame = serde_json::to_string("? response exceeds the 64 KiB command-frame limit").unwrap(); }
        frame.push('\n');
        let mut guard = self.pipe.lock().unwrap();
        let Some(pipe) = guard.as_mut() else { return false; };
        let until = Instant::now() + DEFAULT_WAIT;
        write_until(pipe, frame.as_bytes(), until, &self.stopped).is_ok()
            // Every frame is acknowledged before disconnect or the next frame.
            // This keeps byte-pipe reads bounded and preserves embedded newlines.
            && read_until(pipe, until, &self.stopped).is_ok_and(|s| s == "ack")
    }
}
impl super::CommandReply for Connection {
    fn write(&mut self, line: &str) -> bool { self.frame(Some(line)) }
    fn connected(&self) -> bool {
        if self.stopped.load(Ordering::Acquire) { return false; }
        self.pipe.lock().unwrap().as_ref().is_some_and(|p| unsafe {
            PeekNamedPipe(HANDLE(p.as_raw_handle()), None, 0, None, None, None).is_ok()
        })
    }
}
impl super::CommandReply for Arc<Connection> {
    fn write(&mut self, line: &str) -> bool { self.frame(Some(line)) }
    fn connected(&self) -> bool { super::CommandReply::connected(self.as_ref()) }
}
struct Clients { stopped: Arc<AtomicBool>, live: Vec<Weak<Connection>> }
impl Drop for Clients {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        for client in &self.live { if let Some(client) = client.upgrade() { client.close(); } }
    }
}

fn serve(path: String, mut pipe: File, receive: super::Commands) -> Result<(), String> {
    let mut clients = Clients { stopped: Arc::new(AtomicBool::new(false)), live: Vec::new() };
    let mut workers: Vec<std::thread::JoinHandle<()>> = Vec::new();
    loop {
        wait_for_client(&pipe)?;
        let line = match read_line(&mut pipe, Instant::now() + DEFAULT_WAIT) {
            Ok(line) => line,
            Err(_) => { unsafe { let _ = DisconnectNamedPipe(HANDLE(pipe.as_raw_handle())); } continue; }
        };
        let streaming = line.starts_with(STREAM);
        let line = line.strip_prefix(STREAM).unwrap_or(&line).to_owned();
        let connection = Arc::new(Connection { pipe: Mutex::new(Some(pipe)), stopped: clients.stopped.clone() });
        if line.split_whitespace().next() == Some("quit") {
            // Acknowledge before the UI exits, including when all worker slots are occupied.
            if streaming { connection.frame(None); } else { connection.frame(Some("")); }
            connection.close();
            drop(clients);
            receive(line, &mut |_: &str| false);
            return Ok(());
        }
        workers.retain(|t| !t.is_finished());
        clients.live.retain(|c| c.strong_count() > 0);
        if workers.len() >= MAX_COMMANDS || (!streaming && line.split_whitespace().next() == Some("watch")) {
            connection.frame(Some(if workers.len() >= MAX_COMMANDS { "? too many active scene commands; try again" }
                else { "? watch requires a streaming command client" }));
            if streaming { connection.frame(None); }
            pipe = connection.pipe.lock().unwrap().take().unwrap();
            unsafe { let _ = DisconnectNamedPipe(HANDLE(pipe.as_raw_handle())); }
            continue;
        }
        // Keep a listening instance before handing this one to a worker: the
        // name never becomes unowned between connections, even after a timeout.
        pipe = bind_instance(&path, false, MAX_COMMANDS as u32 + 1)?;
        clients.live.push(Arc::downgrade(&connection));
        let receive = receive.clone();
        let thread = std::thread::Builder::new().name("command".into()).spawn(move || {
            let mut out = connection.clone();
            let answer = receive(line, &mut out);
            if streaming {
                if let Some(answer) = answer { connection.frame(Some(&answer)); }
                connection.frame(None);
            } else { connection.frame(Some(answer.as_deref().unwrap_or_default())); }
            connection.close();
        }).map_err(|e| e.to_string())?;
        workers.push(thread);
    }
}

pub fn listen_for_commands(scene: &str, receive: super::Commands) {
    let mut path = match pipe_path(scene) { Ok(p) => p, Err(e) => { eprintln!("orders · {e}"); return; } };
    let pipe = match bind(scene) {
        Ok(pipe) => pipe,
        Err(first) => {
            let suffix = format!("-{}", std::process::id());
            let mut end = scene.len().min(120 - suffix.len());
            while !scene.is_char_boundary(end) { end -= 1; }
            let own = format!("{}{suffix}", &scene[..end]);
            match bind(&own) {
                Ok(pipe) => {
                    path = pipe_path(&own).unwrap();
                    eprintln!("orders · '{scene}' is unavailable ({first}): this scene answers as '{own}'");
                    pipe
                },
                Err(e) => { eprintln!("orders · {first}; fallback: {e}"); return; },
            }
        },
    };
    std::thread::spawn(move || { if let Err(e) = serve(path, pipe, receive) { eprintln!("orders · {e}"); } });
}

fn connect(path: &str, command: &str, until: Instant) -> Result<File, String> {
    connect_process(path, command, until, None).map(|(pipe, _)| pipe)
}
fn connect_process(path: &str, command: &str, until: Instant, expected: Option<u32>) -> Result<(File, u32), String> {
    if command.len() >= LIMIT || command.contains(['\n', '\r']) { return Err("send one command line at a time (less than 64 KiB)".into()); }
    let mut pipe = loop {
        match OpenOptions::new().read(true).write(true).open(path) {
            Ok(file) => break file,
            Err(e) if Instant::now() >= until => return Err(format!("{path}: {e}")),
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    unsafe { SetNamedPipeHandleState(HANDLE(pipe.as_raw_handle()), Some(&PIPE_NOWAIT), None, None) }.map_err(|e| e.to_string())?;
    let mut pid = 0;
    unsafe { GetNamedPipeServerProcessId(HANDLE(pipe.as_raw_handle()), &mut pid) }.map_err(|e| e.to_string())?;
    if expected.is_some_and(|expected| expected != pid) {
        return Err("the scene process changed; run agent scenes again before sending an action".into());
    }
    write_until(&mut pipe, format!("{command}\n").as_bytes(), until, &AtomicBool::new(false))?;
    Ok((pipe, pid))
}

pub fn ask(scene: &str, command: &str, wait: Duration) -> Result<String, String> { ask_path(&pipe_path(scene)?, command, wait) }
/// The PID comes from the connected native pipe, not text supplied by the scene.
pub fn ask_with_pid(scene: &str, command: &str, wait: Duration) -> Result<(u32, String), String> {
    let until = Instant::now() + wait;
    let (mut pipe, pid) = connect_process(&pipe_path(scene)?, command, until, None)?;
    let reply = read_line(&mut pipe, until)?;
    write_until(&mut pipe, b"ack\n", until, &AtomicBool::new(false))?;
    serde_json::from_str(&reply).map(|reply| (pid, reply)).map_err(|e| e.to_string())
}
pub(super) fn ask_path(path: &str, command: &str, wait: Duration) -> Result<String, String> {
    let until = Instant::now() + wait;
    let mut pipe = connect(path, command, until)?;
    let reply = read_line(&mut pipe, until)?;
    write_until(&mut pipe, b"ack\n", until, &AtomicBool::new(false))?;
    serde_json::from_str(&reply).map_err(|e| e.to_string())
}

/// Each chunk is delivered as it arrives; false cancels the subscription.
pub fn stream(scene: &str, command: &str, wait: Duration, each: &mut dyn FnMut(&str) -> bool) -> Result<(), String> {
    stream_process(scene, command, wait, None, each)
}
fn stream_process(scene: &str, command: &str, wait: Duration, expected: Option<u32>, each: &mut dyn FnMut(&str) -> bool) -> Result<(), String> {
    let until = Instant::now() + wait;
    let (mut pipe, _) = connect_process(&pipe_path(scene)?, &format!("{STREAM}{command}"), until, expected)?;
    loop {
        let reply = read_line(&mut pipe, until)?;
        let reply: Option<String> = serde_json::from_str(&reply).map_err(|e| e.to_string())?;
        write_until(&mut pipe, b"ack\n", until, &AtomicBool::new(false))?;
        match reply { Some(line) => if !each(&line) { return Ok(()); }, None => return Ok(()) }
    }
}

pub fn running_scenes() -> Vec<String> {
    let prefix = match prefix() { Ok(p) => p, Err(e) => { eprintln!("orders · {e}"); return Vec::new(); } };
    let Ok(entries) = std::fs::read_dir(r"\\.\pipe\") else { return Vec::new() };
    let mut names: Vec<String> = entries.filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_string_lossy().strip_prefix(&prefix).map(str::to_owned)).collect();
    names.sort(); names.dedup(); names
}

fn command_wait(command: &str) -> Duration {
    let (what, rest) = command.trim().split_once(' ').unwrap_or((command.trim(), ""));
    match what {
        "watch" => crate::agent::watch_duration(rest) + Duration::from_secs(3),
        "wait" => Duration::from_secs(65),
        _ => Duration::from_secs(10),
    }
}
/// Revalidate the discovery PID on the connected pipe before writing any order.
pub fn send_to_process(scene: &str, pid: u32, command: &str) -> Result<(), String> {
    print_stream(scene, command, Some(pid))
}
fn print_stream(scene: &str, command: &str, expected: Option<u32>) -> Result<(), String> {
    stream_process(scene, command, command_wait(command), expected, &mut |answer| {
        let stdout = std::io::stdout(); let mut output = stdout.lock();
        if !answer.is_empty() && writeln!(output, "{}", answer.trim_end_matches('\n')).is_err() { return false; }
        output.flush().is_ok()
    })
}
pub fn send(scene: Option<&str>, command: &str) -> Result<(), String> {
    let name = match scene {
        Some(name) => name.to_owned(),
        None => match running_scenes().as_slice() {
            [name] => name.clone(),
            [] => return Err("there is no scene running".into()),
            names => return Err(format!("there are several scenes running; say which: {}", names.join(", "))),
        },
    };
    print_stream(&name, command, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_pipe_roundtrip_and_duplicate_owner() {
        let name = format!("pipe test ñ {}", std::process::id());
        listen_for_commands(&name, Arc::new(|line, _| Some(if line == "multiline" { "first\nsecond 🚀".into() } else { format!("reply {line}") })));
        assert!(bind(&name).is_err(), "a second scene must not steal the first pipe");
        assert!(running_scenes().contains(&name));
        assert_eq!(ask(&name, "text greeting héllo 世界", DEFAULT_WAIT).unwrap(), "reply text greeting héllo 世界");
        assert_eq!(ask(&name, "get greeting", DEFAULT_WAIT).unwrap(), "reply get greeting");
        assert_eq!(ask(&name, "multiline", DEFAULT_WAIT).unwrap(), "first\nsecond 🚀");
    }
    #[test]
    fn caller_timeout_bounds_connection_and_reply() {
        let name = format!("slow pipe {}", std::process::id());
        listen_for_commands(&name, Arc::new(|_, _| { std::thread::sleep(Duration::from_millis(150)); Some("late".into()) }));
        let start = Instant::now();
        assert!(ask(&name, "probe report", Duration::from_millis(25)).is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(ask(&format!("missing {}", std::process::id()), "probe", Duration::from_millis(25)).is_err());
    }
    #[test]
    fn abandoned_connection_cannot_block_the_next_command() {
        let name = format!("abandoned pipe {}", std::process::id());
        let path = pipe_path(&name).unwrap();
        let mut pipe = bind(&name).unwrap();
        assert!(!accept_ready(&pipe));
        let abandoned = OpenOptions::new().read(true).write(true).open(&path).unwrap();
        drop(abandoned);
        let stale = unsafe { ConnectNamedPipe(HANDLE(pipe.as_raw_handle()), None) }.unwrap_err();
        assert_eq!(stale.code(), ERROR_NO_DATA.to_hresult());
        assert!(!accept_ready(&pipe));
        let client = std::thread::spawn(move || ask_path(&path, "after abandoned client", Duration::from_secs(5)));
        let until = Instant::now() + Duration::from_secs(5);
        while !accept_ready(&pipe) {
            assert!(Instant::now() < until, "stale connection still owns the command endpoint");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(read_line(&mut pipe, until).unwrap(), "after abandoned client");
        writeln!(pipe, "\"recovered\"").unwrap();
        let _ = read_line(&mut pipe, until);
        assert_eq!(client.join().unwrap().unwrap(), "recovered");
    }
    #[test]
    fn commands_resume_after_idle_and_quit_releases_endpoint() {
        #[link(name="kernel32")]
        unsafe extern "system" { fn QueryThreadCycleTime(thread: HANDLE, cycles: *mut u64) -> i32; }
        let name = format!("idle commands {}", std::process::id());
        let path = pipe_path(&name).unwrap();
        let pipe = bind(&name).unwrap();
        // A stale connection must also recover when switching to blocking accept.
        assert!(!accept_ready(&pipe));
        drop(OpenOptions::new().read(true).write(true).open(&path).unwrap());
        let (quit, observed) = std::sync::mpsc::channel();
        let server_path=path.clone();
        let thread = std::thread::spawn(move || serve(server_path, pipe, Arc::new(move |line, _| {
            if line == "quit" { quit.send(()).unwrap(); }
            Some(format!("reply {line}"))
        })));
        for cycle in 0..2 {
            std::thread::sleep(Duration::from_millis(100));
            let cycles = || {
                let mut count = 0;
                assert_ne!(unsafe { QueryThreadCycleTime(HANDLE(thread.as_raw_handle()), &mut count) }, 0);
                count
            };
            let before = cycles();
            std::thread::sleep(Duration::from_millis(350));
            println!("idle command cycle {cycle}: {} CPU cycles during 350 ms", cycles() - before);
            assert_eq!(ask(&name, "hello 世界", DEFAULT_WAIT).unwrap(), "reply hello 世界");
            // An accepted client that sends no request must not hold the endpoint.
            let until = Instant::now() + DEFAULT_WAIT;
            let abandoned = loop {
                match OpenOptions::new().read(true).write(true).open(&path) {
                    Ok(pipe) => break pipe,
                    Err(e) if Instant::now() >= until => panic!("could not connect idle client: {e}"),
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            };
            drop(abandoned);
            assert_eq!(ask(&name, "after disconnect", DEFAULT_WAIT).unwrap(), "reply after disconnect");
        }
        assert_eq!(ask(&name, "quit", DEFAULT_WAIT).unwrap(), "");
        observed.recv_timeout(DEFAULT_WAIT).unwrap();
        let until = Instant::now() + DEFAULT_WAIT;
        while !thread.is_finished() {
            assert!(Instant::now() < until, "command listener did not stop after quit");
            std::thread::sleep(Duration::from_millis(5));
        }
        thread.join().unwrap().unwrap();
        assert!(bind(&name).is_ok(), "quit must release exclusive ownership of the pipe");
    }
    #[test]
    fn rejects_pipe_path_injection() {
        for name in ["", "../scene", "x\\y", "C:scene", "line\n"] { assert!(pipe_path(name).is_err()); }
        assert!(pipe_path("scene ñ").is_ok());
    }

    #[test]
    fn streaming_delivers_unicode_lines_while_other_commands_continue() {
        let name = format!("stream concurrency {}", std::process::id());
        let path = pipe_path(&name).unwrap();
        let pipe = bind(&name).unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let stop = done.clone();
        let server = std::thread::spawn(move || serve(path, pipe, Arc::new(move |line, out| {
            if line == "watch" {
                assert!(out.write("first\nEspañol 世界 🚀"));
                let until = Instant::now() + Duration::from_secs(5);
                while !stop.load(Ordering::Acquire) && out.connected() && Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(10));
                }
                out.write("last"); None
            } else { Some(format!("reply {line}")) }
        })));
        let (first, heard) = std::sync::mpsc::channel();
        let scene = name.clone();
        let client = std::thread::spawn(move || {
            let mut lines = Vec::new();
            stream(&scene, "watch", Duration::from_secs(6), &mut |line| {
                lines.push(line.to_owned()); first.send(()).unwrap(); true
            }).unwrap();
            lines
        });
        heard.recv_timeout(DEFAULT_WAIT).unwrap();
        assert_eq!(ask(&name, "hello", DEFAULT_WAIT).unwrap(), "reply hello");
        done.store(true, Ordering::Release);
        assert_eq!(client.join().unwrap(), ["first\nEspañol 世界 🚀", "last"]);
        assert_eq!(ask(&name, "quit", DEFAULT_WAIT).unwrap(), "");
        server.join().unwrap().unwrap();
    }

    #[test]
    fn cancelled_quiet_streams_release_workers_without_waiting_for_an_event() {
        let name = format!("cancel stream {}", std::process::id());
        let path = pipe_path(&name).unwrap();
        let pipe = bind(&name).unwrap();
        let (released, heard) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || serve(path, pipe, Arc::new(move |line, out| {
            if line == "watch" {
                out.write("subscribed");
                let until = Instant::now() + DEFAULT_WAIT;
                while out.connected() && Instant::now() < until { std::thread::sleep(Duration::from_millis(10)); }
                assert!(!out.connected(), "quiet disconnected client still appears connected");
                released.send(()).unwrap();
            }
            None
        })));
        for _ in 0..MAX_COMMANDS * 2 {
            stream(&name, "watch", DEFAULT_WAIT, &mut |line| { assert_eq!(line, "subscribed"); false }).unwrap();
            heard.recv_timeout(DEFAULT_WAIT).unwrap();
        }
        ask(&name, "quit", DEFAULT_WAIT).unwrap();
        server.join().unwrap().unwrap();
        assert!(bind(&name).is_ok());
    }

    #[test]
    fn quit_releases_all_instances_even_when_stream_readers_stop_acknowledging() {
        let name = format!("quit occupied streams {}", std::process::id());
        let path = pipe_path(&name).unwrap();
        let pipe = bind(&name).unwrap();
        let server_path = path.clone();
        let server = std::thread::spawn(move || serve(server_path, pipe, Arc::new(|line, out| {
            if line == "watch" { out.write("first"); }
            None
        })));
        let mut clients = Vec::new();
        for _ in 0..MAX_COMMANDS {
            let until = Instant::now() + DEFAULT_WAIT;
            let mut client = connect(&path, "@stream-v1 watch", until).unwrap();
            assert_eq!(read_line(&mut client, until).unwrap(), "\"first\"");
            clients.push(client); // Deliberately no acknowledgement.
        }
        assert!(ask(&name, "hello", DEFAULT_WAIT).unwrap().contains("too many active"));
        ask(&name, "quit", DEFAULT_WAIT).unwrap();
        let until = Instant::now() + DEFAULT_WAIT;
        while !server.is_finished() {
            assert!(Instant::now() < until, "quit is blocked by stream readers");
            std::thread::sleep(Duration::from_millis(5));
        }
        server.join().unwrap().unwrap();
        drop(clients);
        assert!(bind(&name).is_ok(), "all listening and active instances must retire");
    }

    #[test]
    fn duplicate_scene_uses_its_process_suffix_without_replacing_the_first() {
        let name = format!("duplicate ñ {}", std::process::id());
        let second = format!("{name}-{}", std::process::id());
        listen_for_commands(&name, Arc::new(|_, _| Some("first".into())));
        listen_for_commands(&name, Arc::new(|_, _| Some("second".into())));
        assert_eq!(ask(&name, "hello", DEFAULT_WAIT).unwrap(), "first");
        assert_eq!(ask(&second, "hello", DEFAULT_WAIT).unwrap(), "second");
        ask(&name, "quit", DEFAULT_WAIT).unwrap();
        ask(&second, "quit", DEFAULT_WAIT).unwrap();
    }

    #[test]
    fn native_server_identity_is_checked_before_delivering_an_action() {
        let name = format!("peer identity {}", std::process::id());
        let path = pipe_path(&name).unwrap();
        let pipe = bind(&name).unwrap();
        let (sent, heard) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || serve(path, pipe, Arc::new(move |line, _| {
            sent.send(line.clone()).unwrap(); Some(format!("reply {line}"))
        })));
        let (pid, answer) = ask_with_pid(&name, "hello", DEFAULT_WAIT).unwrap();
        assert_eq!(pid, std::process::id());
        assert_eq!(answer, "reply hello");
        assert_eq!(heard.recv_timeout(DEFAULT_WAIT).unwrap(), "hello");
        let wrong = pid.checked_add(1).unwrap_or(0);
        assert!(send_to_process(&name, wrong, "press forbidden").unwrap_err().contains("process changed"));
        let mut received = String::new();
        stream_process(&name, "press allowed", DEFAULT_WAIT, Some(pid), &mut |line| { received.push_str(line); true }).unwrap();
        assert_eq!(received, "reply press allowed");
        assert_eq!(heard.recv_timeout(DEFAULT_WAIT).unwrap(), "press allowed");
        assert!(heard.try_recv().is_err(), "wrong PID must not deliver an order");
        ask(&name, "quit", DEFAULT_WAIT).unwrap();
        server.join().unwrap().unwrap();
    }

    #[test]
    fn only_long_commands_extend_the_client_deadline() {
        assert_eq!(command_wait("watch 2s"), Duration::from_secs(5));
        assert_eq!(command_wait("watch inf"), Duration::from_secs(13));
        assert_eq!(command_wait("watch 1 extra"), Duration::from_secs(13));
        assert_eq!(command_wait("  watch   2s  "), Duration::from_secs(5));
        assert_eq!(command_wait("watch 3601s"), Duration::from_secs(13));
        assert_eq!(command_wait("watchdog"), Duration::from_secs(10));
        assert_eq!(command_wait("wait saved == true 3s"), Duration::from_secs(65));
    }
}
