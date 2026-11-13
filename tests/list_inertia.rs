//! A list dragged by one of its rows goes on gliding when it is let go, with the speed of
//! its last moments, slows down smoothly, never goes past its ends, and does not move at
//! all after a click. It is filmed (`--film`, on a virtual clock: a frame is a 60th of a
//! second whatever the machine) with a mouse script, and the list's scroll is read frame by
//! frame (`--record`), so what it does is the same everywhere. Without a card to paint with,
//! it says so and passes, like `glass_partial_repaint`: `cargo test --test list_inertia -- --ignored`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn has_card() -> bool {
    let instance = wgpu::Instance::default();
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).is_ok()
}

/// One frame of a film: when (ms), the list's scroll, and how many rows were pressed.
type Frame = (f32, f32, f32);

fn film(scene: &str, script: &str, extra: &[&str]) -> Vec<Frame> {
    // (The tests run side by side: each film has a folder of its own.)
    static FILMS: AtomicUsize = AtomicUsize::new(0);
    let dir: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("list-inertia-{}-{}", std::process::id(), FILMS.fetch_add(1, Ordering::Relaxed)));
    // A home of its own: nothing of the person's (skills, sockets, settings) is read or written.
    let home = dir.join("home");
    for d in [".config", ".cache", ".local/share", ".local/state", "run"] {
        std::fs::create_dir_all(home.join(d)).unwrap();
    }
    let out = dir.join("frames");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let scene = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(format!("{scene}.plm"));
    let mut c = Command::new(env!("CARGO_BIN_EXE_pleamar"));
    c.env_clear();
    for keep in ["PATH", "LD_LIBRARY_PATH", "VK_ICD_FILENAMES", "VK_DRIVER_FILES", "LIBGL_ALWAYS_SOFTWARE", "WGPU_BACKEND", "WGPU_ADAPTER_NAME"] {
        if let Some(v) = std::env::var_os(keep) {
            c.env(keep, v);
        }
    }
    c.env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_STATE_HOME", home.join(".local/state"))
        .env("XDG_RUNTIME_DIR", home.join("run"))
        .env("PLEAMAR_SOCKETS", home.join("run"))
        .env("PLEAMAR_NO_SKILL_REFRESH", "1");
    c.arg("--scene").arg(&scene).arg("--film").arg(&out).args(["--seconds", "4", "--record", "list.scroll,opened", "--mouse", script]).args(extra);
    let done = c.output().expect("pleamar runs");
    assert!(done.status.success(), "the film failed: {}", String::from_utf8_lossy(&done.stderr));
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&done.stdout)
        .lines()
        .filter_map(|l| {
            let mut v = l.split('\t').map(|x| x.parse::<f32>());
            Some((v.next()?.ok()?, v.next()?.ok()?, v.next()?.ok()?))
        })
        .collect()
}

/// The scroll at the first frame at or after that time.
fn at(frames: &[Frame], ms: f32) -> f32 {
    frames.iter().find(|f| f.0 >= ms).map_or(f32::NAN, |f| f.1)
}

fn last(frames: &[Frame]) -> f32 {
    frames.last().map_or(f32::NAN, |f| f.1)
}

/// A drag by a row going up at about 1000 px/s, let go at 250 ms (the list's scroll goes up).
const FLICK: &str = "150,170@100 down@120 150,150@150 150,120@180 150,90@210 150,60@240 up@250";

