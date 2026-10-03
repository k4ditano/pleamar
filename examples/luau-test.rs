//! Run isolated logic assertions with the same Luau dependency as pleamar.
//! No renderer, window, IPC listener or native service is initialized.
use std::{path::PathBuf, process::ExitCode, time::{Duration, Instant}};

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let usage = "usage: luau-test [--compile-only] SCRIPT.luau";
    let first = args.next().ok_or(usage)?;
    let compile_only = first == "--compile-only";
    let path = PathBuf::from(if compile_only { args.next().ok_or(usage)? } else { first });
    if args.next().is_some() { return Err(usage.into()); }
    let source = std::fs::read(&path).map_err(|e| e.to_string())?;
    if source.len() > 2 * 1024 * 1024 { return Err("test script exceeds 2 MiB".into()); }
    let lua = mlua::Lua::new();
    lua.set_memory_limit(64 * 1024 * 1024).map_err(|e| e.to_string())?;
    lua.globals().set("log", lua.create_function(|_, values: mlua::Variadic<mlua::Value>| {
        let words = values.iter().map(|v| v.to_string()).collect::<mlua::Result<Vec<_>>>()?;
        println!("{}", words.join(" "));
        Ok(())
    }).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    lua.sandbox(true).map_err(|e| e.to_string())?;
    let function = lua.load(&source).set_name(path.to_string_lossy()).into_function().map_err(|e| e.to_string())?;
    if compile_only { return Ok(()); }
    let started = Instant::now();
    lua.set_interrupt(move |_| {
        if started.elapsed() > Duration::from_secs(2) { Err(mlua::Error::runtime("logic test exceeded two seconds")) }
        else { Ok(mlua::VmState::Continue) }
    });
    function.call::<()>(()).map_err(|e| e.to_string())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => { eprintln!("logic test: {error}"); ExitCode::FAILURE }
    }
}
