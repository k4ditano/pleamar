//! A scene that comes from a file in the language, and that reloads by itself
//! when the file changes. The logic that comes with it is the minimum: it listens.

use crate::scene::*;
use crate::logic::{Context, Script};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// Reads and compiles a scene. The error already comes with its file, its line and its arrow.
pub fn read(path: &str) -> Result<Scene, String> {
    crate::language::read_file(path).map(|(e, _)| e)
}

pub struct FromFile {
    path: String,
}

impl FromFile {
    pub fn new(path: &str) -> Self {
        FromFile { path: path.to_owned() }
    }
}

impl Script for FromFile {
    fn scene(&mut self) -> Scene {
        read(&self.path).unwrap_or_else(|m| {
            eprintln!("{m}");
            std::process::exit(1)
        })
    }

    fn on_event(&mut self, e: Event, c: &mut Context) {
        match e {
            // Without a mouse, `--demo` fires the "demo" signal: the scene will say what it does with it.
            Event::Demo => c.signal("demo"),
            Event::Layer(_, _) => c.work(),
            Event::Text(n, v) => println!("logic  · '{n}' now says: {v:?}"),
            Event::Submit(n, v) => println!("logic  · enter in '{n}': {v:?}"),
            Event::Received(z, kind, d) => println!("logic  · something was dropped on '{z}', a {kind}: {d:?}"),
            Event::Signal(s, None) => println!("logic  · '{s}' happened"),
            Event::Signal(s, Some(v)) => println!("logic  · '{s}' happened, with {v:.2}"),
            _ => {}
        }
    }
}

/// The script for a file scene: if next to it there is a `.luau` with the same
/// name, that is its logic; if not, one that only listens.
pub fn script_for(path: &str, tx: Sender<ToRender>, to_logic: Sender<Event>, blocked: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Box<dyn Script> {
    let logic = logic_path_for(path);
    // There is logic if the scene has its `.luau`, if any of its plugins has its own,
    // or if the scene asks for some service: the script also sets that up and hands it out,
    // even if there is not a single line of Luau.
    #[cfg(feature = "luau")]
    if std::path::Path::new(&logic).is_file() || read(path).is_ok_and(|e| !e.plugins.is_empty() || !e.services.is_empty()) {
        return Box::new(crate::logic_luau::LuauScript::new(path, &logic, tx, to_logic, blocked));
    }
    let _ = (tx, to_logic, blocked, logic);
    Box::new(FromFile::new(path))
}

fn logic_path_for(path: &str) -> String {
    std::path::Path::new(path).with_extension("luau").to_string_lossy().into_owned()
}

fn publish_reload(scene: Scene, to_render: &Sender<ToRender>, notify_logic: impl FnOnce(Event)) -> bool {
    let event = Event::NewScene(scene.facts.clone(), scene.texts.clone(), scene.permissions.clone(),
        scene.models.clone(), scene.types.clone(), scene.plugins.clone(),
        scene.signals.iter().map(|s| s.0).collect(), scene.services.clone(), scene.translations.clone());
    // A service can replay its cached values as soon as logic is notified.
    // Queue their field definitions first, or the render discards those values
    // and a quiet service may never send the unchanged snapshot again.
    if to_render.send(ToRender::Scene(scene)).is_err() { return false; }
    notify_logic(event);
    true
}

/// Hot reload: it checks the file's date four times per second —which
/// works the same on any system— and, if it changed, reads it again. If it is
/// fine, the new scene replaces the old one without losing what was moving; if not,
/// it says why and the old one stays.
/// The program itself: if the binary changes —you just recompiled—, it relaunches
/// with the same arguments.
///
/// A live process keeps the compiler it was born with, so a
/// scene that uses something new in the language will not compile for it however many times it is
/// saved: it keeps the last good one and says so, which is correct, but
/// it looks as if hot reload does not work. This closes that hole.
///
/// `PLEAMAR_NO_RELAUNCH=1` turns it off, for whoever does not want updating the
/// package to restart their bar.
fn watch_binary() {
    // Windows locks a running executable. Stop it before replacing it; scene
    // and Luau reload remain available and do not replace the executable.
    #[cfg(target_os = "windows")]
    return;
    #[cfg(not(target_os = "windows"))]
    {
    if std::env::var_os("PLEAMAR_NO_RELAUNCH").is_some() || super::STAY_ON_UPDATE.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let Ok(me) = std::env::current_exe() else { return };
    let date = |r: &std::path::Path| std::fs::metadata(r).and_then(|m| m.modified()).ok();
    let Some(mut last) = date(&me) else { return };
    let _ = std::thread::Builder::new().name("relaunch".into()).spawn(move || loop {
        std::thread::sleep(Duration::from_millis(400));
        let Some(now) = date(&me) else { continue };
        if now == last {
            continue;
        }
        last = now;
        // Writing a binary is not instantaneous: it waits for it to stay still,
        // or it would relaunch with half a file.
        loop {
            std::thread::sleep(Duration::from_millis(150));
            match date(&me) {
                Some(d) if d == last => break,
                Some(d) => last = d,
                None => continue,
            }
        }
        println!("pleamar · the program changed on disk: starting again");
        let args: Vec<String> = std::env::args().skip(1).collect();
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let error = std::process::Command::new(&me).args(&args).exec();
            eprintln!("pleamar · could not start again: {error}");
        }
        #[cfg(not(unix))]
        {
            let _ = std::process::Command::new(&me).args(&args).spawn();
            std::process::exit(0);
        }
    });
    }
}

