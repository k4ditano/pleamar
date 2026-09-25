//! pleamar — prototype of a shell where animating does not depend on the logic.
//!
//! Three threads: this one, which the platform keeps (the windows of each
//! monitor, their scale and the mouse); the logic one, which decides; and the render one, which is the
//! only one that animates.

mod scene;
mod scenes;
mod shaders;
mod shapes;
mod lsp;
mod gpu;
mod lens;
mod language;
mod logic;
#[cfg(feature = "luau")]
mod logic_luau;
mod permissions;
mod platform;
mod render;
mod text;

use scene::*;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const HELP: &str = "pleamar [options]
  --scene FILE        the scene to open (.plm); it reloads itself when you save it. The test
                      benches written in Rust also work: marea, island, face, showcase, swarm
  --check FILE        reads a scene, says whether it is fine, and exits
  --approve SCENE     shows what the plugins of a scene ask for, and asks whether to approve them
                      (with --yes after it, it does not ask). Unapproved, a plugin runs touching nothing
  --grammar           the words the language accepts, exactly as the compiler consults them
  --lsp               a language server on stdio: mistakes as you type, what fits here, and what
                      each word means. For any editor that speaks LSP
  --highlight EDITOR  writes the syntax file for 'vim' or 'vscode', made from the vocabulary
  --version           the version of the program and of the language it understands
  --say [SCENE] CMD   says something to a running scene and exits. Commands:
                      «emit event [n]», «fact name value», «text name whatever it says», «focus input», «get name» (answers), «quit»
  --screen NAMES      «all», or monitors separated by commas (by default, whatever the scene asks for).
                      A repeated name gives two surfaces on the same monitor.
  --stall MS          how long the logic blocks after every decision (600)
  --naive             the logic blocks the painting thread, as in QtQuick
  --demo              opens and closes by itself, with no mouse
  --mouse SCRIPT      fake mouse: «360,90@1000 click@2500 down@… up@… wheel+@… out@4000» (ms)
  --seconds N         exits by itself after N seconds
  --margin PX         top margin, instead of the scene\u{2019}s
  --no-hud            without the frame graph
  --record NAMES      prints what those properties, facts or texts are worth on every
                      frame, as «ms<tab>value…», to check an animation against its
                      contract: «--record lid,body.y --seconds 20 > log.tsv»
  --no-vsync          paint without waiting for the screen, to measure what a frame costs
  --reduced-motion    springs settle at once and gestures show their still face
Right-click closes it, unless the scene has something under the pointer that uses it.";

struct Args {
    scene: String,
    screen: Option<String>,
    stall: u64,
    naive: bool,
    demo: bool,
    mouse: Option<String>,
    seconds: Option<u64>,
    margin: Option<i32>,
    hud: bool,
    reduced: bool,
    no_vsync: bool,
    /// Which properties, facts or texts to record on every frame.
    record: Vec<String>,
}

fn args() -> Args {
    let mut a = Args { scene: String::new(), screen: None, stall: 600, naive: false, demo: false, mouse: None, seconds: None, margin: None, hud: true, reduced: false, no_vsync: false, record: Vec::new() };
    let mut it = std::env::args().skip(1);
    while let Some(op) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| { eprintln!("{HELP}"); std::process::exit(2) });
        match op.as_str() {
            "--scene" => a.scene = value(),
            "--say" => {
                let (a1, a2) = (value(), it.next());
                let r = match &a2 {
                    Some(command) => platform::send(Some(&a1), command),
                    None => platform::send(None, &a1),
                };
                std::process::exit(match r {
                    Ok(()) => 0,
                    Err(e) => {
                        eprintln!("{e}");
                        1
                    }
                });
            }
            "--version" => {
                println!("pleamar {} · language {}.{}", env!("CARGO_PKG_VERSION"), language::VERSION.0, language::VERSION.1);
                std::process::exit(0);
            }
            "--approve" => {
                let scene = value();
                let yes_to_all = std::env::args().any(|a| a == "--yes");
                std::process::exit(permissions::ask(&scene, yes_to_all));
            }
            "--grammar" => {
                print!("{}", language::vocabulary::to_text());
                std::process::exit(0);
            }
            // The editor: mistakes while you type, and the highlighting.
            "--lsp" => {
                lsp::serve();
                std::process::exit(0);
            }
            "--highlight" => std::process::exit(lsp::highlighting(&value())),
            "--check" => {
                let path = value();
                std::process::exit(match scenes::from_file::read(&path) {
                    Ok(e) => {
                        println!("{path}: ok · {} properties, {} instructions, {} layers, {} rules, {} zones, {} gestures", e.props.len(), e.instrs.len(), e.layers.len(), e.rules.len(), e.zones.len(), e.gestures.len());
                        0
                    }
                    Err(m) => {
                        eprintln!("{m}");
                        1
                    }
                });
            }
            "--screen" => a.screen = Some(value()),
            "--stall" => a.stall = value().parse().expect("--stall wants milliseconds"),
            "--mouse" => a.mouse = Some(value()),
            "--seconds" => a.seconds = Some(value().parse().expect("--seconds wants a number")),
            "--margin" => a.margin = Some(value().parse().expect("--margin wants pixels")),
            "--naive" => a.naive = true,
            "--demo" => a.demo = true,
            "--no-hud" => a.hud = false,
            "--reduced-motion" => a.reduced = true,
            "--no-vsync" => a.no_vsync = true,
            "--record" => a.record = value().split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect(),
            _ => { eprintln!("{HELP}"); std::process::exit(2) }
        }
    }
    if a.scene.is_empty() {
        eprintln!("{HELP}");
        std::process::exit(2);
    }
    a
}

