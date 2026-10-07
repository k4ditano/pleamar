use super::*;
use std::io::Write;

#[test]
#[ignore = "bounded subprocess entry point for run lifetime tests"]
fn run_lifetime_probe() {
    if !std::env::args().any(|arg| arg == "logic_luau::run_tests::run_lifetime_probe") { return; }
    std::fs::write("ready", "started").unwrap();
    std::thread::sleep(Duration::from_secs(3));
    std::fs::write("finished", "outlived its logic").unwrap();
    std::process::exit(23);
}

fn retiring_logic_stops_run(label: &str, collect: bool, reload: bool) {
    let folder = std::env::temp_dir().join(format!("pleamar-run-{label}-{}", std::process::id()));
    std::fs::create_dir(&folder).unwrap();
    let path = folder.join("logic.luau");
    let exe = std::env::current_exe().unwrap().to_string_lossy().into_owned();
    let (tx, _render) = std::sync::mpsc::channel();
    let (sender, events) = std::sync::mpsc::channel();
    let blocked = Arc::default();
    let mut context = Context::for_plugin(tx.clone(), Arc::clone(&blocked));
    let mut script = LuauScript::new("run.plm", path.to_str().unwrap(), tx, sender, blocked);
    script.c.lock().unwrap().permissions.commands = vec![exe.clone()];
    script.c.lock().unwrap().facts.insert("value".into(), 0.0);
    // The no-output case also has no callback: ownership must not depend on one.
    let callback = if collect { "function() fact.value = -1 end" } else { "nil" };
    std::fs::write(&path, format!(r#"
        run([==[{exe}]==], {{"--ignored", "--exact", "logic_luau::run_tests::run_lifetime_probe"}},
            {callback}, {{ cwd = ".", output = {collect} }})
    "#)).unwrap();
    script.load_script();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !folder.join("ready").is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(folder.join("ready").is_file(), "helper did not start");
    let started = Instant::now();
    if reload {
        std::fs::write(&path, "fact.value = 42").unwrap();
        script.load_script();
    } else {
        script.release();
    }
    let Event::Process(id, output, code) = events.recv_timeout(Duration::from_secs(10)).unwrap()
        else { panic!("missing child completion") };
    let elapsed = started.elapsed();
    script.on_event(Event::Process(id, output, code), &mut context);
    let value = script.c.lock().unwrap().facts["value"];
    drop(script);
    let survived = folder.join("finished").exists();
    for entry in std::fs::read_dir(&folder).unwrap() { std::fs::remove_file(entry.unwrap().path()).unwrap(); }
    std::fs::remove_dir(folder).unwrap();
    assert!(!survived, "run child continued writing after its logic was retired");
    assert!(elapsed < Duration::from_secs(2), "retirement waited for normal child exit: {elapsed:?}");
    assert_eq!(value, if reload { 42.0 } else { 0.0 }, "discarded callback ran");
}

#[test]
fn reload_stops_run_with_captured_output() { retiring_logic_stops_run("collect", true, true); }

#[test]
fn reload_stops_run_without_output_or_callback() { retiring_logic_stops_run("silent", false, true); }

#[test]
fn releasing_logic_stops_run() { retiring_logic_stops_run("release", true, false); }

#[test]
#[ignore = "subprocess entry point for run output tests"]
fn run_output_probe() {
    if !std::env::args().any(|arg| arg == "logic_luau::run_tests::run_output_probe") { return; }
    // More than a pipe buffer: discarding stderr must not block collected stdout.
    std::io::stderr().write_all(&vec![b'e'; 256 * 1024]).unwrap();
    println!("RUN_RESULT_ñ_世界");
    std::process::exit(23);
}

#[test]
fn completed_runs_preserve_output_and_exit_codes() {
    let exe = std::env::current_exe().unwrap().to_string_lossy().into_owned();
    let (tx, _render) = std::sync::mpsc::channel();
    let (sender, events) = std::sync::mpsc::channel();
    let mut script = LuauScript::new("run.plm", "", tx, sender, Arc::default());
    script.c.lock().unwrap().permissions.commands = vec![exe.clone()];
    script.lua = Some(script.prepare().unwrap());
    script.lua.as_ref().unwrap().load(format!(r#"
        run([==[{exe}]==], {{"--ignored", "--exact", "logic_luau::run_tests::run_output_probe", "--nocapture"}})
    "#)).exec().unwrap();
    let Event::Process(_, output, code) = events.recv_timeout(Duration::from_secs(10)).unwrap()
        else { panic!("missing child completion") };
    assert_eq!(code, 23);
    assert!(output.ends_with("RUN_RESULT_ñ_世界"), "{output:?}");
    assert!(!output.contains("eeee"));
}

#[test]
#[ignore = "subprocess entry point for bidirectional pipe tests"]
fn bidirectional_probe() {
    if !std::env::args().any(|arg| arg == "logic_luau::run_tests::bidirectional_probe") { return; }
    // Write before reading, exceeding pipe capacity in both directions.
    std::io::stdout().write_all(&vec![b'o'; 128 * 1024]).unwrap();
    println!();
    std::io::stderr().write_all(&vec![b'e'; 128 * 1024]).unwrap();
    eprintln!("\nSTDERR_COMPLETE");
    let mut input = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut input).unwrap();
    assert_eq!(input, "ñ世界".repeat(32768));
    assert_eq!(std::env::var("PLEAMAR_PIPE_TEST").unwrap(), "España-日本語");
    assert!(std::path::Path::new("marker").exists(), "scene-relative cwd was lost");
    println!("INPUT_ENV_CWD_COMPLETE");
    std::process::exit(17);
}

#[test]
fn run_and_spawn_drain_output_while_writing_inline_input() {
    let folder = std::env::temp_dir().join(format!("pleamar pipes ñ {}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("marker"), "owned fixture").unwrap();
    let exe = std::env::current_exe().unwrap().to_string_lossy().into_owned();
    for mode in ["run", "spawn"] {
        let (tx, _render) = std::sync::mpsc::channel();
        let (sender, events) = std::sync::mpsc::channel();
        let logic = folder.join("test.luau");
        let mut script = LuauScript::new("pipes.plm", logic.to_str().unwrap(), tx, sender, Arc::default());
        script.c.lock().unwrap().permissions.commands = vec![exe.clone()];
        script.lua = Some(script.prepare().unwrap());
        let callbacks = if mode == "spawn" { "function() end, function() end" } else { "nil" };
        script.lua.as_ref().unwrap().load(format!(r#"
            {mode}([==[{exe}]==], {{"--ignored", "--exact", "logic_luau::run_tests::bidirectional_probe", "--nocapture"}},
                {callbacks}, {{input = string.rep("ñ世界", 32768), cwd = ".", errors = true,
                    env = {{PLEAMAR_PIPE_TEST = "España-日本語"}}}})
        "#)).exec().unwrap();
        let mut lines = Vec::new();
        loop {
            match events.recv_timeout(Duration::from_secs(15)).expect("bidirectional pipes blocked") {
                Event::Line(_, line) => lines.push(line),
                Event::Process(_, output, code) => {
                    assert_eq!(code, 17, "{mode}: {output}");
                    if mode == "run" {
                        assert!(output.contains("INPUT_ENV_CWD_COMPLETE\n"));
                        assert!(output.ends_with("STDERR_COMPLETE"), "run did not append stderr after stdout");
                    }
                    else {
                        assert!(lines.iter().any(|l| l == "INPUT_ENV_CWD_COMPLETE"));
                        assert!(lines.iter().any(|l| l == "STDERR_COMPLETE"), "exit arrived before stderr");
                    }
                    break;
                }
                _ => panic!("unexpected command event"),
            }
        }
    }
    std::fs::remove_file(folder.join("marker")).unwrap();
    std::fs::remove_dir(folder).unwrap();
}
