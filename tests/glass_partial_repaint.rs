//! Issue 152: a glass with no lens over something of the scene's own came out
//! one way when the surface was painted whole and another when only a piece
//! of it was (a flat grey card, or one flickering between the two). Two scenes
//! —`tests/glass-shadow-partial.plm`, the one reported, with nothing under the
//! glass but its own shadow, and `tests/glass-over-own-partial.plm`, with
//! stripes and a text under it— are filmed as a surface handed to another
//! compositor (`PLEAMAR_FILM_HANDED=1`), once painted whole every frame
//! (`PLEAMAR_FULL_REPAINT=1`) and once in pieces, and every frame has to be
//! the same. Without a card to paint with, it says so and passes. It asks for
//! a card, like `gpu::card_full`: `cargo test --test glass_partial_repaint -- --ignored`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The second card's tint turning on and off (only its piece changes), then the first's.
const SCRIPT: &str = "emit:t1@100 emit:t1@220 emit:t1@340 emit:t1@460 emit:t1@580 emit:t1@700 emit:t0@820";

fn has_card() -> bool {
    let instance = wgpu::Instance::default();
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).is_ok()
}

fn film(dir: &Path, scene: &str, whole: bool) -> PathBuf {
    let out = dir.join(format!("{scene}-{}", if whole { "whole" } else { "pieces" }));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    // A home of its own: nothing of the person's (skills, sockets, settings) is read or written.
    let home = dir.join("home");
    for d in [".config", ".cache", ".local/share", ".local/state", "run"] {
        std::fs::create_dir_all(home.join(d)).unwrap();
    }
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
        .env("PLEAMAR_NO_SKILL_REFRESH", "1")
        .env("PLEAMAR_FILM_HANDED", "1");
    if whole {
        c.env("PLEAMAR_FULL_REPAINT", "1");
    }
    c.arg("--scene").arg(&scene).arg("--film").arg(&out).args(["--seconds", "1.2", "--mouse", SCRIPT]);
    let done = c.output().expect("pleamar runs");
    assert!(done.status.success(), "the film failed: {}", String::from_utf8_lossy(&done.stderr));
    out
}

fn frames(dir: &Path) -> Vec<(String, image::RgbaImage)> {
    let mut names: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "png")).collect();
    names.sort();
    names.into_iter().map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), image::open(&p).unwrap().to_rgba8())).collect()
}

#[test]
#[ignore = "films with a graphics card (some 3 minutes in debug on a software one); run with --ignored"]
fn glass_over_its_own_is_the_same_painted_whole_or_in_pieces() {
    if !has_card() {
        eprintln!("no card to paint with: not filmed");
        return;
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("glass-partial-{}", std::process::id()));
    let mut differing = Vec::new();
    for scene in ["glass-shadow-partial", "glass-over-own-partial"] {
        let whole = frames(&film(&dir, scene, true));
        let pieces = frames(&film(&dir, scene, false));
        assert!(whole.len() >= 60, "{scene}: only {} frames filmed", whole.len());
        assert_eq!(whole.len(), pieces.len());
        let mut here = Vec::new();
        for ((name, a), (_, b)) in whole.iter().zip(&pieces) {
            assert_eq!(a.dimensions(), b.dimensions());
            let worst = a.as_raw().iter().zip(b.as_raw()).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
            if worst > 2 {
                here.push(format!("{name} (by {worst})"));
            }
        }
        if !here.is_empty() {
            differing.push(format!("{scene}: {} of {} frames, {}", here.len(), whole.len(), here.iter().take(6).cloned().collect::<Vec<_>>().join(", ")));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(differing.is_empty(), "painted in pieces, frames differ from painted whole: {}", differing.join(" · "));
}