fn main() {
    let start_time = std::time::Instant::now();
    let a = args();
    let blocked = Arc::new(AtomicBool::new(false));
    let (to_render, from_render) = channel();
    let (to_logic, from_logic) = channel();
    let mut script: Box<dyn logic::Script> = match a.scene.as_str() {
        "marea" => Box::<scenes::marea::Marea>::default(),
        "island" => Box::<scenes::island::Island>::default(),
        "face" => Box::<scenes::face::Face>::default(),
        "showcase" => Box::<scenes::showcase::Showcase>::default(),
        "swarm" => Box::<scenes::swarm::Swarm>::default(),
        path if std::path::Path::new(path).is_file() => scenes::from_file::script_for(path, to_render.clone(), to_logic.clone(), blocked.clone()),
        other => {
            eprintln!("I don't know the scene '{other}', and it is not a file\n{HELP}");
            std::process::exit(2)
        }
    };
    // The scene says which surface it wants; the command line can overrule
    // it.
    let mut scene = script.scene();
    if let Some(p) = &a.screen {
        let which: Vec<String> = p.split(',').map(str::to_owned).collect();
        let per_monitor = matches!(scene.surface().screens, Screens::Number(_));
        // A surface per monitor (`screens: each`) gets the list handed out: the
        // first copy to the first name, and so on. It is how two monitors are rehearsed.
        for s in &mut scene.surfaces {
            if let Screens::Number(k) = s.screens {
                s.screens = match which.get(k) {
                    _ if p == "all" => Screens::Number(k),
                    Some(name) => Screens::Named(vec![name.clone()]),
                    // More copies than monitors asked for: that one does not appear.
                    None => Screens::Named(Vec::new()),
                };
            }
        }
        // The main one, unless it is already one of the per-monitor copies: that one has already been handed out.
        if !per_monitor || p == "all" {
            scene.surface_mut().screens = if p == "all" { Screens::All } else { Screens::Named(which) };
        }
    }
    if let Some(m) = a.margin {
        scene.surface_mut().margin[0] = m;
    }

    // The text and image workshop. The first thing it does is read the system's
    // fonts, which is the slowest part of the startup: let it get going.
    let letters = text::Texts::open(to_render.clone());
    // Without wgpu's checks: in a bar that repaints 60 times a second, what is
    // saved per frame shows in the power draw. To debug, `PLEAMAR_VALIDATE=1`.
    let mut flags = wgpu::InstanceFlags::empty();
    if std::env::var_os("PLEAMAR_VALIDATE").is_some() {
        flags = wgpu::InstanceFlags::VALIDATION | wgpu::InstanceFlags::DEBUG;
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::PRIMARY, flags, ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let wanted = scene.surfaces.clone();

    println!(
        "pleamar · {} mode · the logic blocks {} ms after every decision",
        if a.naive { "NAIVE (single thread)" } else { "decoupled (independent render)" },
        a.stall
    );
    let _ = to_render.send(ToRender::Scene(scene));
    if std::path::Path::new(&a.scene).is_file() {
        scenes::from_file::watch(a.scene.clone(), to_render.clone(), to_logic.clone());
    }
    let to_logic_for_commands = to_logic.clone();
    let render = {
        let blocked = blocked.clone();
        let op = render::Options { hud: a.hud, naive: a.naive, reduced_motion: a.reduced, no_vsync: a.no_vsync, trace: a.record.clone(), start_time };
        let instance = instance.clone();
        std::thread::Builder::new()
            .name("render".into())
            .spawn(move || render::run(instance, from_render, letters, to_logic, blocked, op))
            .unwrap()
    };
    {
        let op = logic::Options { block_for: Duration::from_millis(a.stall), demo: a.demo, naive: a.naive, echo: a.mouse.is_some() };
        let tx = to_render.clone();
        std::thread::Builder::new()
            .name("logic".into())
            .spawn(move || logic::run(script, from_logic, tx, blocked, op))
            .unwrap();
    }

    // What is said from outside —`pleamar --say "emit toggle"`, which is what
    // a global compositor shortcut runs— comes in as if the logic had said it.
    {
        let name = std::path::Path::new(&a.scene).file_stem().map_or(a.scene.clone(), |n| n.to_string_lossy().into_owned());
        let tx = Mutex::new((to_render.clone(), to_logic_for_commands));
        platform::listen_for_commands(&name, Box::new(move |line| {
            let guard = tx.lock().unwrap();
            let (tx, to_logic) = &*guard;
            let mut p = line.trim().splitn(3, ' ');
            let (what, who, rest) = (p.next().unwrap_or(""), p.next().unwrap_or(""), p.next().unwrap_or(""));
            if what == "get" {
                let (question, answer) = std::sync::mpsc::channel();
                let _ = tx.send(ToRender::Query(scene::intern(who), question));
                return Some(answer.recv_timeout(std::time::Duration::from_secs(1)).unwrap_or_else(|_| "? the render does not answer".into()));
            }
            let _ = match what {
                "emit" => tx.send(ToRender::ExternalSignal(scene::intern(who), rest.parse().ok())),
                // What is set from outside, the logic has to know: it did not set it itself.
                // What is written is understood according to what that fact is: `true`, `critical`, `3`.
                "fact" => tx.send(ToRender::ExternalFact(scene::intern(who), rest.to_owned())),
                "text" => {
                    let _ = to_logic.send(Event::Text(scene::intern(who), rest.to_owned()));
                    tx.send(ToRender::Text(scene::intern(who), rest.to_owned()))
                }
                "focus" => tx.send(ToRender::FocusField(Some(scene::intern(who)))),
                "quit" => quit(),
                _ => {
                    eprintln!("orders · I don't understand '{line}'");
                    return Some(format!("? I don't understand '{}': emit, fact, text, focus, get, quit", line.trim()));
                }
            };
            None
        }));
    }
    if let Some(steps) = a.mouse.clone() {
        // To rehearse the zones without taking the mouse away from anyone.
        let tx = to_render.clone();
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            for step in steps.split_whitespace() {
                let (what, when) = step.split_once('@').expect("--mouse: @ms is missing");
                let when = Duration::from_millis(when.parse().expect("--mouse: ms"));
                std::thread::sleep(when.saturating_sub(start.elapsed()));
                // Each step is announced, to know what whatever comes after is responding to.
                println!("mouse  · {what}");
                let _ = match what {
                    // A whole click: down and up.
                    "click" => tx.send(ToRender::Button(0, true)).and_then(|_| tx.send(ToRender::Button(0, false))),
                    "down" => tx.send(ToRender::Button(0, true)),
                    "up" => tx.send(ToRender::Button(0, false)),
                    "right" => tx.send(ToRender::Button(1, true)).and_then(|_| tx.send(ToRender::Button(1, false))),
                    // `key:Escape`, `key:Ctrl+a`: down and up.
                    t if t.starts_with("key:") => {
                        let mut m = Mods::default();
                        let mut name = &t[4..];
                        for (prefix, sets) in [("Ctrl+", 0), ("Alt+", 1), ("Shift+", 2), ("Super+", 3)] {
                            if let Some(rest) = name.strip_prefix(prefix) {
                                name = rest;
                                match sets {
                                    0 => m.ctrl = true,
                                    1 => m.alt = true,
                                    2 => m.shift = true,
                                    _ => m.logo = true,
                                }
                            }
                        }
                        tx.send(ToRender::Key(name.to_owned(), None, m)).and_then(|_| tx.send(ToRender::KeyReleased(name.to_owned())))
                    }
                    // `type:hello`: letter by letter, like a keyboard. A `_` is a space.
                    t if t.starts_with("type:") => {
                        for ch in t[5..].chars() {
                            let ch = if ch == '_' { ' ' } else { ch };
                            let _ = tx.send(ToRender::Key(ch.to_string(), Some(ch.to_string()), Mods::default()));
                            let _ = tx.send(ToRender::KeyReleased(ch.to_string()));
                        }
                        Ok(())
                    }
                    "focus+" => tx.send(ToRender::KeyboardFocus(true)),
                    "focus-" => tx.send(ToRender::KeyboardFocus(false)),
                    t if t.starts_with("drop:") => tx.send(ToRender::Dropped("text/plain".into(), t[5..].to_owned())),
                    "wheel+" => tx.send(ToRender::Wheel(1.0)),
                    "wheel-" => tx.send(ToRender::Wheel(-1.0)),
                    "out" => tx.send(ToRender::Pointer(None)),
                    xy => {
                        let (x, y) = xy.split_once(',').expect("--mouse: x,y");
                        tx.send(ToRender::Pointer(Some((x.parse().unwrap(), y.parse().unwrap()))))
                    }
                };
            }
        });
    }
    if let Some(s) = a.seconds {
        let tx = to_render.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(s));
            // Quitting goes through the render so that it closes its last measurement cycle.
            let _ = tx.send(ToRender::Quit);
            std::thread::sleep(Duration::from_millis(400));
            quit();
        });
    }

    // From here on this thread belongs to the platform: it puts up the windows and attends
    // to the system until someone closes.
    let extra_height = if a.hud { gpu::HUD_HEIGHT as u32 } else { 0 };
    platform::run_event_loop(wanted, extra_height, instance, to_render.clone());
    let _ = to_render.send(ToRender::Quit);
    let _ = render.join();
    quit();
}

/// The process goes away whole —the destruction order does not deserve code in a
/// prototype—, but not without first stopping what the logic left running.
fn quit() -> ! {
    #[cfg(feature = "luau")]
    logic_luau::stop_children();
    std::process::exit(0)
}
