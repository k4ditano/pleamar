use super::*;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

struct LogicFile(PathBuf);

impl Drop for LogicFile {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
}

struct FileState { owner: String, directory: PathBuf }

impl FileState {
    fn new(owner: String, value: &str) -> Self {
        let SysValue::Text(directory) = crate::platform::query(&owner, "files.folder", &[]).unwrap() else { panic!("file directory missing") };
        crate::platform::command(&owner, "files.write", &[SysValue::Text("state.txt".into()), SysValue::Text(value.into())]).unwrap();
        Self { owner, directory: directory.into() }
    }
}

impl Drop for FileState {
    fn drop(&mut self) {
        let _ = crate::platform::command(&self.owner, "files.remove", &[SysValue::Text("state.txt".into())]);
        // Only remove our empty, uniquely named owner directory, never recursively.
        let _ = std::fs::remove_dir(&self.directory);
    }
}

struct Watching {
    script: LuauScript,
    context: Context,
    events: Receiver<Event>,
    file: LogicFile,
}

impl Watching {
    fn new(name: &str, plugin: bool) -> Self {
        let name = format!("watch-test-{}-{name}", std::process::id());
        let path = std::env::temp_dir().join(format!("{name}.luau"));
        let (tx, _render) = mpsc::channel();
        let (sender, events) = mpsc::channel();
        let blocked = Arc::default();
        let context = Context::for_plugin(tx.clone(), Arc::clone(&blocked));
        let mut script = LuauScript::new(&format!("{name}.plm"), path.to_str().unwrap(), tx, sender, blocked);
        if plugin { script.prefix = Some(name); }
        {
            let mut c = script.c.lock().unwrap();
            c.permissions.services = vec!["clock".into(), "files".into()];
            for field in ["updates", "late"] { c.facts.insert(qualified(&script.prefix, field), 0.0); }
            c.texts.insert(qualified(&script.prefix, "result"), String::new());
        }
        Self { script, context, events, file: LogicFile(path) }
    }

    fn load(&mut self, source: &str) {
        std::fs::write(&self.file.0, source).unwrap();
        self.script.load_script();
        assert!(self.script.lua.is_some(), "Luau did not initialize");
    }

    fn next(&self) -> Event {
        self.events.recv_timeout(Duration::from_secs(3)).expect("watcher did not deliver data")
    }

    fn deliver(&mut self, event: Event) { self.script.on_event(event, &mut self.context); }

    fn updates(&self) -> f64 { self.script.c.lock().unwrap().facts[&qualified(&self.script.prefix, "updates")] }

    fn permissions(&mut self, services: Vec<String>) {
        let facts = ["updates", "late"].into_iter().map(|name| (intern(&qualified(&self.script.prefix, name)), 0.0)).collect();
        let texts = vec![(intern(&qualified(&self.script.prefix, "result")), String::new())];
        self.deliver(Event::NewScene(facts, texts, crate::scene::Permissions { commands: vec![], services },
            vec![], vec![], vec![], vec![], vec![], vec![]));
    }
}

const WATCH: &str = r#"assert(sys.watch("clock.seconds", function() fact.updates += 1 end))"#;