pub fn watch(path: String, to_render: Sender<ToRender>, to_logic: Sender<Event>) {
    watch_binary();
    // The logic reloads too: same trick, another file.
    let logic = logic_path_for(&path);
    let to_the_logic = to_logic.clone();
    let scene_for_logic = path.clone();
    std::thread::Builder::new()
        .name("reload-logic".into())
        .spawn(move || {
            // The scene's and its plugins': touching any of them reloads them.
            let all = |scene: &str| -> Vec<String> {
                let mut v = vec![logic.clone()];
                if let Ok(e) = read(scene) {
                    v.extend(e.plugins.iter().map(|p| p.logic.to_string_lossy().into_owned()));
                }
                v
            };
            let date = |v: &[String]| v.iter().map(|r| std::fs::metadata(r).and_then(|m| m.modified()).ok()).collect::<Vec<_>>();
            let mut watched = all(&scene_for_logic);
            let mut last = date(&watched);
            loop {
                std::thread::sleep(Duration::from_millis(250));
                let now = date(&watched);
                if now != last && now.iter().any(Option::is_some) {
                    std::thread::sleep(Duration::from_millis(80));
                    watched = all(&scene_for_logic);
                    last = date(&watched);
                    if to_logic.send(Event::ReloadLogic).is_err() {
                        return;
                    }
                }
            }
        })
        .unwrap();
    std::thread::Builder::new()
        .name("reload".into())
        .spawn(move || {
            // The scene and whatever it imports: touching a library also reloads it.
            // The scene, its libraries and whatever they bring attached —the SVGs of their
            // figures—: you draw the hat, save, and it is on.
            let mut watched: Vec<std::path::PathBuf> = crate::language::read_file(&path).map_or_else(
                |_| vec![path.clone().into()],
                |(e, mut f)| {
                    f.extend(e.attachments.iter().cloned());
                    f
                },
            );
            let date = |v: &[std::path::PathBuf]| v.iter().map(|r| std::fs::metadata(r).and_then(|m| m.modified()).ok()).collect::<Vec<_>>();
            let mut last = date(&watched);
            loop {
                std::thread::sleep(Duration::from_millis(250));
                let mut now = date(&watched);
                if now == last || now.iter().all(Option::is_none) {
                    continue;
                }
                // An editor saves in several steps —truncates, writes, sometimes renames—:
                // it waits for the file to have been still for a moment and not be empty.
                loop {
                    std::thread::sleep(Duration::from_millis(60));
                    let after = date(&watched);
                    // Only what is on disk: a library of pleamar's own
                    // (`pleamar:ui`) has no file, and taking it for one half
                    // saved left this waiting forever —no reload ever again—.
                    let empty = watched.iter().any(|r| std::fs::metadata(r).is_ok_and(|m| m.len() == 0));
                    if after == now && !empty {
                        break;
                    }
                    now = after;
                }
                let t0 = std::time::Instant::now();
                match crate::language::read_file(&path) {
                    Ok((e, files)) => {
                        println!("reload · {path} read in {:.1} ms{}", t0.elapsed().as_secs_f32() * 1000.0, if files.len() > 1 { format!(" · with {} libraries", files.len() - 1) } else { String::new() });
                        // It may import other things now.
                        watched = files;
                        watched.extend(e.attachments.iter().cloned());
                        let _ = to_render.send(ToRender::ReloadError(None));
                        if !publish_reload(e, &to_render, |event| { let _ = to_the_logic.send(event); }) {
                            return;
                        }
                    }
                    Err(m) => {
                        eprintln!("reload · the scene stays as it was:\n{m}");
                        let _ = to_render.send(ToRender::ReloadError(Some(m)));
                    }
                }
                last = date(&watched);
            }
        })
        .unwrap();
}

