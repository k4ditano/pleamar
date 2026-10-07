//! A scene owns its recording. Reload cancels it; process exit waits for MP4 finalization.
use super::SysValue;
use std::{cell::RefCell, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
#[path = "windows_recording_capture.rs"] mod capture;
#[path = "windows_recording_media.rs"] mod media;

#[derive(Default)]
struct Status { phase: &'static str, path: String, error: String, frames: u64, audio_samples: u64, audio_nonzero: u64, audio_discontinuities: u64, width: u32, height: u32 }
struct Session { stop: AtomicBool, owner: Arc<AtomicBool>, status: Mutex<Status> }
struct Running { session: Arc<Session>, thread: std::thread::JoinHandle<()> }
static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
static EXITING: AtomicBool = AtomicBool::new(false);
thread_local! { static OWNED: RefCell<Option<Arc<Session>>> = const { RefCell::new(None) }; }
pub(super) fn has_thread_state() -> bool { OWNED.with(|v| v.borrow().is_some()) }

impl Session {
    fn stopped(&self) -> bool { self.stop.load(Ordering::Acquire) || !self.owner.load(Ordering::Acquire) || EXITING.load(Ordering::Acquire) }
    fn value(&self) -> SysValue {
        let status = self.status.lock().unwrap();
        SysValue::Map(vec![
            ("state".into(), SysValue::Text(status.phase.into())),
            ("path".into(), SysValue::Text(status.path.clone())),
            ("error".into(), SysValue::Text(status.error.clone())),
            ("frames".into(), SysValue::Num(status.frames as f64)),
            ("seconds".into(), SysValue::Num(status.frames as f64 / 60.0)),
            ("width".into(), SysValue::Num(status.width as f64)), ("height".into(), SysValue::Num(status.height as f64)),
            ("audio_samples".into(), SysValue::Num(status.audio_samples as f64)),
            ("audio_nonzero".into(), SysValue::Num(status.audio_nonzero as f64)),
            ("audio_discontinuities".into(), SysValue::Num(status.audio_discontinuities as f64)),
        ])
    }
}

fn record(session: &Session, monitor: &str) -> Result<(), String> {
    if session.stopped() { session.status.lock().unwrap().phase = "cancelled"; return Ok(()); }
    let _apartment = super::windows_system::Apartment::new()?;
    let result = || -> windows::core::Result<()> {
        let _media = media::Media::new()?;
        let mut capture = capture::Capture::new(monitor)?;
        let mut audio = media::Loopback::new()?;
        let mut writer = media::Writer::new(&capture.device, capture.width, capture.height, &media::folder()?)?;
        {
            let mut status = session.status.lock().unwrap();
            status.path = writer.path.to_string_lossy().into_owned();
            status.width = capture.width;
            status.height = capture.height;
        }
        let first = Instant::now();
        while !session.stopped() && !capture.update()? {
            if first.elapsed() > Duration::from_secs(10) { return Err(windows::core::Error::new(windows::Win32::Foundation::E_FAIL, "Windows did not deliver a capture frame")); }
            std::thread::sleep(Duration::from_millis(5));
        }
        if session.stopped() {
            let path = writer.path.clone();
            drop(writer);
            let _ = std::fs::remove_file(path);
            let mut status = session.status.lock().unwrap();
            status.phase = "cancelled";
            status.path.clear();
            return Ok(());
        }
        let start = media::clock()?;
        audio.start()?;
        let mut frame = 0;
        let recording = || -> windows::core::Result<()> {
            loop {
                let now = media::clock()?;
                audio.drain(&writer, start, now)?;
                if session.stopped() { break; }
                let due = ((now - start).max(0) as u64 * 60) / 10_000_000;
                if due.saturating_sub(frame) > 120 { return Err(windows::core::Error::new(windows::Win32::Foundation::E_FAIL, "the encoder fell more than two seconds behind real time")); }
                if frame <= due {
                    capture.update()?;
                    writer.video(&capture.nv12()?, frame)?;
                    frame += 1;
                    let mut status = session.status.lock().unwrap();
                    status.phase = "recording";
                    status.frames = frame;
                    status.audio_samples = audio.frames;
                    status.audio_nonzero = audio.nonzero;
                    status.audio_discontinuities = audio.discontinuities;
                } else { std::thread::sleep(Duration::from_millis(2)); }
            }
            // Match the audio tail to the video's final constant-rate sample.
            audio.silence_until(&writer, frame * 800)?;
            Ok(())
        }();
        session.status.lock().unwrap().phase = "finalizing";
        // Even a device removal attempts to preserve a playable partial file;
        // an error is still an error and must not trigger the saved celebration.
        let finalized = writer.finish();
        recording?;
        finalized?;
        let mut status = session.status.lock().unwrap();
        status.phase = if frame > 0 { "saved" } else { "cancelled" };
        status.audio_samples = audio.frames;
        status.audio_nonzero = audio.nonzero;
        status.audio_discontinuities = audio.discontinuities;
        Ok(())
    }();
    result.map_err(|e| e.to_string())
}

pub fn query(name: &str, args: &[SysValue]) -> Result<SysValue, String> {
    match name {
        "recording.start" => {
            let monitor = match args { [] => "", [SysValue::Text(name)] if name.len() < 256 && !name.contains('\0') => name, _ => return Err("recording.start takes an optional monitor name".into()) };
            let mut running = RUNNING.lock().unwrap();
            if EXITING.load(Ordering::Acquire) { return Err("the runtime is closing".into()); }
            if running.as_ref().is_some_and(|v| !v.thread.is_finished()) { return Err("a recording is already running or finalizing".into()); }
            if let Some(previous) = running.take() { let _ = previous.thread.join(); }
            let owner = super::windows_capture::service_lifetime().ok_or("recording requires an asynchronous scene service worker")?;
            if !owner.load(Ordering::Acquire) { return Err("the scene was reloaded".into()); }
            let session = Arc::new(Session { stop: AtomicBool::new(false), owner, status: Mutex::new(Status { phase: "starting", ..Default::default() }) });
            let thread_session = session.clone();
            let monitor = monitor.to_owned();
            let thread = std::thread::Builder::new().name("recording".into()).spawn(move || {
                if let Err(error) = record(&thread_session, &monitor) {
                    let mut status = thread_session.status.lock().unwrap();
                    status.phase = "error";
                    status.error = error;
                    if status.frames == 0 && !status.path.is_empty() {
                        // This path was created exclusively by this session.
                        // Once its writer has dropped, an empty failed attempt
                        // is not a recording worth leaving in the Videos folder.
                        if std::fs::remove_file(&status.path).is_ok() { status.path.clear(); }
                    }
                    eprintln!("windows · recording: {}", status.error);
                }
            }).map_err(|e| e.to_string())?;
            *running = Some(Running { session: session.clone(), thread });
            OWNED.with(|v| *v.borrow_mut() = Some(session.clone()));
            drop(running);
            // Do not hold the service queue while codecs initialize. A stop
            // arriving during startup must reach this exact session promptly.
            Ok(session.value())
        }
        "recording.state" | "recording.stop" => {
            if !args.is_empty() { return Err(format!("{name} takes no arguments")); }
            OWNED.with(|owned| {
                let owned = owned.borrow();
                let Some(session) = owned.as_ref() else { return Ok(SysValue::Map(vec![("state".into(), SysValue::Text("idle".into()))])); };
                if name == "recording.stop" { session.stop.store(true, Ordering::Release); }
                Ok(session.value())
            })
        }
        "recording.folder" if args.is_empty() => {
            let _apartment = super::windows_system::Apartment::new()?;
            Ok(SysValue::Text(media::folder().map_err(|e| e.to_string())?.to_string_lossy().into_owned()))
        }
        _ => Err(format!("unknown recording query: {name}")),
    }
}

pub fn shutdown() {
    EXITING.store(true, Ordering::Release);
    // The timed exit and window loop may both call quit. Neither may terminate
    // the process while the other is still flushing the recording.
    static FINISHED: std::sync::Once = std::sync::Once::new();
    FINISHED.call_once(|| {
        let running = RUNNING.lock().unwrap().take();
        if let Some(running) = running {
            running.session.stop.store(true, Ordering::Release);
            if running.thread.join().is_err() { eprintln!("windows · recording worker panicked during shutdown"); }
        }
    });
}
