//! The thread that thinks. A script runs here: it receives named events
//! ("enters orb", "presses view"), decides, and declares transitions. It animates
//! nothing and knows nothing about coordinates.
//!
//! After each decision the script can "work" —block on purpose—, which is what
//! a real shell does when it opens a panel: instantiate, read, parse.

use crate::scene::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Options {
    pub block_for: Duration,
    pub demo: bool,
    pub naive: bool,
    /// Report on the output every event that arrives.
    pub echo: bool,
}

pub trait Script: Send {
    fn scene(&mut self) -> Scene;
    fn on_event(&mut self, e: Event, c: &mut Context);
    /// When it wants to be woken up, if it has something scheduled.
    fn next_deadline(&self) -> Option<Instant> {
        None
    }
    /// Its time has come.
    fn tick(&mut self, _: &mut Context) {}
}

pub struct Context {
    tx: Sender<ToRender>,
    blocked: Arc<AtomicBool>,
    op: Options,
    alarms: Vec<(Instant, &'static str)>,
}

impl Context {
    /// The one for a plugin's logic, which runs on its own thread: no demo, no echo, no rehearsal blocking.
    pub fn for_plugin(tx: Sender<ToRender>, blocked: Arc<AtomicBool>) -> Context {
        Context { tx, blocked, op: Options { block_for: Duration::ZERO, demo: false, naive: false, echo: false }, alarms: Vec::new() }
    }
}

#[allow(dead_code)] // animate and impulse are still there for scripts that still send loose intentions
impl Context {
    pub fn animate(&self, prop: PropId, to: f32, spring: Spring, delay_ms: u64) {
        let t = Transition { prop, to: to.into(), spring, delay: Duration::from_millis(delay_ms) };
        let _ = self.tx.send(ToRender::Command(Command::Animate(t)));
    }

    /// The border with the scene: telling what is true…
    pub fn fact(&self, name: &'static str, value: bool) {
        let _ = self.tx.send(ToRender::Fact(name, value as u8 as f32));
    }

    /// …changing what a text says…
    pub fn text(&self, name: &'static str, value: impl Into<String>) {
        let _ = self.tx.send(ToRender::Text(name, value.into()));
    }

    /// …what has just happened…
    pub fn signal(&self, name: &'static str) {
        let _ = self.tx.send(ToRender::Signal(name));
    }

    /// …and asking for a gesture, which the scene will grant or not depending on its class.
    pub fn gesture(&self, name: &'static str) {
        let _ = self.tx.send(ToRender::Gesture(name));
    }

    pub fn impulse(&self, prop: PropId, velocity: f32) {
        let _ = self.tx.send(ToRender::Command(Command::Impulse { prop, velocity }));
    }

    /// A named alarm; setting it again postpones it.
    pub fn alarm(&mut self, name: &'static str, ms: u64) {
        self.cancel(name);
        self.alarms.push((Instant::now() + Duration::from_millis(ms), name));
    }

    pub fn cancel(&mut self, name: &'static str) {
        self.alarms.retain(|(_, n)| *n != name);
    }

    /// The heavy work. In naive mode it is handed over to the render thread,
    /// because there "logic" and "painting" are the same thread.
    pub fn work(&self) {
        self.work_for(self.op.block_for);
    }

    pub fn work_for(&self, how_long: Duration) {
        if how_long.is_zero() {
            return;
        }
        if self.op.naive {
            let _ = self.tx.send(ToRender::Command(Command::Block(how_long)));
            std::thread::sleep(how_long);
            return;
        }
        self.blocked.store(true, Ordering::Relaxed);
        let end = Instant::now() + how_long;
        // Busy wait: one core at 100 %, like a JS loop that does not yield.
        while Instant::now() < end {
            std::hint::spin_loop();
        }
        self.blocked.store(false, Ordering::Relaxed);
    }
}

pub fn run(mut script: Box<dyn Script>, rx: Receiver<Event>, tx: Sender<ToRender>, blocked: Arc<AtomicBool>, op: Options) {
    let demo = op.demo;
    let mut c = Context { tx, blocked, op, alarms: Vec::new() };
    let mut next_demo = Instant::now() + Duration::from_millis(1200);
    script.on_event(Event::Alarm("start"), &mut c);

    loop {
        let mut until = Instant::now() + Duration::from_secs(3600);
        if demo {
            until = until.min(next_demo);
        }
        for (when, _) in &c.alarms {
            until = until.min(*when);
        }
        if let Some(p) = script.next_deadline() {
            until = until.min(p);
        }
        match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            // In the demo the clock is in charge, not the mouse.
            Ok(e) if !(demo && matches!(e, Event::Enter(_) | Event::Leave(_) | Event::Press(_))) => {
                if c.op.echo {
                    // The fake mouse's echo reports what arrives, it does not
                    // dump it: the output of a `run` can be eight hundred
                    // kilobytes of paths from your home, and that is neither read
                    // nor wanted in a log.
                    // And what is typed into a secret field is not reported: neither
                    // the text, nor the keys while there is one in the scene.
                    let secrets = crate::scene::SECRETS.lock().unwrap();
                    let said = match &e {
                        Event::Text(n, _) if secrets.contains(n) => format!("Text({n:?}, «…»)"),
                        Event::Submit(n, _) if secrets.contains(n) => format!("Submit({n:?}, «…»)"),
                        Event::Key(..) if !secrets.is_empty() => "Key(«…»)".to_owned(),
                        _ => format!("{e:?}"),
                    };
                    drop(secrets);
                    match said.char_indices().nth(160) {
                        Some((k, _)) => println!("logic  · {}… ({} characters)", &said[..k], said.chars().count()),
                        None => println!("logic  · {said}"),
                    }
                }
                script.on_event(e, &mut c)
            }
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let now = Instant::now();
        let due: Vec<&'static str> = c.alarms.iter().filter(|(t, _)| *t <= now).map(|(_, n)| *n).collect();
        c.alarms.retain(|(t, _)| *t > now);
        for n in due {
            script.on_event(Event::Alarm(n), &mut c);
        }
        if script.next_deadline().is_some_and(|p| p <= Instant::now()) {
            script.tick(&mut c);
        }
        if demo && now >= next_demo {
            script.on_event(Event::Demo, &mut c);
            next_demo = Instant::now() + Duration::from_millis(2600);
        }
    }
}
