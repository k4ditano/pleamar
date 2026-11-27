//! Language 0.12: a press on a window's own bar (`handle: self`) is heard by `on press`, and the second of a
//! pair by `on double` too. The double press was found and the round it came in was taken for one with
//! nothing in it, so `on double` on such a bar never fired (Proeasy, Mirilla with its own bar); and its
//! rules never heard a `press`, only its logic. Filmed (`--film`, a virtual clock) with the mouse scripted
//! and the facts read frame by frame (`--record`). Without a card it says so and passes:
//! `cargo test --test handle_double -- --ignored` (with software Vulkan on a machine without one).

use std::path::{Path, PathBuf};
use std::process::Command;

fn has_card() -> bool {
    let instance = wgpu::Instance::default();
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).is_ok()
}

/// The last frame's `presses` and `doubles`.
fn film(script: &str) -> (f32, f32) {
    let dir: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("handle-double-{}", std::process::id()));
    // A home of its own: nothing of the person's (skills, sockets, settings) is read or written.
    let home = dir.join("home");
    for d in [".config", ".cache", ".local/share", ".local/state", "run"] {
        std::fs::create_dir_all(home.join(d)).unwrap();
    }
    let out = dir.join("frames");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let scene = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/handle-double.plm");
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
    c.arg("--scene").arg(&scene).arg("--film").arg(&out).args(["--no-hud", "--seconds", "4", "--record", "presses,doubles", "--mouse", script]);
    let done = c.output().expect("pleamar runs");
    assert!(done.status.success(), "the film failed: {}", String::from_utf8_lossy(&done.stderr));
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&done.stdout)
        .lines()
        .filter_map(|l| {
            let mut v = l.split('\t').skip(1).map(|x| x.parse::<f32>());
            Some((v.next()?.ok()?, v.next()?.ok()?))
        })
        .last()
        .expect("frames recorded")
}

#[test]
#[ignore = "films with a graphics card (software Vulkan on a machine without one); run with --ignored"]
fn a_double_press_on_a_windows_own_bar_is_a_double_and_each_press_a_press() {
    if !has_card() {
        eprintln!("no graphics card: not filmed");
        return;
    }
    // Two quick presses: two presses, one double.
    assert_eq!(film("80,20@500 click@800 click@1000"), (2.0, 1.0));
    // Two far apart in time: no double.
    assert_eq!(film("80,20@500 click@800 click@2000"), (2.0, 0.0));
    // Two far apart in place: no double.
    assert_eq!(film("80,20@500 click@800 200,20@900 click@1000"), (2.0, 0.0));
    // Three quick ones: the third starts another pair. Four: two doubles.
    assert_eq!(film("80,20@500 click@800 click@1000 click@1200"), (3.0, 1.0));
    assert_eq!(film("80,20@500 click@800 click@1000 click@1200 click@1400"), (4.0, 2.0));
}
