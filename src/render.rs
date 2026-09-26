//! The thread that paints. It owns the springs and the clock: it receives intentions,
//! walks through them at the screen's cadence and, when everything is still, stops
//! painting altogether.

use crate::scene::*;
use crate::gpu::{DrawList, Gpu, Sheet, HUD_HEIGHT, N_UNIFORMS};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Options {
    pub hud: bool,
    pub naive: bool,
    /// Reduced motion: springs settle and gestures show their still face.
    pub reduced_motion: bool,
    /// Without waiting for the screen and without resting: to measure what painting costs.
    pub no_vsync: bool,
    /// What to record on every frame: properties, facts or texts, by name.
    /// It is how you check that an animation lasts as long as it says it lasts.
    pub trace: Vec<String>,
    pub start_time: Instant,
}

struct Rng(u64);
impl Rng {
    fn between(&mut self, a: f32, b: f32) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        a + (b - a) * ((self.0 >> 40) as f32 / (1u64 << 24) as f32)
    }
}

#[derive(Default)]
struct Cycle {
    dts: Vec<f32>,
    max_blocked: f32,
    frames_blocked: u32,
    /// What goes into READING the scene —evaluating its expressions and noting down what
    /// has to be painted—, against what goes into drawing it. It is always measured:
    /// it is two clocks per frame, and it is the only way for whoever writes a
    /// scene to find out they have made it too expensive.
    compose_ms: f32,
    slow_reported: bool,
}

impl Cycle {
    /// A scene that does not deliver the frames the screen asks for does not show in the
    /// log —they are isolated slow frames, one after another— and from outside it looks
    /// like the runtime has become slow. It is said ONCE, with the breakdown, as
    /// soon as there is enough to say it with: four seconds of lagging behind.
    fn watch(&mut self, period_ms: f32) {
        if self.slow_reported || self.dts.len() < 240 {
            return;
        }
        let mean = self.dts.iter().sum::<f32>() / self.dts.len() as f32;
        if mean <= period_ms * 1.35 {
            return;
        }
        self.slow_reported = true;
        let reading = self.compose_ms / self.dts.len() as f32;
        eprintln!(
            "render · this scene does not keep up: {mean:.1} ms a frame against the {period_ms:.1} the screen gives, and {reading:.1} of those go in reading the scene, not in drawing it. Something in it is too dear to work out sixty times a second"
        );
    }

    fn close(&mut self) {
        if self.dts.len() < 8 {
            self.dts.clear();
            return;
        }
        let mut o = self.dts.clone();
        o.sort_by(|a, b| a.total_cmp(b));
        let reading = self.compose_ms / o.len() as f32;
        let mean = o.iter().sum::<f32>() / o.len() as f32;
        let p99 = o[((o.len() as f32 * 0.99) as usize).min(o.len() - 1)];
        println!(
            "cycle  · {:>4} frames · mean {:>5.2} ms · p99 {:>6.2} ms · max {:>6.2} ms · reading the scene {:.2} ms · with the logic blocked: {} frames, max {:.2} ms",
            o.len(), mean, p99, o[o.len() - 1], reading, self.frames_blocked, self.max_blocked
        );
        let reported = self.slow_reported;
        *self = Cycle::default();
        self.slow_reported = reported;
    }
}