#[test]
#[ignore = "films with a graphics card (a minute in debug on a software one); run with --ignored"]
fn a_list_let_go_with_speed_glides_on_and_stops_smoothly() {
    if !has_card() {
        eprintln!("no card to paint with: not filmed");
        return;
    }
    let with = film("list-inertia", FLICK, &[]);
    let without = film("list-no-inertia", FLICK, &[]);
    assert!(with.len() >= 200, "only {} frames filmed", with.len());
    // Without inertia it stops where the hand left it; with it, it goes a long way on.
    let stopped = last(&without);
    assert!((at(&without, 600.0) - stopped).abs() < 1.0, "`inertia: false` went on moving");
    assert!(last(&with) > stopped + 100.0, "it did not glide on: {} against {}", last(&with), stopped);
    // It keeps going after the hand is gone (the release is at 250 ms), always the same way, and ever more slowly.
    let after: Vec<f32> = with.iter().filter(|f| f.0 >= 300.0).map(|f| f.1).collect();
    let steps: Vec<f32> = after.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(steps.iter().all(|s| *s >= -0.01), "it went back while gliding");
    let moving = steps.iter().position(|s| *s > 0.5).expect("it moves after the release");
    let peak = steps[moving];
    assert!(peak > 1.0, "it should still be moving fast right after the release: {peak}");
    assert!(steps.windows(2).skip(moving).all(|w| w[1] <= w[0] + 0.05), "it slowed down and sped up again");
    // And it comes to rest: the last second does not move.
    assert!((at(&with, 3000.0) - last(&with)).abs() < 0.05, "it was still moving at 3 s");
}

#[test]
#[ignore = "films with a graphics card (a minute in debug on a software one); run with --ignored"]
fn a_list_never_glides_past_its_end() {
    if !has_card() {
        return;
    }
    // 6000 px/s: it would go far past the 1000 px there is to scroll.
    let frames = film("list-inertia", "150,190@100 down@110 150,130@120 150,70@130 150,10@140 up@150", &[]);
    let most = frames.iter().map(|f| f.1).fold(0.0, f32::max);
    assert!(most <= 1000.01, "it went past its end: {most}");
    assert!((last(&frames) - 1000.0).abs() < 0.5, "it should end at its end: {}", last(&frames));
    // Nor past its top, dragged the other way from where it is.
    let down = film("list-inertia", "150,10@100 down@110 150,70@120 150,130@130 150,190@140 up@150", &[]);
    assert!(down.iter().all(|f| f.1 >= -0.01), "it went past its top");
}

#[test]
#[ignore = "films with a graphics card (a minute in debug on a software one); run with --ignored"]
fn a_click_does_not_move_a_list_and_a_pause_before_letting_go_stops_it() {
    if !has_card() {
        return;
    }
    // A click on the first row that moves the mouse a pixel: nothing scrolls, and the row hears it (`repeat` names only its first copy's zone).
    let click = film("list-inertia", "150,15@100 down@120 150,16@140 up@160", &[]);
    assert!(click.iter().all(|f| f.1.abs() < 0.01), "a click moved the list");
    assert_eq!(click.last().unwrap().2, 1.0, "the click did not reach the row");
    // The hand stops for a third of a second before letting go: the list stops with it.
    let paused = film("list-inertia", "150,170@100 down@120 150,150@150 150,120@180 150,90@210 150,60@240 up@600", &[]);
    let rest = last(&paused);
    assert!((at(&paused, 700.0) - rest).abs() < 0.5, "it glided after a pause: {} to {}", at(&paused, 700.0), rest);
}

#[test]
#[ignore = "films with a graphics card (a minute in debug on a software one); run with --ignored"]
fn with_reduced_motion_there_is_no_inertia() {
    if !has_card() {
        return;
    }
    let reduced = film("list-inertia", FLICK, &["--reduced-motion"]);
    let old = film("list-no-inertia", FLICK, &["--reduced-motion"]);
    assert!((last(&reduced) - last(&old)).abs() < 0.5, "reduced motion glided: {} against {}", last(&reduced), last(&old));
}

#[test]
#[ignore = "films with a graphics card (a minute in debug on a software one); run with --ignored"]
fn touching_a_list_that_glides_stops_it_there() {
    if !has_card() {
        return;
    }
    let free = film("list-inertia", FLICK, &[]);
    // Pressed at 600 ms and let go without moving: it stays where it was touched.
    let touched = film("list-inertia", &format!("{FLICK} 150,100@590 down@600 up@650"), &[]);
    let there = at(&touched, 650.0);
    assert!(there < last(&free) - 20.0, "it was not on its way yet: {there}");
    assert!((last(&touched) - there).abs() < 0.5, "it went on gliding after being touched: {there} to {}", last(&touched));
}
