//! pleamar#154: with a machine that has no Open Sans, a text at a weight none of its faces has (600,
//! where there are 400 and 700) drew its digits from the emoji font, which has «0–9», `#` and `*` for
//! the keycap emoji. The sans-serif is now a family that is installed. Filmed (`--film`) with a fontconfig of
//! its own that has only Noto Sans Regular and Noto Color Emoji (the fonts are looked for where fonts
//! are, or in `PLEAMAR_TEST_FONTS`; without them it says so and passes), the digits at weight 600
//! in the default family come out the same as in Noto Sans by its name, and as at weight 400, which is the
//! one face there is: not as emoji. `cargo test --test font_digits -- --ignored`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn has_card() -> bool {
    let instance = wgpu::Instance::default();
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).is_ok()
}

/// A font file by name: in `PLEAMAR_TEST_FONTS`, or anywhere under the usual folders.
fn find(name: &str) -> Option<PathBuf> {
    fn walk(dir: &Path, name: &str, depth: u32) -> Option<PathBuf> {
        for e in std::fs::read_dir(dir).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() && depth > 0 {
                if let Some(f) = walk(&p, name, depth - 1) {
                    return Some(f);
                }
            } else if p.file_name().is_some_and(|n| n == name) {
                return Some(p);
            }
        }
        None
    }
    let mut roots: Vec<PathBuf> = std::env::var_os("PLEAMAR_TEST_FONTS").map(PathBuf::from).into_iter().collect();
    roots.extend(["/usr/share/fonts", "/usr/local/share/fonts", "/run/current-system/sw/share/X11/fonts"].map(PathBuf::from));
    roots.into_iter().find_map(|r| walk(&r, name, 5))
}

fn film(dir: &Path, fonts: &[PathBuf]) -> image::RgbaImage {
    let home = dir.join("home");
    let own = dir.join("fonts");
    for d in [home.join(".config"), home.join(".cache"), home.join(".local/share"), home.join(".local/state"), home.join("run"), own.clone(), dir.join("fontcache")] {
        std::fs::create_dir_all(d).unwrap();
    }
    for f in fonts {
        let _ = std::os::unix::fs::symlink(f, own.join(f.file_name().unwrap()));
    }
    let conf = dir.join("fonts.conf");
    std::fs::write(&conf, format!("<?xml version=\"1.0\"?>\n<fontconfig><dir>{}</dir><cachedir>{}</cachedir></fontconfig>\n", own.display(), dir.join("fontcache").display())).unwrap();
    let out = dir.join("frames");
    std::fs::create_dir_all(&out).unwrap();
    let scene = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/digits-weight.plm");
    let mut c = Command::new(env!("CARGO_BIN_EXE_pleamar"));
    c.env_clear();
    for keep in ["PATH", "LD_LIBRARY_PATH", "VK_ICD_FILENAMES", "VK_DRIVER_FILES", "LIBGL_ALWAYS_SOFTWARE", "WGPU_BACKEND", "WGPU_ADAPTER_NAME"] {
        if let Some(v) = std::env::var_os(keep) {
            c.env(keep, v);
        }
    }
    c.env("HOME", &home)
        .env("FONTCONFIG_FILE", &conf)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_STATE_HOME", home.join(".local/state"))
        .env("XDG_RUNTIME_DIR", home.join("run"))
        .env("PLEAMAR_SOCKETS", home.join("run"))
        .env("PLEAMAR_NO_SKILL_REFRESH", "1");
    c.arg("--scene").arg(&scene).arg("--film").arg(&out).args(["--seconds", "0.5"]);
    let done = c.output().expect("pleamar runs");
    assert!(done.status.success(), "the film failed: {}", String::from_utf8_lossy(&done.stderr));
    let mut names: Vec<PathBuf> = std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "png")).collect();
    names.sort();
    image::open(names.last().expect("frames")).unwrap().to_rgba8()
}

/// The pixels of a band of rows, `y` to `y + 40`, of the whole width.
fn band(img: &image::RgbaImage, y: u32) -> Vec<u8> {
    (y..y + 40).flat_map(|yy| (0..img.width()).flat_map(move |x| img.get_pixel(x, yy).0)).collect()
}

#[test]
#[ignore = "films with a graphics card; run with --ignored"]
fn digits_at_weight_600_are_not_the_emoji_fonts_where_there_is_no_open_sans() {
    if !has_card() {
        eprintln!("no card to paint with: not filmed");
        return;
    }
    let (Some(sans), Some(emoji)) = (find("NotoSans-Regular.ttf"), find("NotoColorEmoji.ttf")) else {
        eprintln!("no Noto Sans Regular and Noto Color Emoji to try with: not filmed");
        return;
    };
    let dir = std::env::temp_dir().join(format!("pleamar-digits-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let img = film(&dir, &[sans, emoji]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(img.dimensions(), (460, 150));
    let (default_600, named_600, default_400) = (band(&img, 5), band(&img, 55), band(&img, 105));
    // There is ink at all.
    assert!(default_600.chunks(4).filter(|p| p[0] > 200).count() > 200, "nothing was drawn");
    assert!(default_400 == named_600, "Noto Sans by its name at 600 is the one face there is, as at 400");
    assert!(default_600 == default_400, "the digits at weight 600 in the default family are not Noto Sans Regular: they come from another font");
}
