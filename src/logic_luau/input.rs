//! Ordered, bounded writes to a long-lived helper, without holding logic state.
use super::{Child, PATIENCE, stop_child};
use std::io::{self, Write};
use std::process::ChildStdin;
use std::sync::{Arc, Weak, Mutex, mpsc};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const MAX_INPUT: usize = 8 << 20;

struct Request {
    text: Option<String>,
    reply: mpsc::Sender<bool>,
}

pub(super) struct InputPipe {
    pub command: String,
    sender: mpsc::SyncSender<Request>,
    active: Arc<AtomicBool>,
    child: Weak<Mutex<Option<std::process::Child>>>,
}

fn deliver(pipe: &mut ChildStdin, text: &str, active: &AtomicBool) -> io::Result<()> {
    if text.len() > MAX_INPUT { return Err(io::Error::other("input exceeds 8 MiB")); }
    let deadline = Instant::now() + PATIENCE;
    let mut bytes = text.as_bytes();
    while !bytes.is_empty() {
        if !active.load(Ordering::Acquire) { return Err(io::ErrorKind::Interrupted.into()); }
        if Instant::now() >= deadline { return Err(io::ErrorKind::TimedOut.into()); }
        match pipe.write(bytes) {
            // A nonblocking Windows byte pipe can succeed with zero bytes.
            Ok(0) => std::thread::sleep(Duration::from_millis(1)),
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(1)),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

impl InputPipe {
    pub fn new(command: String, mut pipe: ChildStdin, child: &Child, initial: Option<String>) -> io::Result<Self> {
        if initial.as_ref().is_some_and(|text| text.len() > MAX_INPUT) {
            return Err(io::Error::other("initial input exceeds 8 MiB"));
        }
        crate::platform::nonblocking_child_input(&pipe)?;
        let (sender, receiver) = mpsc::sync_channel::<Request>(1);
        let active = Arc::new(AtomicBool::new(true));
        let alive = active.clone();
        let weak = Arc::downgrade(child);
        let process = weak.clone();
        std::thread::Builder::new().name("command-input".into()).spawn(move || {
            let initial_ok = initial.as_deref().is_none_or(|text| deliver(&mut pipe, text, &alive).is_ok());
            let mut failed = !initial_ok;
            while !failed && alive.load(Ordering::Acquire) {
                let Ok(request) = receiver.recv() else { break };
                if !alive.load(Ordering::Acquire) { break; }
                let Some(text) = request.text else {
                    drop(pipe);
                    let _ = request.reply.send(true);
                    return;
                };
                let ok = deliver(&mut pipe, &text, &alive).is_ok();
                let _ = request.reply.send(ok);
                failed = !ok;
            }
            if failed {
                alive.store(false, Ordering::Release);
                if let Some(child) = process.upgrade() { stop_child(&child); }
            }
        })?;
        Ok(Self { command, sender, active, child: weak })
    }

    pub fn write(&self, text: Option<String>) -> bool {
        let (reply, result) = mpsc::channel();
        let ok = self.active.load(Ordering::Acquire)
            && text.as_ref().is_none_or(|text| text.len() <= MAX_INPUT)
            && self.sender.try_send(Request { text, reply }).is_ok()
            && result.recv_timeout(PATIENCE).is_ok_and(|ok| ok);
        if !ok { self.abort(); }
        ok
    }

    pub fn abort(&self) {
        self.active.store(false, Ordering::Release);
        if let Some(child) = self.child.upgrade() { stop_child(&child); }
    }
}

impl Drop for InputPipe {
    fn drop(&mut self) { self.active.store(false, Ordering::Release); }
}
