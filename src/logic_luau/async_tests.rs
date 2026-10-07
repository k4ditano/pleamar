use super::*;
use std::sync::mpsc::{self, Receiver};

struct Fixture {
    script: LuauScript,
    context: Context,
    events: Receiver<Event>,
    directory: std::path::PathBuf,
}

impl Fixture {
    fn new(label: &str, parent: Option<&LuauScript>) -> Self {
        let name = format!("async-test-{}-{label}", std::process::id());
        let (tx, _render) = mpsc::channel();
        let (sender, events) = mpsc::channel();
        let blocked = Arc::default();
        let context = Context::for_plugin(tx.clone(), Arc::clone(&blocked));
        let permissions = crate::scene::Permissions { commands: vec![], services: vec!["files".into(), "files.write".into()] };
        let mut script = if let Some(parent) = parent {
            // Two plugins can occupy the same position on different reloads;
            // their pending replies still need distinct identifiers.
            let definition = crate::scene::Plugin { name, logic: Default::default(), permissions: permissions.clone() };
            let mut plugin = parent.for_plugin(&definition);
            plugin.to_logic = sender;
            plugin
        } else {
            LuauScript::new(&format!("{name}.plm"), "", tx, sender, blocked)
        };
        {
            let mut c = script.c.lock().unwrap();
            c.permissions = permissions;
            c.facts.insert(qualified(&script.prefix, "answers"), 0.0);
            c.texts.insert(qualified(&script.prefix, "result"), String::new());
        }
        script.lua = Some(script.prepare().unwrap());
        let SysValue::Text(path) = crate::platform::query(&script.owner(), "files.folder", &[]).unwrap() else { panic!("no files directory") };
        Self { script, context, events, directory: path.into() }
    }

    fn exec(&self, source: &str) { self.script.lua.as_ref().unwrap().load(source).exec().unwrap(); }
    fn next(&self) -> Event { self.events.recv_timeout(Duration::from_secs(3)).expect("worker did not answer") }
    fn deliver(&mut self, event: Event) { self.script.on_event(event, &mut self.context); }
    fn result(&self) -> String { self.script.c.lock().unwrap().texts[&qualified(&self.script.prefix, "result")].clone() }
    fn answers(&self) -> f64 { self.script.c.lock().unwrap().facts[&qualified(&self.script.prefix, "answers")] }
    fn write(&self, value: &str) {
        crate::platform::command(&self.script.owner(), "files.write", &[SysValue::Text("state.txt".into()), SysValue::Text(value.into())]).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.script.release();
        let _ = std::fs::remove_file(self.directory.join("state.txt"));
        let _ = std::fs::remove_dir(&self.directory);
    }
}

const ASK: &str = r#"sys.ask_async("files.read", {"state.txt"}, function(value, error)
    text.result = if error then "denied" else value
    fact.answers += 1
end)"#;

#[test]
fn inserting_a_plugin_before_an_existing_one_does_not_cross_query_replies() {
    let parent = Fixture::new("parent", None);
    let mut existing = Fixture::new("existing", Some(&parent.script));
    existing.write("existing private state");
    existing.exec(ASK);
    let old_reply = existing.next();
    let mut inserted = Fixture::new("inserted", Some(&parent.script));
    inserted.write("inserted private state");
    inserted.exec(ASK);
    let new_reply = inserted.next();
    existing.deliver(new_reply.clone());
    assert_eq!(existing.answers(), 0.0, "a new plugin's reply ran the existing plugin's handler");
    inserted.deliver(old_reply.clone());
    assert_eq!(inserted.answers(), 0.0, "the new plugin consumed its neighbour's pending reply");
    existing.deliver(old_reply);
    inserted.deliver(new_reply);
    assert_eq!(existing.result(), "existing private state");
    assert_eq!(inserted.result(), "inserted private state");
}

#[test]
fn queued_file_commands_recheck_permission_before_touching_the_file() {
    let mut running = Fixture::new("queued-write", None);
    running.write("original");
    running.exec(ASK);
    running.deliver(running.next());
    {
        // Hold the state while queuing, then revoke before the worker can
        // inspect it. The operation itself is a real platform file write.
        let mut state = running.script.c.lock().unwrap();
        state.service_workers["files"].sender.send(ServiceRequest {
            id: u32::MAX, name: "files.write".into(),
            args: vec![SysValue::Text("state.txt".into()), SysValue::Text("unauthorized".into())], query: false,
        }).unwrap();
        state.permissions.services = vec!["files".into()];
    }
    let Event::Process(_, error, code) = running.next() else { panic!("command did not report its outcome") };
    assert_ne!(code, 0, "a queued command ran after its write permission was removed");
    assert!(error.contains("permission"), "{error}");
    assert_eq!(std::fs::read_to_string(running.directory.join("state.txt")).unwrap(), "original");
    running.script.c.lock().unwrap().permissions.services.push("files.write".into());
    running.exec(r#"sys.call_async("files.write", {"state.txt", "restored"}, function(error, code)
        assert(code == 0 and error == ""); fact.answers += 1
    end)"#);
    running.deliver(running.next());
    assert_eq!(running.answers(), 2.0);
    assert_eq!(std::fs::read_to_string(running.directory.join("state.txt")).unwrap(), "restored");
}

#[test]
fn queued_queries_recheck_read_permission() {
    let mut running = Fixture::new("queued-read", None);
    running.write("private");
    running.exec(ASK);
    running.deliver(running.next());
    {
        let mut state = running.script.c.lock().unwrap();
        state.service_workers["files"].sender.send(ServiceRequest {
            id: u32::MAX, name: "files.read".into(), args: vec![SysValue::Text("state.txt".into())], query: true,
        }).unwrap();
        state.permissions.services.clear();
    }
    let Event::Answer(_, result) = running.next() else { panic!("query did not answer") };
    assert!(result.is_err(), "a queued query read private data after permission was removed");
}

#[test]
fn a_queued_query_reply_cannot_disclose_data_after_permission_revocation() {
    let mut running = Fixture::new("queued-reply", None);
    running.write("private");
    running.exec(ASK);
    let reply = running.next();
    // Revoke permissions while keeping the fields used by the reply handler.
    running.deliver(Event::NewScene(vec![("answers", 0.0)], vec![("result", String::new())],
        Default::default(), vec![], vec![], vec![], vec![], vec![], vec![]));
    running.deliver(reply);
    assert_eq!(running.result(), "denied", "the revoked query's private result reached Luau");
    assert_eq!(running.answers(), 1.0, "the caller must still receive its failure callback");
}