#[cfg(test)]
mod tests {
    use crate::scene::{Rule, Trigger};

    #[test]
    fn scene_definition_precedes_synchronous_logic_snapshot() {
        use super::*;
        let (render, queued) = std::sync::mpsc::channel();
        let mut scene = Scene::default();
        scene.facts.push(("now.year", -1.0));
        scene.texts.push(("now.label", String::new()));
        // Exercise the fastest possible service replay: before the notifier
        // returns. Slow or absent rendering must preserve the same ordering.
        assert!(publish_reload(scene, &render, |event| {
            assert!(matches!(event, Event::NewScene(..)));
            render.send(ToRender::Fact("now.year", 2026.0)).unwrap();
            render.send(ToRender::Text("now.label", "cached snapshot".into())).unwrap();
        }));
        let ToRender::Scene(mut current) = queued.recv().unwrap() else {
            panic!("a service value arrived before its field existed");
        };
        for message in queued.try_iter() {
            match message {
                ToRender::Fact(name, value) => current.facts.iter_mut().find(|f| f.0 == name).unwrap().1 = value,
                ToRender::Text(name, value) => current.texts.iter_mut().find(|f| f.0 == name).unwrap().1 = value,
                _ => panic!("unexpected reload message"),
            }
        }
        assert_eq!(current.facts[0].1, 2026.0);
        assert_eq!(current.texts[0].1, "cached snapshot");
    }

    /// A zone made in a `repeat` of a scene with one copy per monitor: each copy
    /// has its own, and each copy's rule presses its own. Both used to be one
    /// name, the second copy's, and the first monitor's zone was deaf.
    #[test]
    fn repeated_zones_belong_to_their_screen_copy() {
        let dir = std::env::temp_dir().join(format!("pleamar-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("copies.plm");
        std::fs::write(&path, "scene Copies {
    surface { size: 200, 40; anchor: top; screens: each max 2 }
    fact hit = 0
    repeat k in 1..3 {
        prop glow.$k = 0 ~140ms
        zone box glow.$k { at: 40 * k, 20; size: 30, 30 }
        on press glow.$k { hit = k; glow.$k: 1 }
    }
}
").unwrap();
        let scene = super::read(path.to_str().unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let ids: Vec<&str> = scene.zones.iter().map(|z| z.id).collect();
        for name in ["glow.1#screen0", "glow.1#screen1", "glow.2#screen0", "glow.2#screen1"] {
            assert!(ids.contains(&name), "no zone {name} in {ids:?}");
        }
        let pressed: Vec<&str> = scene.rules.iter().filter_map(|r: &Rule| match r.when {
            Trigger::Press(z) => Some(scene.zones[z.0 as usize].id),
            _ => None,
        }).collect();
        for name in ["glow.1#screen0", "glow.1#screen1"] {
            assert!(pressed.contains(&name), "no rule presses {name}: {pressed:?}");
        }
    }

    /// A rule that names nothing of its copy is the same rule once per monitor:
    /// only the first one may act, or a count goes up twice per monitor.
    #[test]
    fn a_shared_rule_acts_once_whatever_the_copies() {
        let dir = std::env::temp_dir().join(format!("pleamar-twins-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("twins.plm");
        std::fs::write(&path, "scene Twins {
    surface { size: 200, 40; anchor: top; screens: each max 2 }
    event landed
    fact n = 0
    on landed { n = n + 1 }
    zone box hit { from: 0, 0; size: 20, 20 }
    on press hit { n = 0 }
}
").unwrap();
        let scene = super::read(path.to_str().unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let twins: Vec<usize> = (0..scene.rules.len()).filter(|&k| scene.twin_of[k] != k).collect();
        // The count is one rule written twice; the press is each copy's own zone.
        assert_eq!(twins.len(), 1, "twins: {:?}", scene.twin_of);
        assert!(matches!(scene.rules[twins[0]].when, crate::scene::Trigger::On(_)));
    }
}
