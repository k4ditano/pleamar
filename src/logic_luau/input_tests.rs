use super::*;
use std::io::{Read, Write};

#[test]
#[ignore = "bounded subprocess entry point for persistent stdin tests"]
fn input_probe() {
    if !std::env::args().any(|arg| arg == "logic_luau::input_tests::input_probe") { return; }
    if std::env::var("PLEAMAR_INPUT_MODE").as_deref() == Ok("blocked") {
        println!("INPUT_READY");
        std::io::stdout().flush().unwrap();
        std::thread::sleep(Duration::from_secs(8));
        std::process::exit(0);
    }
    // Both outputs fill before any input is read, including the initial text.
    std::io::stdout().write_all(&vec![b'o'; 128 * 1024]).unwrap();
    println!();
    std::io::stderr().write_all(&vec![b'e'; 128 * 1024]).unwrap();
    eprintln!("\nERROR_STREAM_COMPLETE");
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    assert_eq!(input, "ñ世界".repeat(32768) + "\nsecond ñ line\nthird\n");
    println!("INPUT_ORDER_COMPLETE");
    std::process::exit(17);
}

struct Fixture {
    script: LuauScript,
    context: Context,
    events: std::sync::mpsc::Receiver<Event>,
    exe: String,
}

impl Fixture {
    fn new() -> Self {
        let exe = std::env::current_exe().unwrap().to_string_lossy().into_owned();
        let (render, _rx) = std::sync::mpsc::channel();
        let (sender, events) = std::sync::mpsc::channel();
        let blocked = Arc::default();
        let context = Context::for_plugin(render.clone(), Arc::clone(&blocked));
        let mut script = LuauScript::new("input.plm", "", render, sender, blocked);
        script.c.lock().unwrap().permissions.commands = vec![exe.clone()];
        script.lua = Some(script.prepare().unwrap());
        Self { script, context, events, exe }
    }

    fn exec(&self, text: &str) {
        self.script.lua.as_ref().unwrap().load(text).exec().unwrap();
    }

    fn spawn(&self, mode: &str, initial: &str) {
        self.exec(&format!(r#"
            worker = spawn([==[{}]==], {{"--ignored", "--exact", "logic_luau::input_tests::input_probe", "--nocapture"}},
                function() end, function() end, {{stdin = "open", input = {initial}, errors = true,
                    env = {{PLEAMAR_INPUT_MODE = "{mode}"}}}})
        "#, self.exe));
    }

    fn ready(&self) {
        loop {
            match self.events.recv_timeout(Duration::from_secs(10)).unwrap() {
                Event::Line(_, line) if line == "INPUT_READY" => return,
                Event::Line(..) => {},
                _ => panic!("helper exited before becoming ready"),
            }
        }
    }

    fn finish(&mut self) -> (Vec<String>, i32) {
        let mut lines = Vec::new();
        loop {
            let event = self.events.recv_timeout(Duration::from_secs(10)).unwrap();
            match &event {
                Event::Line(_, text) => lines.push(text.clone()),
                Event::Process(_, _, code) => {
                    let code = *code;
                    self.script.on_event(event, &mut self.context);
                    let state = self.script.c.lock().unwrap();
                    assert!(state.running.is_empty());
                    assert!(state.inputs.is_empty());
                    assert!(state.processes.is_empty());
                    return (lines, code);
                }
                _ => panic!("unexpected event"),
            }
        }
    }
}

#[test]
fn persistent_input_drains_duplex_pipes_preserves_utf8_order_and_closes() {
    let mut fixture = Fixture::new();
    fixture.spawn("echo", "string.rep('ñ世界', 32768)");
    let other = Fixture::new();
    let id: u32 = fixture.script.lua.as_ref().unwrap().globals().get("worker").unwrap();
    other.exec(&format!("assert(not write({id}, 'foreign'))"));
    fixture.exec(r#"
        assert(write(worker, "\nsecond ñ line\n"))
        assert(write(worker, "third\n"))
        assert(write(worker))
        assert(not write(worker, "after EOF"))
        assert(not write(worker))
    "#);
    let (lines, code) = fixture.finish();
    assert_eq!(code, 17);
    assert!(lines.iter().any(|line| line == "INPUT_ORDER_COMPLETE"));
    assert!(lines.iter().any(|line| line == "ERROR_STREAM_COMPLETE"));
}

#[test]
fn stalled_input_returns_false_and_stops_the_helper() {
    let mut fixture = Fixture::new();
    fixture.spawn("blocked", "nil");
    fixture.ready();
    let state = fixture.script.c.clone();
    let unlocked = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        state.try_lock().is_ok()
    });
    let started = Instant::now();
    fixture.exec("assert(not write(worker, string.rep('x', 512 * 1024)))");
    assert!(started.elapsed() < Duration::from_secs(4));
    assert!(unlocked.join().unwrap(), "a pipe write held the logic state lock");
    assert_ne!(fixture.finish().1, 0);
    fixture.exec("assert(not write(worker, 'late'))");
}

#[test]
fn closing_immediately_preserves_initial_input_before_eof() {
    let mut fixture = Fixture::new();
    fixture.spawn("echo", r#"string.rep('ñ世界', 32768) .. '\nsecond ñ line\nthird\n'"#);
    fixture.exec("assert(write(worker))");
    let (lines, code) = fixture.finish();
    assert_eq!(code, 17);
    assert!(lines.iter().any(|line| line == "INPUT_ORDER_COMPLETE"));
}

#[test]
fn retirement_cancels_a_blocked_initial_input_without_waiting() {
    let mut fixture = Fixture::new();
    let started = Instant::now();
    fixture.spawn("blocked", "string.rep('x', 512 * 1024)");
    assert!(started.elapsed() < Duration::from_secs(2), "spawn waited on its initial input");
    fixture.ready();
    let started = Instant::now();
    fixture.script.release();
    assert_ne!(fixture.finish().1, 0);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn permission_revocation_closes_input_and_stops_the_helper() {
    let mut fixture = Fixture::new();
    fixture.spawn("blocked", "nil");
    fixture.ready();
    fixture.script.c.lock().unwrap().permissions.commands.clear();
    fixture.exec("assert(not write(worker, 'denied'))");
    assert_ne!(fixture.finish().1, 0);
}