#[test]
fn failed_reload_restores_the_previous_vm_and_native_watch() {
    let mut running = Watching::new("failed-candidate", false);
    running.load(WATCH);
    running.deliver(running.next());
    assert_eq!(running.updates(), 1.0);
    running.load(r#"
        assert(sys.watch("clock.seconds", function() fact.updates = 999 end))
        error("reject this candidate after registering its watch")
    "#);
    let until = Instant::now() + Duration::from_secs(3);
    while running.updates() < 2.0 && Instant::now() < until {
        running.deliver(running.next());
    }
    assert_eq!(running.updates(), 2.0, "the previous VM lost its native watch or ran the candidate's handler");
    running.events.try_iter().for_each(drop);
    running.deliver(running.next());
    assert_eq!(running.updates(), 3.0, "watch restoration must also receive future ticks");
}

#[test]
fn watched_files_are_delivered_only_to_their_owner() {
    let mut scene = Watching::new("scene-files", false);
    let mut plugin = Watching::new("plugin-files", true);
    let _scene_file = FileState::new(scene.script.owner(), "scene state");
    let _plugin_file = FileState::new(plugin.script.owner(), "plugin state");
    let source = r#"assert(sys.watch("files:state.txt", function(value) text.result = value; fact.updates += 1 end))"#;
    scene.load(source);
    plugin.load(source);
    let from_scene = scene.next();
    let from_plugin = plugin.next();
    scene.deliver(from_scene.clone());
    plugin.deliver(from_scene);
    assert_eq!(plugin.updates(), 0.0, "a plugin received the scene's private file");
    scene.deliver(from_plugin.clone());
    plugin.deliver(from_plugin);
    assert_eq!(scene.updates(), 1.0, "the scene received its plugin's private file");
    assert_eq!(scene.script.c.lock().unwrap().texts["result"], "scene state");
    assert_eq!(plugin.script.c.lock().unwrap().texts[&qualified(&plugin.script.prefix, "result")], "plugin state");
}

#[test]
fn queued_watch_data_cannot_enter_a_reloaded_lua_vm() {
    let mut running = Watching::new("reload", false);
    running.load(WATCH);
    let old = running.next();
    running.load(&format!("fact.updates = 100\n{WATCH}"));
    running.deliver(old);
    assert_eq!(running.updates(), 100.0, "old data ran the new VM's handler");
    running.deliver(running.next());
    assert_eq!(running.updates(), 101.0);
}

#[test]
fn watch_permission_revocation_stops_delivery_and_restoration_replays() {
    let mut running = Watching::new("permissions", false);
    running.load(WATCH);
    let old = running.next();
    running.permissions(vec![]);
    running.deliver(old);
    assert_eq!(running.updates(), 0.0, "a revoked service still called Luau");
    running.events.try_iter().for_each(drop);
    assert!(running.events.recv_timeout(Duration::from_millis(1300)).is_err());
    running.permissions(vec!["clock".into()]);
    running.deliver(running.events.try_recv().expect("restored permission needs the current snapshot"));
    assert_eq!(running.updates(), 1.0);
    running.deliver(running.next());
    assert_eq!(running.updates(), 2.0);
}

#[test]
fn removed_watchers_stop_filling_the_logic_mailbox() {
    let mut running = Watching::new("removed", false);
    running.load(WATCH);
    running.next();
    running.load("fact.updates = 100");
    running.events.try_iter().for_each(drop);
    assert!(running.events.recv_timeout(Duration::from_millis(1300)).is_err(),
        "an unused native subscription is still filling the mailbox");
}

#[test]
fn dropping_a_watching_scene_stops_delivery() {
    let mut running = Watching::new("drop", false);
    running.load(WATCH);
    running.next();
    drop(running.script);
    running.events.try_iter().for_each(drop);
    assert!(running.events.recv_timeout(Duration::from_millis(1300)).is_err(),
        "the dropped scene still receives service events");
}

#[test]
fn a_late_listener_gets_the_snapshot_without_repeating_earlier_handlers() {
    let mut running = Watching::new("late", false);
    running.load(r#"assert(sys.watch("clock", function() fact.updates += 1 end))"#);
    running.deliver(running.next());
    running.script.lua.as_ref().unwrap().load(r#"
        assert(sys.watch("clock", function() fact.late += 1 end))
    "#).exec().unwrap();
    running.deliver(running.events.try_recv().expect("the late listener must not wait for the next minute"));
    assert_eq!(running.updates(), 1.0, "replaying to a late listener repeated the first handler");
    assert_eq!(running.script.c.lock().unwrap().facts["late"], 1.0);
}

#[test]
fn a_new_update_supersedes_the_queued_replay_to_a_late_listener() {
    let mut running = Watching::new("late-update", false);
    running.load(WATCH);
    running.deliver(running.next());
    let pending = running.next();
    running.script.lua.as_ref().unwrap().load(r#"
        assert(sys.watch("clock.seconds", function() fact.late += 1 end))
    "#).exec().unwrap();
    let replay = running.events.try_recv().expect("the late listener needs an initial snapshot");
    running.deliver(pending);
    running.deliver(replay);
    assert_eq!(running.updates(), 2.0);
    assert_eq!(running.script.c.lock().unwrap().facts["late"], 1.0, "a cached replay overwrote the newer update");
}
