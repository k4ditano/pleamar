//! R6: a scene that connects to pleamar-wm the instant its socket is there —before
//! the compositor has offered programs its card's buffers (linux-dmabuf), which
//! NVIDIA's Vulkan needs to show anything— found no card that could show on its
//! surface, fell back to OpenGL with a surface that instance did not know, and
//! wgpu panicked («non-existent resource SurfaceId»). Now it waits for the
//! compositor and shows its first frame.
//!
//! It needs a pleamar-wm and a card: `PLEAMAR_TEST_WM=path/to/pleamar-wm
//! PLEAMAR_TEST_WM_SCENE=path/to/session.plm cargo test --test compositor_not_ready -- --ignored`.
//! Without them it says so and passes.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
#[ignore = "needs a pleamar-wm (PLEAMAR_TEST_WM, PLEAMAR_TEST_WM_SCENE) and a card; run with --ignored"]
fn a_scene_that_comes_before_its_compositor_is_ready_waits_for_it() {
    let (Some(wm), Some(wm_scene)) = (std::env::var_os("PLEAMAR_TEST_WM"), std::env::var_os("PLEAMAR_TEST_WM_SCENE")) else {
        eprintln!("no PLEAMAR_TEST_WM / PLEAMAR_TEST_WM_SCENE: not tried");
        return;
    };
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("compositor-not-ready-{}", std::process::id()));
    let run = dir.join("run");
    for d in ["wm/.config", "wm/.cache", "scene/.config", "scene/.cache", "run"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    std::fs::set_permissions(&run, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    // Each with a home of its own, nothing of the person's session.
    let clean = |c: &mut Command, home: &Path| {
        c.env_clear();
        for keep in ["PATH", "LD_LIBRARY_PATH", "VK_ICD_FILENAMES", "VK_DRIVER_FILES"] {
            if let Some(v) = std::env::var_os(keep) {
                c.env(keep, v);
            }
        }
        c.env("HOME", home).env("XDG_CONFIG_HOME", home.join(".config")).env("XDG_CACHE_HOME", home.join(".cache")).env("XDG_RUNTIME_DIR", &run).env("PLEAMAR_SOCKETS", home.join("run")).env("PLEAMAR_NO_SKILL_REFRESH", "1");
    };
    let mut c = Command::new(&wm);
    clean(&mut c, &dir.join("wm"));
    c.env("PLEAMAR_HEADLESS_AT", "999999").arg("headless").arg(&wm_scene).args(["--seconds", "40"]).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut wm = c.spawn().expect("pleamar-wm starts");
    // Its socket, the moment it says it: before its card is up.
    let mut lines = BufReader::new(wm.stdout.take().unwrap()).lines();
    let started = Instant::now();
    let display = loop {
        let line = lines.next().and_then(Result::ok).expect("pleamar-wm says where programs connect");
        if let Some(d) = line.split("WAYLAND_DISPLAY=").nth(1) {
            break d.split_whitespace().next().unwrap().to_owned();
        }
        assert!(started.elapsed() < Duration::from_secs(20), "pleamar-wm said no socket");
    };
    // (What it says after is not waited for; it is read so it never blocks.)
    std::thread::spawn(move || for _ in lines {});
    let scene = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/screen-reader.plm");
    let mut c = Command::new(env!("CARGO_BIN_EXE_pleamar"));
    clean(&mut c, &dir.join("scene"));
    c.env("WAYLAND_DISPLAY", &display).arg("--scene").arg(&scene).args(["--seconds", "15"]);
    let out = c.output().expect("pleamar runs");
    let _ = wm.kill();
    let _ = wm.wait();
    let said = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!said.contains("panicked"), "it panicked:\n{said}");
    assert!(said.contains("render · first frame"), "no first frame:\n{said}");
}
