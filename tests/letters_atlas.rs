//! The letters have an atlas of their own (one byte a pixel: how much of each pixel a letter
//! covers, which the shader paints in the text's colour) and the images and emoji keep the
//! RGBA one. Filmed (`--film`, a virtual clock), `tests/letters-atlas.plm` has to show the
//! text in its colour —plain, and with an outline and a shadow—, dark text over a light box,
//! and the picture beside them, so that neither atlas is read in place of the other.
//! Without a card to paint with, it says so and passes: `cargo test --test letters_atlas -- --ignored`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn has_card() -> bool {
    let instance = wgpu::Instance::default();
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).is_ok()
}

fn film() -> image::RgbaImage {
    let dir: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("letters-atlas-{}", std::process::id()));
    let home = dir.join("home");
    for d in [".config", ".cache", ".local/share", ".local/state", "run"] {
        std::fs::create_dir_all(home.join(d)).unwrap();
    }
    let out = dir.join("frames");
    std::fs::create_dir_all(&out).unwrap();
    let scene = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/letters-atlas.plm");
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
    c.arg("--scene").arg(&scene).arg("--film").arg(&out).args(["--seconds", "1"]);
    let done = c.output().expect("pleamar runs");
    assert!(done.status.success(), "the film failed: {}", String::from_utf8_lossy(&done.stderr));
    let mut names: Vec<PathBuf> = std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "png")).collect();
    names.sort();
    let last = image::open(names.last().expect("frames")).unwrap().to_rgba8();
    let _ = std::fs::remove_dir_all(&dir);
    last
}

/// How many pixels of that box are within `by` of that colour, on each channel.
fn near(img: &image::RgbaImage, [x, y, w, h]: [u32; 4], [r, g, b]: [u8; 3], by: u8) -> usize {
    let mut n = 0;
    for yy in y..y + h {
        for xx in x..x + w {
            let p = img.get_pixel(xx, yy);
            if p[0].abs_diff(r) <= by && p[1].abs_diff(g) <= by && p[2].abs_diff(b) <= by {
                n += 1;
            }
        }
    }
    n
}

#[test]
#[ignore = "films with a graphics card; run with --ignored"]
fn letters_and_pictures_each_come_from_their_own_atlas() {
    if !has_card() {
        eprintln!("no card to paint with: not filmed");
        return;
    }
    let img = film();
    assert_eq!(img.dimensions(), (360, 150));
    // The yellow text: its letters, in its colour, not as a smudge of grey (the mask is only coverage).
    assert!(near(&img, [10, 10, 220, 30], [0xff, 0xd1, 0x66], 12) > 150, "the text is not in its colour");
    // The white title with its outline: letters in white, and the dark outline round them.
    assert!(near(&img, [10, 50, 200, 50], [0xff, 0xff, 0xff], 12) > 300, "the title is not white");
    // Dark text over the light box: the letters are darker than the box.
    assert!(near(&img, [8, 106, 50, 24], [0x10, 0x14, 0x18], 40) > 20, "dark text over a light box is not there");
    // The picture, from the RGBA atlas: not the background, not a mask.
    let background = near(&img, [280, 70, 64, 64], [0x14, 0x18, 0x1c], 3);
    assert!(background < 64 * 64 - 400, "the picture is missing: {background} of {} pixels are the background", 64 * 64);
}