pub fn run(
    instance: wgpu::Instance,
    rx: Receiver<ToRender>,
    mut letters: crate::text::Texts,
    to_logic: Sender<Event>,
    logic_blocked: Arc<AtomicBool>,
    op: Options,
) {
    let mut gpu: Option<Gpu> = None;
    let mut sheets: Vec<Sheet> = Vec::new();
    let mut draw = DrawList::default();
    let mut previous = crate::gpu::PreviousFrame::default();
    let mut changed: Vec<[f32; 4]> = Vec::new();
    let mut sheet_counts = (0u32, 0u32, 0u32);
    let no_lens = std::env::var_os("PLEAMAR_NO_LENS").is_some();
    // The compositor's notice that it wants another frame: from which sheet it is
    // expected, whether it has already arrived, how many times in a row it did not arrive, and what arrived
    // from the rest of the world while waiting (it is handled on the next round).
    let mut frame_requested: Option<u32> = None;
    let mut frame_ready = false;
    let mut missed_notices = 0u32;
    let mut held_back: Vec<ToRender> = Vec::new();
    let mut atlas_stale = false;
    let mut first_frame = true;
    let mut texts: Vec<String> = Vec::new();
    // The language the texts are shown in right now (`locale`, with `translations`).
    let mut shown_locale: Option<usize> = None;
    let mut key_repeat: Option<(u32, u32)> = Some((400, 33));
    // Each popup of the scene: whether it is open, where, and at what size.
    let mut popups: Vec<Option<[i32; 4]>> = Vec::new();
    // The lock surfaces that are engaged (or requested).
    let mut locks: Vec<usize> = Vec::new();
    let mut uniforms = [0f32; N_UNIFORMS];
    let mut size = (720.0f32, 224.0f32);
    // If `--record` was asked for, the header is written once.
    let mut trace_header_done = false;
    let mut region: Vec<[i32; 4]> = vec![[i32::MIN; 4]];

    // ── state ────────────────────────────────────────────────────
    let mut scene = Scene::default();
    let mut props: Vec<Animated> = Vec::new();
    let mut facts: Vec<f32> = Vec::new();
    let mut inside: Vec<bool> = Vec::new();
    let mut blinks: Vec<(Instant, Option<Instant>)> = Vec::new();
    let mut pending: Vec<(Instant, Transition)> = Vec::new();
    let mut layers: Vec<LayerState> = Vec::new();
    let mut rules: Vec<RuleState> = Vec::new();
    let mut gesture: Option<Playback> = None;
    // What a keyframe emits is handled on the next frame.
    let mut late_signals: Vec<SignalId> = Vec::new();
    let mut pointer: Option<(f32, f32)> = None;
    // The light a click leaves on the glass: where, and how much is left. It lights up
    // on pressing and goes out by itself, slowly.
    let mut finger = (0.0f32, 0.0f32);
    let mut finger_light = 0.0f32;
    let mut finger_down = false;
    // The ring a press sends through the glass: where, and since when.
    let mut ripple: Option<((f32, f32), Instant)> = None;
    // The mouse on the whole desktop, and the monitors, when the system says.
    let mut cursor: Option<((f32, f32), Vec<(String, [i32; 4])>)> = None;
    // What is being dragged: which zone, and where the mouse was when pressed.
    let mut drag: Option<(usize, (f32, f32), Instant)> = None;
    let mut cursor_set = Cursor::Normal;

    // The keyboard that has been asked of the compositor, whether it has the focus, and whether it has
    // been asked for exclusively (see `Effect::FocusField`).
    let mut keyboard_set: Option<Keyboard> = None;
    let mut have_keyboard = false;
    let mut keyboard_lent: Option<Instant> = None;
    // Since when the scene stopped wanting the keyboard while it was lent.
    let mut loan_grace: Option<Instant> = None;
    // The field being typed into, and the key that has been left held down.
    let mut editing: Option<Editing> = None;
    let mut repeat: Option<(String, Option<String>, Mods, Instant)> = None;
    let mut last_key = Instant::now();
    let mut last_pointer: Option<(f32, f32)> = None;
    let mut last_activity = Instant::now();
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let start = Instant::now();

    let mut history = [0f32; 120];
    let mut cycle = Cycle::default();
    let mut last = Instant::now();
    let mut resting = false;
    let mut next_appointment: Option<Instant> = None;
    let mut period_ms = 16.7f32;
    let mut last_presented = Instant::now();
    // Which edge each surface is attached to right now, so as not to ask for it twice.
    let mut levels_set: Vec<crate::scene::Level> = Vec::new();
    let mut anchors_set: Vec<crate::scene::SurfaceAnchor> = Vec::new();
    // The "this does not compile" banner, and the good scene with it on top.
    let mut warning: Option<Vec<Instr>> = None;
    let mut with_warning: Vec<Instr> = Vec::new();
    let mut next_frame = Instant::now();
    let mut frames_for_period = 0u32;

    loop {
        // ── 1. what has arrived ─────────────────────────────────
        let mut block = None;
        // (which button, whether it goes down or up), the wheel notches and the keys of this frame.
        let mut buttons: Vec<(u8, bool)> = Vec::new();
        let mut wheel = 0.0f32;
        let mut keys: Vec<String> = Vec::new();
        let mut key_presses: Vec<(String, Option<String>, Mods)> = Vec::new();
        let mut focus_changes: Vec<bool> = Vec::new();
        let mut drops: Vec<(String, String)> = Vec::new();
        // (which one, whether it comes from the logic)
        let mut signals: Vec<(usize, bool, Option<f32>)> = late_signals.drain(..).map(|s| (s.0 as usize, false, None)).collect();
        let mut gestures_asked: Vec<usize> = Vec::new();
        let mut incoming: Vec<ToRender> = std::mem::take(&mut held_back);
        if resting {
            // Still: not a single frame. Only a message or the next appointment wakes it
            // up: a delay that expires, a blink, a claim that runs out.
            let until = next_appointment.unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
            if incoming.is_empty() {
                match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
                    Ok(m) => incoming.push(m),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            resting = false;
            last = Instant::now() - Duration::from_secs_f32(period_ms / 1000.0);
        }
        incoming.extend(rx.try_iter());
        for m in incoming {
            match m {
                // A reload that does not go through is shown on the surface, not only on
                // a console that nobody may be looking at. It is composed
                // once, on arrival: while it lasts, the good scene is painted with the
                // banner on top.
                ToRender::ReloadError(what) => {
                    // `size: full` does not tell its width until the compositor
                    // configures it: then it is the screen's, which is `screen.width`.
                    let width = match scene.surface().width {
                        0 => facts.first().copied().unwrap_or(900.0),
                        w => w as f32,
                    };
                    warning = what.map(|m| error_banner(&m, width));
                    with_warning.clear();
                    if let Some(banner) = &warning {
                        with_warning.extend(scene.instrs.iter().cloned());
                        with_warning.extend(banner.iter().cloned());
                    }
                }
                ToRender::Scene(fresh) => {
                    crate::platform::release_memory();
                    crate::platform::CURSOR_WANTED.store(fresh.wants_cursor, std::sync::atomic::Ordering::Relaxed);
                    // The properties with the same name survive the change.
                    let old: Vec<(&str, Animated)> =
                        scene.props.iter().map(|p| p.0).zip(props.iter().copied()).collect();
                    props = fresh
                        .props
                        .iter()
                        .map(|(name, initial, spring)| {
                            old.iter().find(|(n, _)| n == name).map(|(_, a)| *a).unwrap_or(Animated::at(*initial, *spring))
                        })
                        .collect();
                    // What was true and what the texts said also survives: on a
                    // hot reload, the scene carries on where it was.
                    let hot = !scene.props.is_empty();
                    facts = fresh.facts.iter().map(|(n, initial)| scene.facts.iter().position(|h| h.0 == *n).map_or(*initial, |k| facts[k])).collect();
                    inside = vec![false; fresh.zones.len()];
                    warning = None;
                    with_warning.clear();
                    anchors_set = fresh.surfaces.iter().map(|s| s.anchor).collect();
                    blinks = fresh
                        .behaviors
                        .iter()
                        .filter_map(|c| match c {
                            Behavior::Blink { every, .. } => {
                                Some((Instant::now() + Duration::from_secs_f32(rng.between(every.0, every.1) * 0.6), None))
                            }
                            _ => None,
                        })
                        .collect();
                    layers = fresh.layers.iter().map(|c| LayerState::new(c.claims.len(), hot)).collect();
                    rules = fresh
                        .rules
                        .iter()
                        .map(|r| {
                            let mut e = RuleState::default();
                            if let Trigger::Every { between, .. } = &r.when {
                                e.next = Some(Instant::now() + Duration::from_secs_f32(rng.between(between.0, between.1)));
                            }
                            e
                        })
                        .collect();
                    gesture = None;
                    pending.clear();
                    size = (fresh.surface().width as f32, fresh.surface().height as f32 + if op.hud { HUD_HEIGHT } else { 0.0 });
                    texts = fresh.texts.iter().map(|(n, initial)| scene.texts.iter().position(|t| t.0 == *n).map_or_else(|| initial.clone(), |k| texts[k].clone())).collect();
                    shown_locale = None;
                    atlas_stale = true;
                    println!(
                        "render · scene: {} properties, {} instructions, {} facts, {} layers, {} gestures, {} rules, {} zones",
                        fresh.props.len(), fresh.instrs.len(), fresh.facts.len(), fresh.layers.len(),
                        fresh.gestures.len(), fresh.rules.len(), fresh.zones.len()
                    );
                    scene = fresh;
                    // Its own shaders: the pipeline is only remade if they changed.
                    if let Some(g) = gpu.as_mut() {
                        g.set_shaders(&scene.shaders);
                    }
                }
                ToRender::Sheet(n) => {
                    let g = gpu.get_or_insert_with(|| Gpu::new(&instance, &n.surface));
                    g.set_shaders(&scene.shaders);
                    // A scene that asks for "the full width" measures whatever its monitor measures, and it
                    // can know it: `screen.width`.
                    // A window measures whatever the compositor has given it, and that can change.
                    let is_window = scene.surface().window.is_some();
                    let its_own = n.view.surface == 0 && n.view.popup.is_none();
                    // A named surface publishes what it measures (see `SheetSize`).
                    if let Some((w, h)) = scene.surfaces.get(n.view.surface).and_then(|s| s.size_props).filter(|_| n.view.popup.is_none()) {
                        for (p, v) in [(w, n.size.0 as f32), (h, n.size.1 as f32)] {
                            props[p.0 as usize] = Animated { x: v, v: 0.0, target: v, spring: props[p.0 as usize].spring };
                        }
                    }
                    if (scene.surface().width == 0 || is_window) && its_own {
                        size.0 = n.size.0 as f32;
                    }
                    if is_window && its_own {
                        size.1 = n.size.1 as f32;
                    }
                    let height = if is_window { n.size.1 as f32 } else { scene.surface().height as f32 };
                    for (k, (name, _)) in scene.facts.iter().enumerate() {
                        match *name {
                            "screen.width" => facts[k] = size.0,
                            "screen.height" => facts[k] = height,
                            _ => {}
                        }
                    }
                    // With `screens: each`, each copy knows which monitor it belongs to: its name and
                    // what it measures. It is what lets it show its own things and not the other one's.
                    // Only the copies of the main one: a surface with a name —the
                    // recording corner, the lock— also has instance 0, and when it
                    // arrived later it TRAMPLED `screen.0.name` with its monitor. With the
                    // corner on DP-3, both copies said "DP-3" and the logic did not
                    // know which was which.
                    if let Some(its_own) = scene.surfaces.get(n.view.surface).filter(|s| s.name.is_empty()) {
                        let k = its_own.instance;
                        if let Some(i) = scene.texts.iter().position(|t| t.0 == format!("screen.{k}.name")) {
                            texts.resize(scene.texts.len().max(texts.len()), String::new());
                            texts[i] = n.name.clone();
                            let _ = to_logic.send(Event::Text(scene.texts[i].0, n.name.clone()));
                        }
                        for (part, v) in [("width", n.size.0 as f32), ("height", n.size.1 as f32)] {
                            if let Some(i) = scene.facts.iter().position(|h| h.0 == format!("screen.{k}.{part}")) {
                                facts[i] = v;
                            }
                        }
                    }
                    {
                        // Which surface of the scene it is, not just the sheet number: with
                        // `screens: each` there are several alike and it is worth knowing which landed where.
                        let which = scene.surfaces.get(n.view.surface).map_or(String::new(), |s| match (s.name.as_str(), s.instance) {
                            ("", 0) => String::new(),
                            ("", k) => format!(" · copy {k}"),
                            (name, 0) => format!(" · {name}"),
                            (name, k) => format!(" · {name} copy {k}"),
                        });
                        println!("render · surface {} on {} · {}×{} · scale {} · {:.0} Hz{which}", n.id, n.name, n.size.0, n.size.1, n.scale, n.mhz as f32 / 1000.0);
                    }
                    sheets.push(g.sheet(*n, size));
                    // How many monitors are showing something right now.
                    if let Some(i) = scene.facts.iter().position(|h| h.0 == "screens.count") {
                        let how_many = sheets.iter().filter(|l| l.view.popup.is_none()).map(|l| l.view.surface).collect::<std::collections::HashSet<_>>().len();
                        facts[i] = how_many as f32;
                    }
                    assign_pace(g, &mut sheets, size, op.no_vsync);
                    region = vec![[i32::MIN; 4]];
                }
                ToRender::SheetGone(id) => {
                    sheets.retain(|l| l.id != id);
                    println!("render · surface {id} gone; {} left", sheets.len());
                    if let Some(g) = &gpu {
                        assign_pace(g, &mut sheets, size, op.no_vsync);
                    }
                }
                ToRender::Workshop(p) => letters.receive(*p),
                // A window someone stretches: the sheet changes, and with it what the
                // scene reads in `screen.width` and `screen.height`.
                // A capture of what is behind a glass: the background is unmixed, and if
                // something has changed, that sheet is painted again. And the next one is
                // asked for, which will not arrive until something changes on screen.
                ToRender::Backdrop(d) => {
                    if let Some(g) = &gpu {
                        handle_backdrop(g, &mut sheets, d);
                    }
                }
                ToRender::Frame(id) => {
                    if frame_requested == Some(id) {
                        frame_ready = true;
                    }
                }
                ToRender::SheetSize(id, new_size) => {
                    if let (Some(g), Some(l)) = (&gpu, sheets.iter_mut().find(|l| l.id == id)) {
                        // A named surface publishes what it measures.
                        if let Some((w, h)) = scene.surfaces.get(l.view.surface).and_then(|s| s.size_props) {
                            for (p, v) in [(w, new_size.0), (h, new_size.1)] {
                                props[p.0 as usize] = Animated { x: v, v: 0.0, target: v, spring: props[p.0 as usize].spring };
                            }
                        }
                        if l.view.size != new_size {
                            l.view.size = new_size;
                            g.reconfigure(l, size);
                            if l.view.surface == 0 && l.view.popup.is_none() && scene.surface().window.is_some() {
                                // What is painted outside the surface does not exist: if the
                                // window grows, the frame has to grow with it.
                                size = new_size;
                                for (k, (name, _)) in scene.facts.iter().enumerate() {
                                    match *name {
                                        "screen.width" => facts[k] = new_size.0,
                                        "screen.height" => facts[k] = new_size.1,
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }
                ToRender::Scale(id, e) => {
                    if let (Some(g), Some(l)) = (&gpu, sheets.iter_mut().find(|l| l.id == id)) {
                        if (l.scale - e).abs() > 0.001 {
                            println!("render · surface {id}: scale {} → {e}", l.scale);
                            l.scale = e;
                            g.reconfigure(l, size);
                        }
                    }
                }
                ToRender::Command(Command::Animate(t)) => pending.push((Instant::now() + t.delay, t)),
                ToRender::Command(Command::Impulse { prop, velocity }) => props[prop.0 as usize].v += velocity,
                ToRender::Command(Command::Block(d)) => block = Some(d),
                ToRender::Text(name, value) => match scene.texts.iter().position(|t| t.0 == name) {
                    Some(i) => texts[i] = value,
                    None => eprintln!("render · I don't know the text '{name}'"),
                },
                ToRender::Fact(name, v) => match scene.facts.iter().position(|h| h.0 == name) {
                    Some(i) => facts[i] = v,
                    None => eprintln!("render · I don't know the fact '{name}'"),
                },
                ToRender::LockScreen(held) => {
                    if !held {
                        // It has not granted it, or it has ended it: what was being painted is released
                        // and then the surfaces. It stays noted as requested, so as
                        // not to try again until the scene releases it itself.
                        for &k in &locks {
                            sheets.retain(|l| !(l.view.surface == k && l.view.popup.is_none()));
                            crate::platform::lock_screen(k, None);
                        }
                        if let Some(g) = &gpu {
                            assign_pace(g, &mut sheets, size, op.no_vsync);
                        }
                    }
                    if let Some(k) = scene.facts.iter().position(|(n, _)| *n == "lock.held") {
                        facts[k] = if held { 1.0 } else { 0.0 };
                    }
                }
                ToRender::PopupClosed(k) => {
                    // They have clicked outside: first what was painting in it is released, then it.
                    sheets.retain(|l| l.view.popup != Some(k));
                    crate::platform::popup(k, None);
                    if let Some(g) = &gpu {
                        assign_pace(g, &mut sheets, size, op.no_vsync);
                    }
                    if let Some(open) = popups.get_mut(k) {
                        *open = None;
                    }
                    if let Some(em) = scene.popups.get(k) {
                        let h = em.open.0 as usize;
                        facts[h] = 0.0;
                        let _ = to_logic.send(Event::Fact(scene.facts[h].0, 0.0));
                    }
                }
                ToRender::ExternalFact(name, written) => match scene.facts.iter().position(|h| h.0 == name) {
                    Some(i) => {
                        let kind = scene.types.iter().find(|(n, _)| n == name).map(|(_, t)| t);
                        let v = kind.and_then(|t| t.from_text(&written)).or_else(|| written.parse().ok()).or(match written.as_str() { "true" => Some(1.0), "false" => Some(0.0), _ => None });
                        match v {
                            Some(v) => {
                                facts[i] = v;
                                // The logic did not set it: let it know.
                                let _ = to_logic.send(Event::Fact(name, v));
                            }
                            None => eprintln!("render · '{name}' cannot be '{written}'"),
                        }
                    }
                    None => eprintln!("render · I don't know the fact '{name}'"),
                },
                ToRender::Query(name, reply_to) => {
                    let number = |v: f32| if v.fract() == 0.0 { format!("{}", v as i64) } else { format!("{v}") };
                    let r = if let Some(i) = scene.facts.iter().position(|h| h.0 == name) {
                        // As it would be written: `true`, `critical`, or the number.
                        match scene.types.iter().find(|(n, _)| n == name) {
                            Some((_, t)) => t.to_text(facts[i]),
                            None => number(facts[i]),
                        }
                    } else if let Some(i) = scene.texts.iter().position(|t| t.0 == name) {
                        texts.get(i).cloned().unwrap_or_default()
                    } else if let Some(i) = scene.props.iter().position(|p| p.0 == name) {
                        number(props[i].x)
                    } else {
                        format!("? I don't know '{name}'")
                    };
                    let _ = reply_to.send(r);
                }
                ToRender::Signal(name) => match scene.signals.iter().position(|s| s.0 == name) {
                    Some(i) => signals.push((i, true, None)),
                    None => eprintln!("render · I don't know the event '{name}'"),
                },
                ToRender::ExternalSignal(name, payload) => match scene.signals.iter().position(|s| s.0 == name) {
                    Some(i) => signals.push((i, false, payload)),
                    None => eprintln!("render · I don't know the event '{name}'"),
                },
                ToRender::Gesture(name) => match scene.gestures.iter().position(|g| g.name == name) {
                    Some(i) => gestures_asked.push(i),
                    None => eprintln!("render · I don't know the gesture '{name}'"),
                },
                // The mouse moving anywhere is someone there: for `idle`, it counts
                // as much as moving it over the scene.
                ToRender::Cursor(at, monitors) => {
                    if cursor.as_ref().is_some_and(|c| c.0 != at) {
                        last_activity = Instant::now();
                    }
                    cursor = Some((at, monitors));
                }
                ToRender::Pointer(p) => {
                    pointer = p;
                    last_activity = Instant::now();
                }
                ToRender::Button(b, down) => {
                    buttons.push((b, down));
                    if b == 0 {
                        finger_down = down;
                        if let (true, Some(p)) = (down, pointer) {
                            finger = p;
                            finger_light = 1.0;
                            if !op.reduced_motion {
                                ripple = Some((p, Instant::now()));
                            }
                        }
                    }
                    last_activity = Instant::now();
                }
                ToRender::Wheel(d) => {
                    wheel += d;
                    last_activity = Instant::now();
                }
                ToRender::KeyRepeat(r) => key_repeat = r,
                ToRender::Key(name, typed, mods) => {
                    last_activity = Instant::now();
                    // If it is held down, it repeats, however this user has it set up.
                    repeat = key_repeat.map(|(delay, _)| (name.clone(), typed.clone(), mods, Instant::now() + Duration::from_millis(delay as u64)));
                    key_presses.push((name, typed, mods));
                }
                ToRender::KeyReleased(name) => {
                    if repeat.as_ref().is_some_and(|r| r.0 == name) {
                        repeat = None;
                    }
                }
                ToRender::KeyboardFocus(yes) => focus_changes.push(yes),
                ToRender::FocusField(name) => {
                    editing = name.and_then(|n| scene.texts.iter().position(|t| t.0 == n)).map(|k| Editing { field: k, cursor: texts[k].len(), anchor: texts[k].len() });
                    last_key = Instant::now();
                }
                ToRender::Dropped(kind, data) => drops.push((kind, data)),
                ToRender::Quit => {
                    cycle.close();
                    return;
                }
            }
        }
        // The key that is still held down counts again.
        if let Some((name, typed, mods, when)) = &mut repeat {
            if Instant::now() >= *when {
                key_presses.push((name.clone(), typed.clone(), *mods));
                *when = Instant::now() + Duration::from_millis(key_repeat.map_or(33, |r| r.1) as u64);
            }
        }
        let mut submitted: Vec<usize> = Vec::new();
        for (name, typed, mods) in key_presses {
            // The field first: whatever is typing belongs to it. The rest —Escape, a
            // shortcut— goes on to the rules and to the logic.
            if let Some(ed) = &mut editing {
                let k = ed.field;
                match ed.handle_key(&mut texts[k], &name, typed.as_deref(), mods) {
                    KeyOutcome::Changed => {
                        last_key = Instant::now();
                        let _ = to_logic.send(Event::Text(scene.texts[k].0, texts[k].clone()));
                        continue;
                    }
                    KeyOutcome::Moved => {
                        last_key = Instant::now();
                        continue;
                    }
                    KeyOutcome::Submitted => {
                        submitted.push(k);
                        let _ = to_logic.send(Event::Submit(scene.texts[k].0, texts[k].clone()));
                        continue;
                    }
                    KeyOutcome::Unhandled => {}
                }
            }
            let mut combo = String::new();
            for (on, prefix) in [(mods.ctrl, "Ctrl+"), (mods.alt, "Alt+"), (mods.logo, "Super+")] {
                if on {
                    combo.push_str(prefix);
                }
            }
            combo.push_str(&name);
            let _ = to_logic.send(Event::Key(combo.clone(), typed));
            keys.push(combo);
        }
        for gained in &focus_changes {
            have_keyboard = *gained;
            let _ = to_logic.send(Event::Focus(*gained));
            if *gained {
                // On gaining the keyboard, if there is somewhere to type and nobody has it, the first one.
                if editing.is_none() {
                    editing = scene.instrs.iter().find_map(|i| if let Instr::Field { text, .. } = i { Some(text.0 as usize) } else { None }).map(|k| Editing { field: k, cursor: texts[k].len(), anchor: texts[k].len() });
                }
            } else {
                editing = None;
                repeat = None;
            }
        }

        if let Some(d) = block {
            // Naive mode: the logic's work happens here, on the thread
            // that paints. It is what happens in QtQuick with a heavy handler.
            if op.naive {
                logic_blocked.store(true, Ordering::Relaxed);
                std::thread::sleep(d);
                logic_blocked.store(false, Ordering::Relaxed);
            }
        }

        // ── 2. advance time ─────────────────────────────────────
        let now = Instant::now();
        let dt = (now - last).as_secs_f32();
        last = now;
        let t_total = (now - start).as_secs_f32();
        let mut effects: Vec<Effect> = Vec::new();
        let mut keyboard_changed = false;
        let mut appointments: Vec<Instant> = Vec::new();

        // Zones: who has the mouse over them.
        let mut edges: Vec<(bool, usize)> = Vec::new(); // (enters, zone)
        {
            let c = Ctx { props: &props, facts: &facts };
            for (k, (z, was_inside)) in scene.zones.iter().zip(inside.iter_mut()).enumerate() {
                let is_inside = z.active.is_true(c) && pointer.is_some_and(|(x, y)| z.contains(c, x, y));
                if is_inside != *was_inside {
                    *was_inside = is_inside;
                    edges.push((is_inside, k));
                    let _ = to_logic.send(if is_inside { Event::Enter(z.id) } else { Event::Leave(z.id) });
                }
            }
        }
        // The topmost one is the last declared.
        let hovered = inside.iter().rposition(|d| *d);
        if std::env::var_os("PLEAMAR_DEBUG_ZONES").is_some() && !edges.is_empty() {
            let names: Vec<&str> = inside.iter().enumerate().filter(|(_, d)| **d).map(|(k, _)| scene.zones[k].id).collect();
            eprintln!("zones  · under the pointer: {names:?}");
        }
        let (mut pressed, mut pressed_with, mut released) = (None, None, None);
        for (button, down) in &buttons {
            match (*button, *down) {
                (0, true) => {
                    pressed = hovered;
                    if let (Some(k), Some(p)) = (hovered, pointer) {
                        // Clicking a field focuses it, with the cursor where the click landed.
                        let id = scene.zones[k].id;
                        if let Some(placed) = draw.fields.iter().find(|c| c.zone == id) {
                            let local = scene.zones[k].to_local(Ctx { props: &props, facts: &facts }, p.0, p.1);
                            let b = placed.layout.as_ref().map_or(0, |m| m.byte_at_x(local.0 - placed.x0 + placed.scroll));
                            let b = if placed.secret { crate::gpu::unmasked_byte(&texts[placed.text], b) } else { b };
                            let b = b.min(texts[placed.text].len());
                            editing = Some(Editing { field: placed.text, cursor: b, anchor: b });
                            last_key = now;
                        }
                        drag = Some((k, p, now));
                        let _ = to_logic.send(Event::Press(scene.zones[k].id));
                    }
                }
                (0, false) => {
                    if let Some((k, _, _)) = drag.take() {
                        released = Some(k);
                        if let Some(z) = scene.zones.get(k) {
                            let _ = to_logic.send(Event::Release(z.id));
                        }
                    }
                }
                (b, true) => pressed_with = hovered.map(|k| (k, b)),
                _ => {}
            }
        }
        // A prototype's emergency exit —the right button closes— must not
        // steal the button from a scene that uses it. What knows whether it has landed
        // on top of something is this, which is what looks at the zones: if there was none under the
        // pointer, it closes; if there was, the click belongs to the scene.
        // The platform already closes on its own when NO surface uses the
        // right button, and then this does not even run.
        let dragged = match (drag, pointer) {
            (Some((k, _, _)), Some(p)) if last_pointer != Some(p) => Some(k),
            _ => None,
        };
        last_pointer = pointer;
        // The wheel is not only for the topmost zone: it works for any zone it has
        // underneath, because a whole pill wants the wheel even if there is a button inside.
        if wheel != 0.0 {
            if let Some(k) = hovered {
                let _ = to_logic.send(Event::Wheel(scene.zones[k].id, wheel));
            }
        }

        // Each zone's own springs: over it, and pressed on it.
        for (z, hover, pressed) in &scene.zone_springs {
            let k = z.0 as usize;
            props[hover.0 as usize].target = if inside.get(k).copied().unwrap_or(false) { 1.0 } else { 0.0 };
            props[pressed.0 as usize].target = if drag.is_some_and(|d| d.0 == k) { 1.0 } else { 0.0 };
        }

        // What a rule can read from the mouse: where it is, where inside the zone
        // it is dealing with, how far it has dragged and how much the wheel has turned.
        {
            let focus = drag.map(|a| a.0).or(hovered);
            let (px, py) = pointer.unwrap_or((0.0, 0.0));
            // The mouse wherever it is, in the plane of the scene. It is placed
            // against a surface that is open —the one on the mouse's monitor if
            // there is one, else the first—: from there on it is just a sum, even
            // if it is on another monitor. Over the scene, the pointer itself.
            // Each open sheet placed on the desktop: which surface it is, whether
            // the mouse is on its monitor, and the mouse from its own corner.
            let placed: Vec<(usize, bool, (f32, f32))> = match (scene.wants_cursor, &cursor) {
                (true, Some(((gx, gy), monitors))) => {
                    let on = |m: &[i32; 4]| *gx >= m[0] as f32 && *gy >= m[1] as f32 && *gx < (m[0] + m[2]) as f32 && *gy < (m[1] + m[3]) as f32;
                    let under = monitors.iter().find(|m| on(&m.1)).map(|m| m.0.as_str());
                    sheets.iter().filter(|l| l.open && l.view.popup.is_none()).filter_map(|l| {
                        let (name, at) = l.desktop_place()?;
                        let m = monitors.iter().find(|m| m.0 == name)?.1;
                        let corner = ((m[0] + at.0) as f32, (m[1] + at.1) as f32);
                        Some((l.view.surface, under == Some(name.as_str()), (gx - corner.0, gy - corner.1)))
                    }).collect()
                }
                _ => Vec::new(),
            };
            // Of a set of sheets, the one on the mouse's monitor, or else the first.
            let pick = |of: &dyn Fn(usize) -> bool| placed.iter().filter(|p| of(p.0)).fold(None, |best: Option<&(usize, bool, (f32, f32))>, p| match best { Some(b) if b.1 || !p.1 => Some(b), _ => Some(p) });
            // `cursor.x` belongs to the scene's surface: its plane is the one the
            // loose drawing uses. The named ones have their own, below.
            let seen_cursor = match pointer {
                Some(p) => p,
                None => match pick(&|k| scene.surfaces.get(k).is_some_and(|s| s.name.is_empty())).or_else(|| pick(&|_| true)) {
                    // From its corner to the plane: plus where the sheet looks from.
                    Some(&(k, _, (x, y))) => {
                        let origin = sheets.iter().find(|l| l.view.surface == k && l.open).map_or((0.0, 0.0), |l| l.view.origin);
                        (origin.0 + x, origin.1 + y)
                    }
                    None => (f32::NAN, f32::NAN),
                },
            };
            // `panel.cursor.x`: from that surface's corner, which is what it draws from.
            for (k, s) in scene.surfaces.iter().enumerate() {
                let Some((px_, py_)) = s.cursor_props else { continue };
                if scene.surfaces.iter().position(|o| o.cursor_props == s.cursor_props) != Some(k) {
                    continue;
                }
                let same = |j: usize| scene.surfaces.get(j).is_some_and(|o| o.cursor_props == s.cursor_props);
                if let Some(p) = pick(&same) {
                    for (id, v) in [(px_, p.2 .0), (py_, p.2 .1)] {
                        props[id.0 as usize] = Animated { x: v, v: 0.0, target: v, spring: props[id.0 as usize].spring };
                    }
                }
            }
            // Unknown, it stays where it was.
            let seen_cursor = if seen_cursor.0.is_nan() {
                let k = |n: &str| scene.facts.iter().position(|f| f.0 == n).map_or(0.0, |k| facts[k]);
                (k("cursor.x"), k("cursor.y"))
            } else {
                seen_cursor
            };
            let local = focus.and_then(|k| scene.zones.get(k)).map_or((px, py), |z| z.to_local(Ctx { props: &props, facts: &facts }, px, py));
            let (dx, dy) = drag.map_or((0.0, 0.0), |(_, o, _)| (px - o.0, py - o.1));
            for (k, (name, _)) in scene.facts.iter().enumerate() {
                match *name {
                    "pointer.x" => facts[k] = px,
                    "pointer.y" => facts[k] = py,
                    "cursor.x" => facts[k] = seen_cursor.0,
                    "cursor.y" => facts[k] = seen_cursor.1,
                    "local.x" => facts[k] = local.0,
                    "local.y" => facts[k] = local.1,
                    "drag.dx" => facts[k] = dx,
                    "drag.dy" => facts[k] = dy,
                    "wheel" => facts[k] = wheel,
                    _ => {}
                }
            }
        }

        for (kind, data) in &drops {
            if let Some(k) = hovered {
                let _ = to_logic.send(Event::Received(scene.zones[k].id, kind.clone(), data.clone()));
            }
        }

        // The keyboard, only while the scene wants it: a closed launcher cannot
        // keep it. And if it asked for the focus for a field without having it,
        // exclusively until it arrives (see `Effect::FocusField`).
        let keyboard_wanted = match &scene.keyboard_while {
            Some(when) => when.is_true(Ctx { props: &props, facts: &facts }),
            None => true,
        };
        // The loan lasts while the scene wants the keyboard: giving it back on
        // receiving the focus does not work, because on going back to "on demand" Hyprland
        // gives it again to the previous window. It is what any launcher does:
        // open, the keyboard is its own; Esc closes it and releases it.
        //
        // And it outlives a short gap: the finder closes, and 180 ms later the
        // card it opened comes out. Released in between, the keyboard went back
        // to the window below and the card was left without it —Esc did not
        // reach it until the mouse went over—. So on not being wanted the loan
        // is kept 300 ms more; if something wants it again by then, it goes on.
        if !keyboard_wanted {
            if keyboard_lent.is_some() {
                let since = *loan_grace.get_or_insert(now);
                if now.duration_since(since) >= Duration::from_millis(300) {
                    keyboard_lent = None;
                    loan_grace = None;
                } else {
                    appointments.push(since + Duration::from_millis(300));
                }
            }
        } else {
            loan_grace = None;
        }
        let mode = if keyboard_lent.is_some() {
            Keyboard::Always
        } else if !keyboard_wanted {
            Keyboard::Never
        } else {
            scene.surface().keyboard
        };
        // With no condition and no loan, the keyboard is the one asked for when creating it.
        // Each surface gets it while it is OPEN, and no other: a closed one
        // never asks for the keyboard. Handed to all of them, a closed
        // full-screen catcher or the copy on the other monitor took it —an
        // exclusive keyboard asked for by the finder went to one of them, and
        // nothing could be typed in the finder—.
        if keyboard_set.is_some() || scene.keyboard_while.is_some() || keyboard_lent.is_some() {
            keyboard_set = Some(mode);
            for l in &mut sheets {
                let its = if l.open || l.view.popup.is_some() { mode } else { Keyboard::Never };
                if l.keyboard_mode != Some(its) {
                    l.keyboard(its);
                    l.keyboard_mode = Some(its);
                    // It is applied with the frame that gets presented: let there be one.
                    l.painted = None;
                    keyboard_changed = true;
                }
            }
        }

        // The cursor, the one of the zone it is over.
        let wanted = drag.map(|a| a.0).or(hovered).and_then(|k| scene.zones.get(k)).map_or(Cursor::Normal, |z| z.cursor);
        if wanted != cursor_set {
            cursor_set = wanted;
            for l in &sheets {
                l.cursor(wanted);
            }
        }

        // With `screens: each`, the things of a copy whose surface is closed are not
        // looked at: neither its rules nor its drawing. Its behaviors YES: a `follow`
        // of its own is what opens it —`open: nothing.here > 0.01` chases a fact—,
        // and asleep it would never come back. They are cheap; the expensive part is reading the drawing.
        // Without this two copies were twice the scene per frame even if one was not
        // visible.
        let asleep: Vec<&crate::scene::Span> = {
            let c = Ctx { props: &props, facts: &facts };
            scene.spans.iter().filter(|t| scene.surfaces.get(t.surface).and_then(|s| s.open.as_ref()).is_some_and(|e| !e.is_true(c))).collect()
        };
        let rule_asleep = |k: usize| asleep.iter().any(|t| t.rules.contains(&k));

        // Rules: all of this happens here, whatever state the logic is in.
        {
            let c = Ctx { props: &props, facts: &facts };
            let mut acted: Vec<usize> = Vec::new();
            for (k, (r, e)) in scene.rules.iter().zip(rules.iter_mut()).enumerate() {
                if rule_asleep(k) {
                    continue;
                }
                let fires = match &r.when {
                    Trigger::Enter(z) => edges.contains(&(true, z.0 as usize)),
                    Trigger::Leave(z) => edges.contains(&(false, z.0 as usize)),
                    Trigger::Press(z) => pressed == Some(z.0 as usize),
                    Trigger::PressWith(z, b) => pressed_with == Some((z.0 as usize, *b)),
                    Trigger::Release(z) => released == Some(z.0 as usize),
                    Trigger::Wheel(z) => wheel != 0.0 && inside[z.0 as usize],
                    // Dragging is not only for the topmost zone, like the wheel: it works
                    // for any zone that was underneath when it was pressed. That way a list
                    // can be dragged by grabbing one of its rows.
                    Trigger::Drag(z) => {
                        dragged.is_some()
                            && match (scene.zones.get(z.0 as usize), drag) {
                                (Some(zone), Some((_, o, _))) => zone.contains(c, o.0, o.1),
                                _ => false,
                            }
                    }
                    Trigger::Hold { zone, duration } => {
                        let held = drag.is_some_and(|(k, _, _)| k == zone.0 as usize);
                        e.sustained(held, *duration, now, &mut appointments)
                    }
                    // `on change floor(list.scroll / 34) { … }`: when that changes.
                    Trigger::Change(x) => {
                        let now = x.eval(c);
                        let before = e.last_value.replace(now);
                        before.is_some_and(|v| (v - now).abs() > 0.001)
                    }
                    // `on still audio.volume for 1.1s { … }`: when that has been still for that
                    // long. Each change resets the clock to zero, so six
                    // taps in a row on the volume key are a single wait.
                    Trigger::Still { value, duration } => {
                        let value = value.eval(c);
                        let before = e.last_value.replace(value);
                        if before.is_some_and(|v| (v - value).abs() > 0.001) {
                            e.armed = true;
                            e.next = Some(now + *duration);
                        }
                        match e.next.filter(|_| e.armed) {
                            Some(p) if now >= p => {
                                e.armed = false;
                                true
                            }
                            Some(p) => {
                                appointments.push(p);
                                false
                            }
                            None => false,
                        }
                    }
                    Trigger::Key(t) => keys.iter().any(|x| x == t),
                    Trigger::Submit(t) => submitted.contains(&(t.0 as usize)),
                    Trigger::FocusGained => focus_changes.contains(&true),
                    Trigger::FocusLost => focus_changes.contains(&false),
                    Trigger::Receive(z) => !drops.is_empty() && inside[z.0 as usize],
                    Trigger::Above { zone, duration } => {
                        e.sustained(inside[zone.0 as usize], *duration, now, &mut appointments)
                    }
                    Trigger::Away { zone, duration } => {
                        let is_inside = inside[zone.0 as usize];
                        e.armed |= is_inside;
                        let done = e.sustained(e.armed && !is_inside, *duration, now, &mut appointments);
                        if done {
                            e.armed = false;
                        }
                        done
                    }
                    Trigger::Idle { duration, during } => {
                        let left = (last_activity + *duration).saturating_duration_since(now);
                        let ready = during.is_true(c) && left.is_zero();
                        if during.is_true(c) && !left.is_zero() {
                            appointments.push(last_activity + *duration);
                        }
                        let done = ready && e.last_time != Some(last_activity);
                        if done {
                            e.last_time = Some(last_activity);
                        }
                        done
                    }
                    Trigger::Every { between, during } => {
                        // While the condition is not met, the wait does NOT count: the
                        // clock is set in full every frame, so it starts counting
                        // when it starts being true. Before, it ran on its own and
                        // `every 1s while counting` took its first step at 300 ms —whatever
                        // was left of the previous clock—, which in a countdown is
                        // a second that does not exist.
                        if !during.is_true(c) {
                            e.next = Some(now + Duration::from_secs_f32(rng.between(between.0, between.1)));
                            false
                        } else {
                            let due = e.next.is_some_and(|p| now >= p);
                            if due {
                                e.next = Some(now + Duration::from_secs_f32(rng.between(between.0, between.1)));
                            }
                            if let Some(p) = e.next {
                                appointments.push(p);
                            }
                            due
                        }
                    }
                    Trigger::On(_) => false, // handled with the signals, below
                };
                // Its twins in the other copies watched too —each one keeps its own
                // `on change`, its own `on still`—, but only the first awake acts.
                let twin = scene.twin_of.get(k).copied().unwrap_or(k);
                if fires && !acted.contains(&twin) && r.guard.as_ref().is_none_or(|guard| guard.is_true(c)) {
                    acted.push(twin);
                    effects.extend(r.effects.iter().cloned());
                }
            }
        }

        // Effects and signals, until there are none left (with a cap: a rule
        // that fires itself does not hang the render).
        //
        // And if something has happened, it does not go to sleep yet: the rules are all looked at
        // before applying anything, so an `on change` cannot see in the same
        // frame what another rule has just changed. It sees it on the next one, and without
        // this "the next one" could be a second later —the render asleep
        // until the next appointment—, which is how a countdown skipped its
        // end.
        let something_happened = !effects.is_empty() || !signals.is_empty() || !gestures_asked.is_empty();
        let mut cap = 8;
        while (!effects.is_empty() || !signals.is_empty() || !gestures_asked.is_empty()) && cap > 0 {
            cap -= 1;
            for ef in std::mem::take(&mut effects) {
                match ef {
                    Effect::Animate(t) => pending.push((now + t.delay, t)),
                    // A fact that a rule changes is told to the logic, which otherwise
                    // would go on believing what it said itself the last time.
                    Effect::Fact(h, v) => {
                        let v = v.eval(Ctx { props: &props, facts: &facts });
                        facts[h.0 as usize] = v;
                        let _ = to_logic.send(Event::Fact(scene.facts[h.0 as usize].0, v));
                    }
                    Effect::Toggle(h) => {
                        let v = if facts[h.0 as usize] > 0.5 { 0.0 } else { 1.0 };
                        facts[h.0 as usize] = v;
                        let _ = to_logic.send(Event::Fact(scene.facts[h.0 as usize].0, v));
                    }
                    Effect::Signal(s, payload) => {
                        let v = payload.as_ref().map(|e| e.eval(Ctx { props: &props, facts: &facts }));
                        signals.push((s.0 as usize, false, v));
                    }
                    Effect::Impulse(p, v) => {
                        let v = v.eval(Ctx { props: &props, facts: &facts });
                        props[p.0 as usize].v += v;
                    }
                    Effect::Gesture(g) => gestures_asked.push(g.0 as usize),
                    Effect::FocusField(t) => {
                        editing = t.map(|t| t.0 as usize).map(|k| Editing { field: k, cursor: texts[k].len(), anchor: texts[k].len() });
                        last_key = now;
                        // Focusing a field is wanting to type right away. With the keyboard "on
                        // demand", the compositor only gives it with a click, and a search box
                        // that opens with a shortcut never receives it: what is typed
                        // goes to another window and Esc does not close it. It is asked for exclusively
                        // while the scene keeps wanting the keyboard.
                        if t.is_some() && !have_keyboard && scene.surface().keyboard == Keyboard::OnDemand {
                            keyboard_lent = Some(now);
                        }
                    }
                }
            }
            for (s, from_logic, payload) in std::mem::take(&mut signals) {
                let id = SignalId(s as u16);
                // When it last happened: what a `burst:` of particles starts from.
                if draw.signal_times.len() != scene.signals.len() {
                    draw.signal_times = vec![-1.0; scene.signals.len()];
                }
                draw.signal_times[s] = t_total;
                for (layer, st) in scene.layers.iter().zip(layers.iter_mut()) {
                    for (k, r) in layer.claims.iter().enumerate() {
                        match &r.when {
                            When::After { signals: ss, duration } if ss.contains(&id) => st.until[k] = Some(now + *duration),
                            When::FromUntil { from, until } => {
                                if from.contains(&id) {
                                    st.lit[k] = true;
                                } else if until.contains(&id) {
                                    st.lit[k] = false;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                for (k, r) in scene.rules.iter().enumerate() {
                    // One signal, one answer per rule: not one per monitor.
                    if scene.twin_of.get(k).is_some_and(|&t| t != k) {
                        continue;
                    }
                    if matches!(&r.when, Trigger::On(x) if *x == id) && r.guard.as_ref().is_none_or(|guard| guard.is_true(Ctx { props: &props, facts: &facts })) {
                        effects.extend(r.effects.iter().cloned());
                    }
                }
                let (name, goes_out) = scene.signals[s];
                if goes_out && !from_logic {
                    let _ = to_logic.send(Event::Signal(name, payload));
                }
            }
            // Gestures: one only cuts off another of its class or lower.
            for g in std::mem::take(&mut gestures_asked) {
                let requested = &scene.gestures[g];
                let current = gesture.as_ref().map(|r| scene.gestures[r.gesture].class);
                if current.is_some_and(|c| c > requested.class) {
                    let _ = to_logic.send(Event::GestureRejected(requested.name));
                    continue;
                }
                gesture = Some(Playback::start(g, requested, &scene.pose, Ctx { props: &props, facts: &facts }, now, op.reduced_motion));
            }
        }

        pending.retain(|(when, t)| {
            if *when <= now {
                // The destination is computed now, not when it was declared.
                let target = t.to.eval(Ctx { props: &props, facts: &facts });
                let a = &mut props[t.prop.0 as usize];
                a.target = target;
                a.spring = t.spring;
                if op.reduced_motion {
                    a.settle();
                }
                false
            } else {
                true
            }
        });
        appointments.extend(pending.iter().map(|p| p.0));

        // Layers: the first claim that holds wins.
        for (layer, st) in scene.layers.iter().zip(layers.iter_mut()) {
            let c = Ctx { props: &props, facts: &facts };
            let winner = layer
                .claims
                .iter()
                .enumerate()
                .position(|(k, r)| match &r.when {
                    When::Always => true,
                    When::While(e) => e.is_true(c),
                    When::After { .. } => st.until[k].is_some_and(|h| now < h),
                    When::FromUntil { .. } => st.lit[k],
                })
                .unwrap_or(layer.claims.len().saturating_sub(1));
            appointments.extend(st.until.iter().flatten().filter(|h| **h > now));
            if st.winner != Some(winner) {
                // Freshly loaded, a layer settles where it belongs. Hot reloaded,
                // no: if the file changed a destination, it goes towards it with its spring.
                let first_time = st.winner.is_none() && !st.hot;
                let silent = st.winner.is_none();
                let before = st.winner.map_or("—", |k| layer.claims[k].name);
                st.winner = Some(winner);
                let r = &layer.claims[winner];
                for (k, p) in layer.presences.iter().enumerate() {
                    let a = &mut props[p.0 as usize];
                    a.target = if k == winner { 1.0 } else { 0.0 };
                    if first_time || op.reduced_motion {
                        a.settle();
                    }
                }
                for t in &r.sets {
                    if first_time {
                        let v = t.to.eval(Ctx { props: &props, facts: &facts });
                        props[t.prop.0 as usize].set(v);
                    } else {
                        pending.push((now + t.delay, t.clone()));
                    }
                }
                if !first_time && !silent {
                    println!("layer {} · {} → {}", layer.name, before, r.name);
                    let _ = to_logic.send(Event::Layer(layer.name, r.name));
                }
            }
        }

        // Behaviors: the scene's own life. Whatever a gesture is
        // moving right now they do not touch: a gesture rules over the ambient.
        let from_a_gesture: Vec<PropId> = gesture
            .as_ref()
            .map(|r| scene.gestures[r.gesture].keyframes.iter().flat_map(|f| f.values.iter().map(|(p, _)| *p)).collect())
            .unwrap_or_default();
        let mut alive = false;
        let mut n_blink = 0;
        for behavior in &scene.behaviors {
            match behavior {
                Behavior::Blink { prop, every, duration } => {
                    let (next, since) = &mut blinks[n_blink];
                    n_blink += 1;
                    // While a gesture is leading this same pose by the hand, the ambient
                    // keeps quiet: the gesture already says what the eyelids do. And its clock
                    // stops with it, so that when it ends it does not suddenly fire what was
                    // due halfway through the gesture.
                    if from_a_gesture.contains(prop) {
                        *next += Duration::from_secs_f32(dt);
                        *since = None;
                        continue;
                    }
                    // With reduced motion the ambient keeps quiet: a blink
                    // stays with the eye open. Springs settle and gestures
                    // show their still face; what goes on its own and in a loop is exactly
                    // what must not keep going round.
                    if op.reduced_motion {
                        *since = None;
                        props[prop.0 as usize].set(1.0);
                        continue;
                    }
                    if since.is_none() && now >= *next {
                        *since = Some(now);
                    }
                    if let Some(d) = *since {
                        let t = (now - d).as_secs_f32() / duration;
                        if t >= 1.0 {
                            *since = None;
                            // The period is counted from start to start: "every 5.2 s" is
                            // that, not 5.2 s **after** closing the eye.
                            let following = d + Duration::from_secs_f32(rng.between(every.0, every.1));
                            *next = following.max(now);
                            props[prop.0 as usize].set(1.0);
                        } else {
                            props[prop.0 as usize].set((2.0 * t - 1.0).abs().powf(1.6));
                            alive = true;
                        }
                    }
                    appointments.push(*next);
                }
                Behavior::Wave { prop, frequency, amplitude } => {
                    if from_a_gesture.contains(prop) {
                        continue;
                    }
                    // And a wave stays at its rest —halfway through the journey—, which
                    // is where it was when it started.
                    if op.reduced_motion {
                        props[prop.0 as usize].set(0.0);
                        continue;
                    }
                    let a = amplitude.eval(Ctx { props: &props, facts: &facts });
                    props[prop.0 as usize].set(a * (t_total * frequency).sin());
                    alive |= a.abs() > 0.01;
                }
                Behavior::Follow { prop, to } => {
                    let v = to.eval(Ctx { props: &props, facts: &facts });
                    props[prop.0 as usize].target = v;
                }
                Behavior::Bind { prop, to } => {
                    let v = to.eval(Ctx { props: &props, facts: &facts });
                    let p = &mut props[prop.0 as usize];
                    alive |= (p.x - v).abs() > 1e-3;
                    p.set(v);
                }
                Behavior::Advance { prop, per_second } => {
                    // A spin has no rest to return to: it stays where it is going.
                    if op.reduced_motion {
                        continue;
                    }
                    let v = per_second.eval(Ctx { props: &props, facts: &facts });
                    // Stopped, it lets the property be: a rule can send it
                    // somewhere. Pinning it every frame at speed 0 undid every
                    // `x: 0` —a reading clock that was never reset—.
                    if v.abs() < 1e-9 {
                        continue;
                    }
                    // Turning, it carries the property and where it is going
                    // together: a rule that sends it back to 0 midway is obeyed,
                    // and it goes on turning from there.
                    let a = &mut props[prop.0 as usize];
                    a.x += v * dt;
                    a.target += v * dt;
                    alive |= v.abs() > 1e-4;
                }
                Behavior::Gaze { x, y, center, reach, distance, rest } => {
                    let c = Ctx { props: &props, facts: &facts };
                    let (mx, my) = match pointer {
                        Some((px, py)) => {
                            let (dx, dy) = (px - center.0.eval(c), py - center.1.eval(c));
                            let far = dx.hypot(dy).max(1.0);
                            let pull = (far / distance).min(1.0);
                            (dx / far * reach.0 * pull, dy / far * reach.1 * pull)
                        }
                        None => (rest.0.eval(c), rest.1.eval(c)),
                    };
                    props[x.0 as usize].target = mx;
                    props[y.0 as usize].target = my;
                }
            }
        }

        // Postures: gestures that repeat on their own while something is true.
        if gesture.is_none() {
            let c = Ctx { props: &props, facts: &facts };
            if let Some((g, _)) = scene.postures.iter().find(|(_, e)| e.is_true(c)) {
                let k = g.0 as usize;
                gesture = Some(Playback::start(k, &scene.gestures[k], &scene.pose, Ctx { props: &props, facts: &facts }, now, op.reduced_motion));
            }
        }

        for a in &mut props {
            a.step(dt);
        }

        // The gesture leads the pose's properties by the hand; when it ends
        // it lets them go and their springs bring them back to the base.
        let mut gesture_ended = false;
        if let Some(r) = &mut gesture {
            alive = true;
            let g = &scene.gestures[r.gesture];
            loop {
                let f = &g.keyframes[r.keyframe];
                let elapsed = (now - r.started_at).as_secs_f32() * 1000.0;
                if elapsed < (f.ms + f.hold) as f32 {
                    break;
                }
                // It has arrived: whatever comes next starts from where this one ends.
                for (k, p) in scene.pose.iter().enumerate() {
                    r.from[k] = r.target(r.keyframe, *p, &props);
                }
                r.started_at += Duration::from_millis((f.ms + f.hold) as u64);
                r.keyframe += 1;
                if r.keyframe >= g.keyframes.len() {
                    break;
                }
                if let Some(s) = g.keyframes[r.keyframe].emit {
                    late_signals.push(s);
                }
            }
            if r.keyframe >= g.keyframes.len() {
                for (k, p) in scene.pose.iter().enumerate() {
                    let a = &mut props[p.0 as usize];
                    a.x = if op.reduced_motion { a.target } else { r.from[k] };
                    a.v = 0.0;
                }
                gesture_ended = true;
            } else {
                let f = &g.keyframes[r.keyframe];
                let t = if f.ms == 0 { 1.0 } else { (now - r.started_at).as_secs_f32() * 1000.0 / f.ms as f32 };
                let progress = if r.still.is_some() { 1.0 } else { f.curve.apply(t) };
                let of = r.still.unwrap_or(r.keyframe);
                for (k, p) in scene.pose.iter().enumerate() {
                    let to = r.target(of, *p, &props);
                    let a = &mut props[p.0 as usize];
                    a.x = r.from[k] + (to - r.from[k]) * progress;
                    a.v = 0.0;
                }
            }
        }
        if gesture_ended {
            gesture = None;
        }

        // The popups: open while their fact is true, where and how their expressions
        // say. If they change place or size while open, they are remade.
        popups.resize(scene.popups.len(), None);
        // What is drawn exists if it falls on some live surface, or on some open
        // popup. A closed surface does not contribute its own: that way all of its things are discarded when
        // composing and its next frame comes out empty, which is what makes it disappear.
        let open = |k: usize| scene.surfaces.get(k).and_then(|s| s.open.as_ref()).is_none_or(|e| e.is_true(Ctx { props: &props, facts: &facts }));
        // The lock ones are not put up: they are engaged when their `open:` becomes
        // true and removed when it stops being so. When removing them, first what was
        // being painted on them is released and then them, like a popup.
        let want: Vec<(usize, bool)> = scene.surfaces.iter().enumerate().filter(|(_, s)| s.lock_screen).map(|(k, _)| (k, open(k))).collect();
        for (k, wants) in want {
            let was = locks.contains(&k);
            if wants && !was {
                locks.push(k);
                let s = &scene.surfaces[k];
                crate::platform::lock_screen(k, Some(((s.width, s.height), s.origin)));
            } else if !wants && was {
                locks.retain(|x| *x != k);
                sheets.retain(|l| !(l.view.surface == k && l.view.popup.is_none()));
                crate::platform::lock_screen(k, None);
                if let Some(g) = &gpu {
                    assign_pace(g, &mut sheets, size, op.no_vsync);
                }
                if let Some(h) = scene.facts.iter().position(|(n, _)| *n == "lock.held") {
                    facts[h] = 0.0;
                }
            }
        }
        let open = |k: usize| scene.surfaces.get(k).and_then(|s| s.open.as_ref()).is_none_or(|e| e.is_true(Ctx { props: &props, facts: &facts }));
        draw.views.clear();
        draw.views.extend(sheets.iter().filter(|l| open(l.view.surface)).map(|l| l.view.bounds()));
        for (k, em) in scene.popups.iter().enumerate() {
            let c = Ctx { props: &props, facts: &facts };
            let wants = (facts[em.open.0 as usize] > 0.5 && !sheets.is_empty()).then(|| {
                [em.at.0.eval(c).round() as i32, em.at.1.eval(c).round() as i32, em.size.0.eval(c).round().max(1.0) as i32, em.size.1.eval(c).round().max(1.0) as i32]
            });
            if wants != popups[k] {
                if popups[k].is_some() {
                    sheets.retain(|l| l.view.popup != Some(k));
                    crate::platform::popup(k, None);
                    if let Some(g) = &gpu {
                        assign_pace(g, &mut sheets, size, op.no_vsync);
                    }
                }
                if let Some(g) = wants {
                    crate::platform::popup(k, Some((g, em.origin)));
                }
                popups[k] = wants;
            }
            if let Some(g) = popups[k] {
                draw.views.push([em.origin.0, em.origin.1, em.origin.0 + g[2] as f32, em.origin.1 + g[3] as f32]);
            }
        }

        // The language changed (`locale`): the declared texts that still say one of their
        // versions say the new one. One the logic has written something else into is its own.
        if let Some(l) = scene.locale {
            let now = facts[l.0 as usize].round().max(0.0) as usize;
            if shown_locale != Some(now) {
                for (t, versions) in &scene.text_versions {
                    let k = t.0 as usize;
                    let Some(v) = versions.get(now) else { continue };
                    if texts.get(k).is_some_and(|x| versions.contains(x)) && texts[k] != *v {
                        texts[k] = v.clone();
                        let _ = to_logic.send(Event::Text(scene.texts[k].0, v.clone()));
                    }
                }
                shown_locale = Some(now);
            }
        }

        // ── 3. paint ────────────────────────────────────────────
        let blocked = logic_blocked.load(Ordering::Relaxed);
        let c = Ctx { props: &props, facts: &facts };
        // Text and images are painted at the scale of the finest sheet; the
        // others see them reduced, which shows much less than enlarged.
        let finest = sheets.iter().map(|l| l.scale).fold(1.0f32, f32::max);
        if atlas_stale || (finest - letters.scale()).abs() > 0.001 {
            letters.reset(finest, &scene.images);
            atlas_stale = false;
        }
        // It is composed even if there is nowhere to paint yet: that way the workshop gets on with
        // the texts and the images while the GPU and the windows start up.
        // The text cursor blinks: half a second on, half off, and always on right
        // after typing. Between blinks there is no need to paint.
        let view = editing.as_ref().map(|e| {
            let t = (now - last_key).as_secs_f32();
            appointments.push(now + Duration::from_secs_f32(0.53 - t % 0.53 + 0.001));
            crate::gpu::FieldView { text: e.field, cursor: e.cursor, anchor: e.anchor, visible: (t % 1.06) < 0.53 }
        });
        if let Some((_, _, _, when)) = &repeat {
            appointments.push(*when);
        }
        let to_paint: &[Instr] = if warning.is_some() { &with_warning } else { &scene.instrs };
        draw.set_attached_edges(scene.surface().anchor.attached_edges());
        let reading = Instant::now();
        let skip: Vec<std::ops::Range<usize>> = asleep.iter().map(|t| t.instrs.clone()).collect();
        draw.skip = skip;
        draw.clock = t_total;
        draw.reduced_motion = op.reduced_motion;
        if draw.signal_times.len() != scene.signals.len() {
            draw.signal_times = vec![-1.0; scene.signals.len()];
        }
        draw.compose(to_paint, c, &texts, &mut letters, view, size, op.hud);
        // Particles carry themselves: while one is alive, the scene does not rest.
        alive |= draw.particles_alive;
        // An image that moves: wake up when it changes frame.
        if let Some(t) = draw.wake_at {
            appointments.push(start + Duration::from_secs_f32(t.max(t_total + 0.001)));
        }
        cycle.compose_ms += reading.elapsed().as_secs_f32() * 1000.0;
        let Some(g) = &mut gpu else {
            // Nowhere yet: time runs all the same, but unhurried.
            std::thread::sleep(Duration::from_millis(8));
            continue;
        };
        // What was uploaded to the atlas may take the slot of something that is no longer there: with
        // a new atlas, any letter may have changed without changing its `uv`.
        if !letters.pending_upload.is_empty() {
            previous.forget();
        }
        g.upload_atlas(&mut letters.pending_upload);
        g.upload(&draw);
        // What has changed, and where. The frame graph always changes.
        // The light of a click changes the glass without changing the list: everything is painted.
        let all_changed = !previous.changed_rects(&draw, &mut changed) || op.hud || finger_light > 0.0 || ripple.is_some();

        // Where the mouse comes in: the active zones, and nothing else. The rest of
        // the surface is transparent for the click too.
        // Nothing can be clicked on a closed surface. Its piece of the plane is
        // the one its sheet really covers, not the size written in the scene:
        // `size: full, full` is written as 0 × 0, and a closed full-screen
        // surface kept its zones —a transparent wall over the whole desktop
        // that took every click, and with no cursor of its own, the mouse
        // vanished—.
        let closed: Vec<[f32; 4]> = sheets
            .iter()
            .filter(|l| l.view.popup.is_none())
            .filter(|l| scene.surfaces.get(l.view.surface).is_some_and(|s| s.open.as_ref().is_some_and(|e| !e.is_true(c))))
            .map(|l| l.view.bounds())
            .chain(scene.surfaces.iter().filter(|s| s.open.as_ref().is_some_and(|e| !e.is_true(c))).map(|s| [s.origin.0, s.origin.1, s.origin.0 + s.width.max(1) as f32, s.origin.1 + s.height.max(1) as f32]))
            .collect();
        let boxes: Vec<[i32; 4]> = scene
            .zones
            .iter()
            .filter(|z| z.active.is_true(c))
            .filter_map(|z| z.bounds(c))
            .filter(|b| !closed.iter().any(|v| b[0] < v[2] && b[2] > v[0] && b[1] < v[3] && b[3] > v[1]))
            .map(|b| [(b[0] - 3.0).floor() as i32, (b[1] - 3.0).floor() as i32, (b[2] + 3.0).ceil() as i32, (b[3] + 3.0).ceil() as i32])
            .collect();
        let region_changes = boxes != region;
        if region_changes {
            // Each surface gets the zones that fall on ITS piece of the plane, in its
            // coordinates. A popup is all its own, and carries no region.
            for l in sheets.iter_mut().filter(|l| l.view.popup.is_none()) {
                let v = l.view.bounds();
                let (dx, dy) = (l.view.origin.0 as i32, l.view.origin.1 as i32);
                let its_own: Vec<[i32; 4]> = boxes
                    .iter()
                    .filter(|b| (b[0] as f32) < v[2] && (b[2] as f32) > v[0] && (b[1] as f32) < v[3] && (b[3] as f32) > v[1])
                    .map(|b| [b[0] - dx, b[1] - dy, b[2] - dx, b[3] - dy])
                    .collect();
                // `PLEAMAR_REGIONS=1`: where each surface takes the mouse, when it
                // changes. What answers «why does this not click» —or «why does
                // everything click here»—.
                // Only if it changed for THIS surface, and then with a frame: the
                // region is committed with the next frame presented, and a
                // surface that draws nothing —a full-screen catcher— may never
                // present another, leaving its new region pending for ever.
                if l.input_region != its_own {
                    if std::env::var_os("PLEAMAR_REGIONS").is_some() {
                        eprintln!("regions · surface {} ({}): {:?}", l.view.surface, scene.surfaces.get(l.view.surface).map_or("", |s| s.name.as_str()), its_own);
                    }
                    l.update_input_region(&its_own);
                    l.input_region = its_own;
                    l.painted = None;
                }
            }
            region = boxes;
        }
        // And behind the glass, what the compositor blurs: the strips that
        // fall on each surface's piece, in its coordinates. Only when they change.
        for l in sheets.iter_mut() {
            let v = l.view.bounds();
            let local = |b: &[f32; 4]| [(b[0].max(v[0]) - v[0]) as i32, (b[1].max(v[1]) - v[1]) as i32, (b[2].min(v[2]) - v[0]).ceil() as i32, (b[3].min(v[3]) - v[1]).ceil() as i32];
            // The ones of the glass that bends the light (`lens`), and those of the one that does not.
            // `PLEAMAR_NO_LENS=1`: none bends it, and the compositor blurs it.
            let (mut with_lens, mut without) = (Vec::new(), Vec::new());
            for (b, lens) in draw.glass_regions.iter().filter(|(b, _)| b[0] < v[2] && b[2] > v[0] && b[1] < v[3] && b[3] > v[1]) {
                if *lens && !no_lens { with_lens.push(local(b)) } else { without.push(local(b)) }
            }
            l.wants_lens = !with_lens.is_empty();
            // Only what is needed is captured: the glass's box, with a
            // margin for frosting, rounded to 16 px so as not to remake
            // textures over one pixel. In Marea, the little ball and not the 820 × 680.
            l.glass_box = with_lens.iter().copied().reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]).map(|b| {
                let m = crate::lens::MARGIN.ceil() as i32;
                let (w, h) = (l.view.size.0 as i32, l.view.size.1 as i32);
                let x0 = ((b[0] - m).max(0) / 16) * 16;
                let y0 = ((b[1] - m).max(0) / 16) * 16;
                let x1 = ((b[2] + m + 15) / 16 * 16).min(w);
                let y1 = ((b[3] + m + 15) / 16 * 16).min(h);
                [x0, y0, x1 - x0, y1 - y0]
            });
            // What the compositor blurs: the glass without a lens, and the one that has
            // it while there is still no background to show.
            if !l.lens.as_ref().is_some_and(|x| x.ready) {
                without.extend(with_lens);
            }
            if without != l.blur_rects {
                l.update_blur_region(&without);
                l.blur_rects = without;
            }
        }

        // The 4 and the 7 are the view's origin: zero on the main one; each popup sets its own.
        uniforms[..8].copy_from_slice(&[size.0, size.1, t_total, 1.0, 0.0, period_ms, if blocked { 1.0 } else { 0.0 }, 0.0]);
        uniforms[8..128].copy_from_slice(&history);
        // While pressed, the light follows the finger; on release, it goes out in ~0.4 s.
        if finger_down {
            if let Some(p) = pointer {
                finger = p;
            }
        } else {
            finger_light *= (-dt / 0.4).exp();
        }
        let light_alive = finger_light > 0.01;
        if !light_alive {
            finger_light = 0.0;
        }
        uniforms[129..132].copy_from_slice(&[finger.0, finger.1, finger_light]);
        // It lasts 0.9 s, and while it lasts there are frames.
        if ripple.is_some_and(|(_, t)| t.elapsed().as_secs_f32() > 0.9) {
            ripple = None;
        }
        uniforms[132..136].copy_from_slice(&match ripple {
            Some(((x, y), t)) => [x, y, t.elapsed().as_secs_f32(), 1.0],
            None => [0.0; 4],
        });
        // The one that sets the pace goes last: it is the one that waits for the screen.
        sheets.sort_by_key(|l| l.drives_pace);
        // Has any surface decided to attach itself to another edge? The corner that
        // Marea's recording face chooses is a fact, and layer-shell lets it be
        // changed without creating anything again.
        if anchors_set.len() != scene.surfaces.len() {
            anchors_set = scene.surfaces.iter().map(|s| s.anchor).collect();
        }
        // And a level of its own while something holds: above the rest while
        // it has something open, where it belongs the rest of the time.
        if levels_set.len() != scene.surfaces.len() {
            levels_set = scene.surfaces.iter().map(|s| s.level).collect();
        }
        for (k, sup) in scene.surfaces.iter().enumerate() {
            let Some((raised, when)) = &sup.level_while else { continue };
            let wants = if when.is_true(Ctx { props: &props, facts: &facts }) { *raised } else { sup.level };
            if levels_set[k] != wants {
                levels_set[k] = wants;
                crate::platform::relayer(k, wants);
            }
        }
        for (k, sup) in scene.surfaces.iter().enumerate() {
            let Some((fact, anchors)) = &sup.anchor_from else { continue };
            let wants = anchors.get(facts[fact.0 as usize].round().max(0.0) as usize).copied();
            if let Some(a) = wants.filter(|a| anchors_set[k] != *a) {
                anchors_set[k] = a;
                crate::platform::reanchor(k, a);
            }
        }

        // Which ones are open now. If that changes, the pace is handed out again: a
        // closed surface that set it would wait with vsync for a frame that the
        // compositor is not going to give it —what is not seen is not given any—, and with it
        // everything stopped: 300 ms in the middle of an animation of another window.
        let mut open_changed = false;
        for l in &mut sheets {
            let is_open = l.view.popup.is_some() || open(l.view.surface);
            open_changed |= is_open != l.open;
            if is_open {
                l.cleared = false;
            } else if l.lens.is_some() {
                // Closed it shows no glass: its canvas and its background, out.
                l.cancel_backdrop();
                l.lens = None;
            }
            l.open = is_open;
        }
        if open_changed {
            assign_pace(g, &mut sheets, size, op.no_vsync);
            sheets.sort_by_key(|l| l.drives_pace);
        }
        // The step. With mailbox the render sets it: an absolute deadline per period of the
        // monitor that sets the pace, so that the error of each wait does not
        // accumulate; if it arrives late —a rest, a long frame— it starts again
        // from now instead of rushing to catch up. With queued vsync the screen
        // sets it, but only when its queue is full: on waking up it
        // accepted two or three frames without waiting and the loop presented them in two
        // milliseconds; there, never more than one frame per period.
        if !op.no_vsync && !op.naive {
            let right_now = Instant::now();
            if g.uses_mailbox() {
                let mhz = sheets.iter().find(|l| l.drives_pace).map_or(60_000, |l| l.mhz.max(1));
                // With `rate:`, the scene decides the step and not the monitor: a bar
                // that breathes does not need 165 frames per second, and painting them is what
                // costs. Never faster than the screen, which would be no use.
                let mhz = match scene.surface().max_fps {
                    0 => mhz,
                    r => mhz.min(r as i32 * 1000),
                };
                let period = Duration::from_secs_f64(1000.0 / mhz as f64);
                // The clock gives the step: one deadline per period. The compositor's
                // notice is no use as a metronome —Hyprland notifies when it composes,
                // and if a frame reaches it right after, it composes again and notifies
                // straight away: two frames per refresh—, but it does say something else: whether
                // anyone is watching.
                if next_frame > right_now {
                    // While waiting, the capture that a sheet with a lens
                    // needed in order to paint may arrive: then it is painted right away, without waiting
                    // for the clock, or its step and the compositor's drift out of phase and
                    // one out of every two gets painted.
                    let awaits_capture = sheets.iter().any(|l| l.capture == crate::gpu::BackdropCapture::AfterPresent);
                    if awaits_capture {
                        loop {
                            let left = next_frame.saturating_duration_since(Instant::now());
                            if left.is_zero() {
                                next_frame += period;
                                break;
                            }
                            match rx.recv_timeout(left) {
                                Ok(ToRender::Backdrop(d)) => {
                                    if handle_backdrop(g, &mut sheets, d) {
                                        next_frame = Instant::now() + period;
                                        break;
                                    }
                                }
                                // The frame notice is waited for right after: kept for the
                                // next round, that wait would not see it and would run out.
                                Ok(ToRender::Frame(k)) => frame_ready |= frame_requested == Some(k),
                                Ok(m) => held_back.push(m),
                                Err(RecvTimeoutError::Timeout) => {}
                                Err(RecvTimeoutError::Disconnected) => return,
                            }
                        }
                    } else {
                        std::thread::sleep(next_frame - right_now);
                        next_frame += period;
                    }
                } else {
                    next_frame = right_now + period;
                }
                // What it is not showing —a monitor that is off, a window on another
                // workspace— it does not notify. After three waits in vain, it paints once
                // a second instead of 60 or 165 times for nobody.
                if let Some(id) = frame_requested {
                    let patience = if missed_notices >= 3 { Duration::from_secs(1) } else { (period * 3).max(Duration::from_millis(50)) };
                    let limit = Instant::now() + patience;
                    while !frame_ready {
                        let left = limit.saturating_duration_since(Instant::now());
                        if left.is_zero() {
                            missed_notices += 1;
                            if missed_notices == 3 {
                                println!("render · the compositor is not showing this surface: painting once a second until it shows it again");
                            }
                            break;
                        }
                        match rx.recv_timeout(left) {
                            Ok(ToRender::Frame(k)) => frame_ready |= k == id,
                            Ok(m) => held_back.push(m),
                            Err(RecvTimeoutError::Timeout) => {}
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    if frame_ready {
                        if missed_notices >= 3 {
                            println!("render · the compositor is showing it again");
                        }
                        missed_notices = 0;
                    }
                    if Instant::now() > next_frame {
                        next_frame = Instant::now() + period;
                    }
                }
            } else {
                let minimum = Duration::from_secs_f32(period_ms * 0.8 / 1000.0);
                let since = right_now - last_presented;
                if since < minimum {
                    std::thread::sleep(minimum - since);
                }
            }
        }
        let before_painting = last_presented;
        last_presented = Instant::now();
        let mut painted = 0;
        let mut up_to_date = 0;
        let mut waiting_for_screen = false;
        // If the one that sets the pace does not paint this time, there is no notice to wait for:
        // the clock sets the next round.
        frame_requested = None;
        frame_ready = false;
        for l in &mut sheets {
            // Closed, it is painted once, empty, and that is it: that, yes, always, whether something changes or not.
            if !l.open && std::mem::replace(&mut l.cleared, true) {
                continue;
            }
            // What it shows is already up to date: nothing of what has changed falls on its piece of the plane.
            let where_ = (l.view.bounds(), l.scale);
            let v = where_.0;
            // A lens still warming up (see `lens::WARM_UP`) whose capture has been
            // lost: the compositor answers a capture the next time it paints the
            // monitor, and with nothing moving there it does not paint. Presenting
            // again is what makes it paint. Without this, a glass or a shader
            // that stays still from its first frame never saw its background.
            let unseen = l.lens.as_ref().is_some_and(|x| x.warming()) && l.capture == crate::gpu::BackdropCapture::AfterPresent;
            if unseen && l.capture_asked.elapsed() < Duration::from_millis(100) {
                appointments.push(l.capture_asked + Duration::from_millis(101));
            }
            let nudge = unseen && l.capture_asked.elapsed() >= Duration::from_millis(100);
            let its_turn = !l.open || all_changed || nudge || l.painted != Some(where_) || changed.iter().any(|b| b[0] < v[2] && b[2] > v[0] && b[1] < v[3] && b[3] > v[1]);
            if !its_turn {
                up_to_date += 1;
                continue;
            }
            // With a lens: if the capture of the last presented frame has not arrived yet, it
            // waits for it —it arrives in less than a refresh— and paints afterwards.
            // If it was only watching, that capture is forgotten and it paints.
            if l.lens.is_some() {
                match l.capture {
                    // It arrives in less than a refresh. If it takes longer, it has been lost.
                    crate::gpu::BackdropCapture::AfterPresent if l.capture_asked.elapsed() < Duration::from_millis(100) => {
                        l.painted = None;
                        up_to_date += 1;
                        continue;
                    }
                    crate::gpu::BackdropCapture::AfterPresent => l.cancel_backdrop(),
                    crate::gpu::BackdropCapture::Watching => l.cancel_backdrop(),
                    crate::gpu::BackdropCapture::Idle => {}
                }
            }
            // The one that sets the pace asks the compositor to notify when it wants another.
            let asks = l.drives_pace && g.uses_mailbox() && !op.no_vsync && !op.naive;
            let was_painted = g.paint(l, &draw, &uniforms, asks);
            // With a lens, what was presented is captured to keep track of what is behind.
            // Not on every frame: for Hyprland each capture costs reading the screen
            // back from the card, and at 60 per second that was 14 points of a core.
            // One every `CAPTURE_INTERVAL` at most while painting; and when it
            // stops painting, it watches (see further down).
            let warming = l.lens.as_ref().is_some_and(|x| x.warming());
            if was_painted && l.lens.is_some() && (warming || l.capture_taken.elapsed() >= crate::lens::CAPTURE_INTERVAL) {
                l.request_backdrop(false);
            }
            l.painted_now = was_painted;
            if asks {
                frame_requested = was_painted.then_some(l.id);
                frame_ready = false;
            }
            // Cleared, it does not show the scene's things: on reopening, it is painted no matter what.
            l.painted = (was_painted && l.open).then_some(where_);
            waiting_for_screen |= was_painted && l.drives_pace && !g.uses_mailbox();
            painted += was_painted as u32;
        }
        // A sheet with a lens that has not painted this round has stayed
        // still: what is behind is watched. Watching while painting is no use,
        // because our own frame already counts as a change and the capture would arrive
        // on every refresh; now it arrives only if something changes behind, and it brings the
        // last thing that was presented.
        for l in &mut sheets {
            if l.lens.is_some() && !std::mem::take(&mut l.painted_now) && l.capture == crate::gpu::BackdropCapture::Idle {
                l.request_backdrop(true);
            }
        }
        // With queued vsync, the step was given by the wait of the one that sets the pace.
        // If it was not its turn to paint this time, the clock gives the step: a whole period.
        if painted + up_to_date > 0 && !waiting_for_screen && !g.uses_mailbox() && !op.no_vsync && !op.naive {
            let period = Duration::from_secs_f32(period_ms / 1000.0);
            let since = before_painting.elapsed();
            if since < period {
                std::thread::sleep(period - since);
            }
            last_presented = Instant::now();
        }
        if crate::gpu::timing_enabled() && painted + up_to_date > 0 {
            sheet_counts.0 += painted;
            sheet_counts.1 += up_to_date;
            sheet_counts.2 += 1;
            if sheet_counts.2 >= 300 {
                println!("timing · surfaces per frame: {:.2} painted, {:.2} up to date", sheet_counts.0 as f32 / 300.0, sheet_counts.1 as f32 / 300.0);
                sheet_counts = (0, 0, 0);
            }
        }
        if painted + up_to_date == 0 {
            std::thread::sleep(Duration::from_millis(8));
        } else if painted > 0 && first_frame {
            first_frame = false;
            println!("render · first frame {} ms after starting", op.start_time.elapsed().as_millis());
        }

        // What the texts measured goes into their properties: the coming frame,
        // the box that depends on them already knows it.
        let mut measure_changed = false;
        for (p, v) in &draw.measurements {
            let a = &mut props[p.0 as usize];
            if (a.x - v).abs() > 0.01 {
                a.set(*v);
                measure_changed = true;
            }
        }

        // ── 4. measure ──────────────────────────────────────────
        //  What was asked to be recorded, frame by frame: it is how you check that an
        //  animation lasts as long as its contract says it lasts.
        if !op.trace.is_empty() {
            if !trace_header_done {
                trace_header_done = true;
                // A name that does not exist was recorded as "?" for the whole
                // measurement, and there was no way of knowing whether it was misspelled or
                // the scene just did not move it. Now it is said, once, with the
                // ones that resemble it: inside a copy the names carry their
                // mark (`px#Hat1`), and nobody could guess that.
                for n in &op.trace {
                    let n = n.as_str();
                    let exists = scene.props.iter().any(|p| p.0 == n) || scene.facts.iter().any(|h| h.0 == n) || scene.texts.iter().any(|t| t.0 == n);
                    if !exists {
                        let mut close: Vec<&str> = scene
                            .props
                            .iter()
                            .map(|p| p.0)
                            .chain(scene.facts.iter().map(|h| h.0))
                            .chain(scene.texts.iter().map(|t| t.0))
                            .filter(|c| c.contains(n) || n.contains(*c) || c.split('#').next() == Some(n))
                            .collect();
                        close.sort_unstable();
                        close.dedup();
                        close.truncate(6);
                        let hint = if close.is_empty() { String::new() } else { format!(" · there is {}", close.join(", ")) };
                        println!("record · there is nothing called '{n}'{hint}");
                    }
                }
                println!("ms\t{}", op.trace.join("\t"));
            }
            let values: Vec<String> = op
                .trace
                .iter()
                .map(|n| {
                    let n = n.as_str();
                    if let Some(i) = scene.props.iter().position(|p| p.0 == n) {
                        format!("{:.4}", props[i].x)
                    } else if let Some(i) = scene.facts.iter().position(|h| h.0 == n) {
                        format!("{:.4}", facts[i])
                    } else if let Some(i) = scene.texts.iter().position(|t| t.0 == n) {
                        texts.get(i).cloned().unwrap_or_default()
                    } else {
                        "?".into()
                    }
                })
                .collect();
            println!("{:.1}\t{}", t_total * 1000.0, values.join("\t"));
        }
        let ms = dt * 1000.0;
        // A permanent telltale: any frame that goes over two periods, with its time.
        if ms > period_ms * 2.4 && !first_frame && cycle.dts.len() > 1 && !op.no_vsync && !op.naive {
            println!("render · slow frame: {ms:.0} ms at {t_total:.2} s");
        }
        history.copy_within(1.., 0);
        history[119] = if blocked { -ms } else { ms };
        cycle.dts.push(ms);
        if !op.no_vsync && !op.naive && !blocked {
            // The LEARNT period is no good here: a scene that is always
            // late teaches it that the screen gives 28 ms and then it is never
            // late. It is compared with the monitor's real refresh.
            let mut real = sheets.iter().find(|l| l.drives_pace).map_or(16.7, |l| 1_000_000.0 / l.mhz.max(1) as f32);
            if scene.surface().max_fps > 0 {
                real = real.max(1000.0 / scene.surface().max_fps as f32);
            }
            cycle.watch(real);
        }
        if blocked {
            cycle.frames_blocked += 1;
            cycle.max_blocked = cycle.max_blocked.max(ms);
        }
        // The screen's period, learnt from the first good frames.
        if frames_for_period < 40 && ms > 3.0 && ms < 40.0 {
            frames_for_period += 1;
            if frames_for_period == 40 {
                let mut o: Vec<f32> = cycle.dts.iter().copied().filter(|m| *m > 3.0 && *m < 40.0).collect();
                o.sort_by(|a, b| a.total_cmp(b));
                if !o.is_empty() {
                    period_ms = o[o.len() / 2];
                }
            }
        }

        // ── 5. is anything still moving? ────────────────────────
        if op.no_vsync && cycle.dts.len() >= 600 {
            cycle.close();
        }
        if !op.no_vsync && !alive && !something_happened && finger_light == 0.0 && ripple.is_none() && !blocked && !region_changes && !measure_changed && !keyboard_changed && props.iter().all(Animated::at_rest) {
            for a in &mut props {
                a.settle();
            }
            cycle.close();
            next_appointment = appointments.iter().min().copied();
            resting = true;
            // Still now: if something overflowed, this is what stays watching.
            draw.report_pending();
        }
    }
}

/// A capture of what is behind a sheet arrives: the background is unmixed with the
/// canvas that was presented. If something has changed, it has to be painted again (and
/// after painting it another will be asked for); if not, it watches until something changes behind.
/// Returns whether it has to paint.
fn handle_backdrop(g: &Gpu, sheets: &mut [Sheet], d: Box<crate::platform::Backdrop>) -> bool {
    let Some(l) = sheets.iter_mut().find(|l| l.id == d.sheet) else { return false };
    l.capture = crate::gpu::BackdropCapture::Idle;
    if l.lens.is_none() {
        return false;
    }
    if g.receive_backdrop(l, *d) {
        l.painted = None;
        true
    } else {
        l.request_backdrop(true);
        false
    }
}

/// The banner that says that what you have just saved does not compile, with the file, the
/// line and the message. It goes at the top of the main surface, which is where you are
/// looking, and goes away by itself when the scene is fine again. Underneath, the last good
/// scene keeps running: this stops nothing, it only tells.
fn error_banner(message: &str, width: f32) -> Vec<Instr> {
    let width = if width > 0.0 { width } else { 900.0 };
    // The first line is the one that says what is going on; the rest is the finger pointing.
    let said = message.lines().next().unwrap_or(message).trim().to_string();
    let height = 52.0;
    let rect = |x: f32, w: f32, r: f32| Shape::Rect {
        center: ((x + w * 0.5).into(), (height * 0.5).into()),
        half_size: ((w * 0.5).into(), (height * 0.5).into()),
        radius: r.into(),
    };
    vec![
        Instr::Group { shadow: Some(Shadow { offset: (0.0.into(), 6.0.into()), blur: 18.0.into(), alpha: 0.45.into(), color: None }) },
        Instr::Shape { shape: rect(8.0, width - 16.0, 10.0), fusion: 0.0.into() },
        Instr::Fill { paint: color(0.18, 0.05, 0.06).into(), alpha: 1.0.into(), rim: 0.05, light: None, border: None, glass_spec: None },
        // A tab in the colour of errors, on the left: it is read before the text.
        Instr::Group { shadow: None },
        Instr::Shape { shape: rect(8.0, 5.0, 2.5), fusion: 0.0.into() },
        Instr::Fill { paint: color(0.99, 0.41, 0.33).into(), alpha: 1.0.into(), rim: 0.05, light: None, border: None, glass_spec: None },
        Instr::Text {
            content: Content::Literal("this does not compile — the last good scene is still running".into()),
            at: (26.0.into(), 17.0.into()),
            anchor: (0.0, 0.5),
            width: Some((width - 52.0).into()),
            style: Style::new(10.5, color(0.99, 0.41, 0.33)).weight(600).lines(1),
            alpha: 1.0.into(),
            measure: None,
        },
        Instr::Text {
            content: Content::Literal(said),
            at: (26.0.into(), 34.0.into()),
            anchor: (0.0, 0.5),
            width: Some((width - 52.0).into()),
            style: Style::new(12.5, color(0.96, 0.96, 0.96)).lines(1),
            alpha: 1.0.into(),
            measure: None,
        },
    ]
}

/// A single sheet waits for its screen —the one of the fastest monitor— and the others
/// present without blocking. If they all waited, a monitor at 60 Hz would hold back
/// another at 165.
fn assign_pace(g: &Gpu, sheets: &mut [Sheet], size: (f32, f32), no_vsync: bool) {
    // A popup never sets the pace: it comes and goes, and it may be covered.
    // Nor a closed one. And between equals, the first: `max_by_key` keeps the
    // last, which with two surfaces at 60 Hz was the corner one, closed.
    let mut fastest: Option<(i32, _)> = None;
    for l in sheets.iter().filter(|l| l.view.popup.is_none() && l.open) {
        if fastest.is_none_or(|(mhz, _)| l.mhz > mhz) {
            fastest = Some((l.mhz, l.id));
        }
    }
    let fastest = fastest.map(|(_, id)| id);
    for l in sheets.iter_mut() {
        let drives = !no_vsync && Some(l.id) == fastest;
        if l.drives_pace != drives {
            l.drives_pace = drives;
            g.reconfigure(l, size);
        }
    }
}

/// The field being typed into: which text, where the cursor is and from where it was
/// selected. In bytes, always on the boundary of a letter.
struct Editing {
    field: usize,
    cursor: usize,
    anchor: usize,
}

enum KeyOutcome {
    Changed,
    Moved,
    Submitted,
    Unhandled,
}

impl Editing {
    fn selection(&self) -> (usize, usize) {
        (self.cursor.min(self.anchor), self.cursor.max(self.anchor))
    }

    fn delete_selection(&mut self, t: &mut String) -> bool {
        let (a, b) = self.selection();
        t.replace_range(a..b, "");
        self.cursor = a;
        self.anchor = a;
        b > a
    }

    fn handle_key(&mut self, t: &mut String, name: &str, typed: Option<&str>, m: Mods) -> KeyOutcome {
        self.cursor = self.cursor.min(t.len());
        self.anchor = self.anchor.min(t.len());
        let before = |t: &str, b: usize| t[..b].char_indices().next_back().map_or(0, |c| c.0);
        let after = |t: &str, b: usize| t[b..].chars().next().map_or(t.len(), |c| b + c.len_utf8());
        let move_to = |me: &mut Self, a: usize| {
            me.cursor = a;
            if !m.shift {
                me.anchor = a;
            }
            KeyOutcome::Moved
        };
        match name {
            "a" if m.ctrl => {
                self.anchor = 0;
                self.cursor = t.len();
                KeyOutcome::Moved
            }
            "c" | "x" if m.ctrl => {
                let (a, b) = self.selection();
                if b > a {
                    crate::platform::clipboard_write(&t[a..b]);
                }
                if name == "x" && self.delete_selection(t) { KeyOutcome::Changed } else { KeyOutcome::Moved }
            }
            "v" if m.ctrl => {
                let Some(pasted) = crate::platform::clipboard_read() else { return KeyOutcome::Moved };
                // A field is single-line: whatever comes with line breaks, without them.
                let pasted: String = pasted.chars().filter(|c| !c.is_control()).collect();
                self.delete_selection(t);
                t.insert_str(self.cursor, &pasted);
                self.cursor += pasted.len();
                self.anchor = self.cursor;
                KeyOutcome::Changed
            }
            "BackSpace" | "Delete" => {
                if !self.delete_selection(t) {
                    let (a, b) = if name == "BackSpace" { (before(t, self.cursor), self.cursor) } else { (self.cursor, after(t, self.cursor)) };
                    t.replace_range(a..b, "");
                    self.cursor = a;
                    self.anchor = a;
                }
                KeyOutcome::Changed
            }
            "Left" => {
                let a = before(t, self.cursor);
                move_to(self, a)
            }
            "Right" => {
                let a = after(t, self.cursor);
                move_to(self, a)
            }
            "Home" => move_to(self, 0),
            "End" => {
                let a = t.len();
                move_to(self, a)
            }
            "Return" | "KP_Enter" => KeyOutcome::Submitted,
            _ => match typed {
                Some(letters) if !m.ctrl && !m.alt && !m.logo => {
                    self.delete_selection(t);
                    t.insert_str(self.cursor, letters);
                    self.cursor += letters.len();
                    self.anchor = self.cursor;
                    KeyOutcome::Changed
                }
                _ => KeyOutcome::Unhandled,
            },
        }
    }
}

struct LayerState {
    hot: bool,
    winner: Option<usize>,
    until: Vec<Option<Instant>>,
    lit: Vec<bool>,
}

impl LayerState {
    fn new(n: usize, hot: bool) -> Self {
        LayerState { hot, winner: None, until: vec![None; n], lit: vec![false; n] }
    }
}

#[derive(Default)]
struct RuleState {
    /// What the expression of an `on change` was worth the last time.
    last_value: Option<f32>,
    since: Option<Instant>,
    fired: bool,
    armed: bool,
    next: Option<Instant>,
    last_time: Option<Instant>,
}

impl RuleState {
    /// "It has been true for this long": fires once per episode.
    fn sustained(&mut self, true_now: bool, duration: Duration, now: Instant, appointments: &mut Vec<Instant>) -> bool {
        if !true_now {
            self.since = None;
            self.fired = false;
            return false;
        }
        let since = *self.since.get_or_insert(now);
        if self.fired {
            return false;
        }
        if now - since >= duration {
            self.fired = true;
            return true;
        }
        appointments.push(since + duration);
        false
    }
}

struct Playback {
    gesture: usize,
    keyframe: usize,
    started_at: Instant,
    from: Vec<f32>,
    /// With reduced motion: the keyframe that is shown, still, the whole time.
    still: Option<usize>,
    values: Vec<Vec<(PropId, f32)>>,
}

impl Playback {
    fn start(k: usize, g: &Gesture, pose: &[PropId], c: Ctx, now: Instant, reduced_motion: bool) -> Self {
        let props = c.props;
        // The values of each keyframe are fixed at the start.
        let values: Vec<Vec<(PropId, f32)>> = g.keyframes.iter().map(|f| f.values.iter().map(|(p, e)| (*p, e.eval(c))).collect()).collect();
        // The still face of a gesture is the keyframe that strays furthest from the base.
        let still = reduced_motion.then(|| {
            let score = |f: &Vec<(PropId, f32)>| -> f32 { f.iter().map(|(p, v)| (v - props[p.0 as usize].target).abs() / props[p.0 as usize].target.abs().max(1.0)).sum() };
            (0..values.len()).max_by(|a, b| score(&values[*a]).total_cmp(&score(&values[*b]))).unwrap_or(0)
        });
        Playback { gesture: k, keyframe: 0, started_at: now, from: pose.iter().map(|p| props[p.0 as usize].x).collect(), still, values }
    }

    /// Where a keyframe takes a property: to what it says, or to its base.
    fn target(&self, keyframe: usize, p: PropId, props: &[Animated]) -> f32 {
        self.values[keyframe].iter().find(|(q, _)| *q == p).map_or(props[p.0 as usize].target, |(_, v)| *v)
    }
}
