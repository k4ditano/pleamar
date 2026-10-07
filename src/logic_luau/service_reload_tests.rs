use super::*;
use std::sync::mpsc::{self, Receiver};

struct Running {
    script: LuauScript,
    context: Context,
    events: Receiver<Event>,
    rendered: Receiver<ToRender>,
}

impl Running {
    fn new(name: &str) -> Self {
        let (tx, rendered) = mpsc::channel();
        let (sender, events) = mpsc::channel();
        let blocked = Arc::default();
        let context = Context::for_plugin(tx.clone(), Arc::clone(&blocked));
        let script = LuauScript::new(&format!("service-reload-{name}.plm"), "", tx, sender, blocked);
        Self { script, context, events, rendered }
    }

    fn reload(&mut self, body: &str) {
        let path = std::env::temp_dir().join(format!("{}-{}", std::process::id(), self.script.scene));
        let source = format!("scene Services {{ surface {{ size: 100, 100 }} {body} }}");
        std::fs::write(&path, source).unwrap();
        let compiled = crate::language::read_file(path.to_str().unwrap());
        std::fs::remove_file(path).unwrap();
        let scene = compiled.unwrap().0;
        self.script.on_event(Event::NewScene(scene.facts, scene.texts, scene.permissions,
            scene.models, scene.types, scene.plugins, scene.signals.iter().map(|s| s.0).collect(),
            scene.services, scene.translations), &mut self.context);
    }

    fn next(&self) -> Event {
        self.events.recv_timeout(Duration::from_secs(3)).expect("the service did not report its state")
    }

    fn deliver(&mut self, event: Event) {
        self.script.on_event(event, &mut self.context);
    }

    fn value(&self, name: &str) -> f64 {
        self.script.c.lock().unwrap().facts[name]
    }
}

const CLOCK: &str = r#"permissions { services: "clock" }
    service clock as now { second: number = -1 }"#;
const SECONDS: &str = r#"permissions { services: "clock" }
    service clock.seconds as now { second: number = -1 }"#;

#[test]
fn added_fields_receive_the_quiet_services_snapshot() {
    let mut running = Running::new("added-field");
    running.reload(CLOCK);
    running.deliver(running.next());
    running.reload(r#"permissions { services: "clock" }
        service clock as now { second: number = -1; year: number = -1 }"#);
    let snapshot = running.events.try_recv().expect("the new field must not wait for the next minute");
    running.deliver(snapshot);
    assert!(running.value("now.year") > 1900.0);
}

#[test]
fn rebinding_an_alias_starts_the_new_service_and_discards_old_data() {
    let mut running = Running::new("rebind");
    running.reload(CLOCK);
    let old = running.next();
    running.reload(SECONDS);
    running.deliver(running.next());
    assert!(running.value("now.second") >= 0.0);
    running.deliver(Event::Fact("now.second", -2.0));
    running.deliver(old);
    assert_eq!(running.value("now.second"), -2.0, "a queued snapshot from the old source was applied");
    // The replacement must continue ticking, not merely replay the old clock.
    running.deliver(running.next());
    assert!(running.value("now.second") >= 0.0);
}

#[test]
fn revoking_permission_rejects_an_already_queued_snapshot() {
    let mut running = Running::new("revoke");
    running.reload(SECONDS);
    let old = running.next();
    running.reload("service clock.seconds as now { second: number = -1 }");
    running.deliver(old);
    assert_eq!(running.value("now.second"), -1.0, "revoked service data reached the scene");
    // The native source keeps running, but must stop filling this mailbox.
    running.events.try_iter().for_each(drop);
    assert!(running.events.recv_timeout(Duration::from_millis(1300)).is_err());
    running.reload(SECONDS);
    running.deliver(running.next());
    assert!(running.value("now.second") >= 0.0, "restoring permission must restore the service");
}

#[test]
fn removing_and_restoring_a_service_replays_without_accepting_stale_data() {
    let mut running = Running::new("restore");
    running.reload(CLOCK);
    let old = running.next();
    running.reload("");
    running.reload(CLOCK);
    running.deliver(old);
    assert_eq!(running.value("now.second"), -1.0, "the removed subscription became valid again");
    running.deliver(running.events.try_recv().expect("a restored quiet service needs its snapshot"));
    assert!(running.value("now.second") >= 0.0);
}

#[test]
fn restored_quiet_fields_reach_the_renderer_even_when_the_value_is_unchanged() {
    let mut running = Running::new("restore-render");
    let declaration = r#"permissions { services: "clock" }
        service clock as now { second: number = -1; time: text = "" }"#;
    running.reload(declaration);
    let initial = running.next();
    let Event::ServiceData(_, _, value) = initial.clone() else { panic!("clock snapshot") };
    running.deliver(initial);
    running.rendered.try_iter().for_each(drop);
    running.reload("");
    running.reload(declaration);
    let Event::ServiceData(alias, live, _) = running.next() else { panic!("restored snapshot") };
    // Freeze the value across removal, irrespective of the wall-clock minute.
    running.deliver(Event::ServiceData(alias, live, value));
    let rendered: Vec<_> = running.rendered.try_iter().collect();
    assert!(rendered.iter().any(|e| matches!(e, ToRender::Fact("now.second", n) if *n >= 0.0)),
        "the renderer recreated the numeric field at its default but received no snapshot");
    assert!(rendered.iter().any(|e| matches!(e, ToRender::Text("now.time", s) if !s.is_empty())),
        "the renderer recreated the text field at its default but received no snapshot");
}

#[test]
fn reload_preserves_retained_fields_and_retires_removed_ones() {
    let mut running = Running::new("field-lifetime");
    running.reload("fact kept = 1; fact removed = 2; text label = \"initial\"; text gone = \"old\"");
    running.deliver(Event::Fact("kept", 42.0));
    running.deliver(Event::Text("label", "current".into()));
    running.reload("fact kept = 0; text label = \"default\"");
    let state = running.script.c.lock().unwrap();
    assert_eq!(state.facts["kept"], 42.0);
    assert_eq!(state.texts["label"], "current");
    assert!(!state.facts.contains_key("removed"));
    assert!(!state.texts.contains_key("gone"));
}

#[test]
fn dropping_the_scene_stops_service_delivery() {
    let mut running = Running::new("drop");
    running.reload(SECONDS);
    running.next();
    drop(running.script);
    running.events.try_iter().for_each(drop);
    assert!(running.events.recv_timeout(Duration::from_millis(1300)).is_err(),
        "a retired scene is still receiving native service data");
}

#[test]
fn plugins_receive_current_declarations_but_reject_retired_snapshots() {
    let mut running = Running::new("plugin");
    let mut plugin = Running::new("plugin-consumer");
    plugin.script.prefix = Some("Plugin".into());
    running.reload(SECONDS);
    plugin.reload(SECONDS);
    let current = running.next();
    plugin.deliver(current.clone());
    assert!(plugin.value("now.second") >= 0.0);
    running.reload("service clock.seconds as now { second: number = -1 }");
    plugin.deliver(Event::Fact("now.second", -2.0));
    plugin.deliver(current);
    assert_eq!(plugin.value("now.second"), -2.0, "a plugin accepted a retired scene subscription");
}
