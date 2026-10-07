//! Explorer's wallpaper and native image preparation for scene transitions.
use super::SysValue;
use std::{collections::VecDeque, path::{Path, PathBuf}, hash::{Hash, Hasher}};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::{System::Com::*, UI::Shell::*};

fn desktop() -> Result<IDesktopWallpaper, String> {
    unsafe { CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL).map_err(|e| e.to_string()) }
}
unsafe fn take(raw: PWSTR) -> Result<String, String> {
    if raw.is_null() { return Ok(String::new()); }
    let value = unsafe { raw.to_string() }.map_err(|e| e.to_string());
    unsafe { CoTaskMemFree(Some(raw.0 as _)); }
    value
}
fn wide(value: &str) -> Vec<u16> { value.encode_utf16().chain([0]).collect() }

fn wait_for_wallpapers(expected: &[(String, String)]) -> Result<(), String> {
    let started = std::time::Instant::now();
    let mut confirmed_since = None;
    loop {
        // A writer can expose its requested value before Explorer commits it.
        // A fresh reader must observe a stable value before reporting success.
        let desktop = desktop()?;
        let mut matches = true;
        for (id, path) in expected {
            let actual = unsafe { take(desktop.GetWallpaper(PCWSTR(wide(id).as_ptr())).map_err(|e| e.to_string())?)? };
            matches &= actual.replace('/', "\\").eq_ignore_ascii_case(&path.replace('/', "\\"));
        }
        if matches {
            let since = confirmed_since.get_or_insert_with(std::time::Instant::now);
            if since.elapsed() >= std::time::Duration::from_millis(250) { return Ok(()); }
        } else {
            confirmed_since = None;
        }
        if started.elapsed() >= std::time::Duration::from_secs(5) {
            return Err("Windows has not confirmed the wallpaper change within five seconds".into());
        }
        // Explorer acknowledges SetWallpaper before it commits the new image.
        // This runs on the service worker, never the render/event thread.
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

struct WallpaperSnapshot {
    position: DESKTOP_WALLPAPER_POSITION,
    paths: Vec<(String, String)>,
}
impl WallpaperSnapshot {
    fn capture(desktop: &IDesktopWallpaper) -> Result<Self, String> {
        let mut paths = Vec::new();
        unsafe {
            for index in 0..desktop.GetMonitorDevicePathCount().map_err(|e| e.to_string())? {
                let id = take(desktop.GetMonitorDevicePathAt(index).map_err(|e| e.to_string())?)?;
                let path = take(desktop.GetWallpaper(PCWSTR(wide(&id).as_ptr())).map_err(|e| e.to_string())?)?;
                paths.push((id, path));
            }
            if paths.is_empty() { return Err("Windows did not report any wallpaper monitors".into()); }
            Ok(Self { position: desktop.GetPosition().map_err(|e| e.to_string())?, paths })
        }
    }

    fn restore(&self) -> Result<(), String> {
        let desktop = desktop()?;
        let mut failures = Vec::new();
        unsafe {
            // Attempt every monitor, even after a hot-unplug or a missing file.
            for (id, path) in &self.paths {
                if let Err(error) = desktop.SetWallpaper(PCWSTR(wide(id).as_ptr()), PCWSTR(wide(path).as_ptr())) {
                    failures.push(error.to_string());
                }
            }
            if let Err(error) = desktop.SetPosition(self.position) { failures.push(error.to_string()); }
        }
        if let Err(error) = wait_for_wallpapers(&self.paths) { failures.push(error); }
        if failures.is_empty() { Ok(()) } else { Err(failures.join("; ")) }
    }
}

fn state(desktop: &IDesktopWallpaper) -> Result<(String, SysValue), String> {
    let mut current = String::new();
    let mut monitors = Vec::new();
    unsafe {
        for index in 0..desktop.GetMonitorDevicePathCount().map_err(|e| e.to_string())? {
            let id = take(desktop.GetMonitorDevicePathAt(index).map_err(|e| e.to_string())?)?;
            let path = take(desktop.GetWallpaper(PCWSTR(wide(&id).as_ptr())).map_err(|e| e.to_string())?)?;
            if current.is_empty() { current = path.clone(); }
            monitors.push(SysValue::Map(vec![("id".into(), SysValue::Text(id)), ("path".into(), SysValue::Text(path))]));
        }
        let position = desktop.GetPosition().map_err(|e| e.to_string())?.0;
        Ok((current.clone(), SysValue::Map(vec![
            ("current".into(), SysValue::Text(current)),
            ("position".into(), SysValue::Num(position as f64)),
            ("monitors".into(), SysValue::List(monitors)),
        ])))
    }
}

fn image_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| matches!(ext.to_string_lossy().to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png" | "webp" | "bmp"))
}
fn catalog(current: &str) -> Vec<String> {
    use std::os::windows::fs::MetadataExt;
    let media_name = super::windows_media_paths::name().ok().map(|name| name.to_lowercase());
    let mut roots = VecDeque::new();
    if let Ok(pictures) = unsafe { SHGetKnownFolderPath(&FOLDERID_Pictures, KF_FLAG_DEFAULT, None).and_then(|p| {
        let result = p.to_string(); CoTaskMemFree(Some(p.0 as _)); Ok(result?)
    }) } { roots.push_back((PathBuf::from(pictures), 0)); }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        for name in ["Wallpapers", "Fondos"] { roots.push_back((PathBuf::from(&home).join(name), 0)); }
    }
    if let Some(system) = std::env::var_os("SystemRoot") { roots.push_back((PathBuf::from(system).join("Web/Wallpaper"), 0)); }
    let started = std::time::Instant::now();
    let mut paths = Vec::new();
    'walk: while let Some((folder, depth)) = roots.pop_front() {
        let Ok(entries) = std::fs::read_dir(folder) else { continue; };
        for entry in entries.flatten() {
            if paths.len() >= 200 || started.elapsed().as_millis() >= 500 { break 'walk; }
            let Ok(meta) = entry.metadata() else { continue; };
            if meta.file_attributes() & 0x400 != 0 { continue; }
            let path = entry.path();
            if meta.is_dir() && depth < 2 {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if !matches!(name.as_str(), "screenshots" | "capturas")
                    && media_name.as_ref() != Some(&name) {
                    roots.push_back((path, depth + 1));
                }
            } else if meta.is_file() && image_file(&path) {
                paths.push(path.to_string_lossy().into_owned());
            }
        }
    }
    paths.sort_by_cached_key(|p| Path::new(p).file_name().unwrap_or_default().to_string_lossy().to_lowercase());
    paths.dedup();
    paths.retain(|p| !p.eq_ignore_ascii_case(current));
    if Path::new(current).is_file() { paths.insert(0, current.into()); }
    paths
}

fn aspect(width: u32, height: u32) -> (u32, u32) {
    let (mut a, mut b) = (width, height);
    while b != 0 { (a, b) = (b, a % b); }
    (width / a, height / a)
}

fn preview(path: &Path, width: u32, height: u32) -> Result<String, String> {
    if !path.is_absolute() || !path.is_file() { return Err("wallpaper must be an existing absolute image path".into()); }
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let mut hash = std::hash::DefaultHasher::new();
    // Equal-aspect monitors use the same 1280x720 texture. Avoid decoding the
    // original twice just because one monitor has more pixels or another DPI.
    (path, aspect(width, height), meta.len(), meta.modified().ok()).hash(&mut hash);
    let cache = PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is not set")?).join("pleamar/wallpaper-cache");
    std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    let target = cache.join(format!("{:016x}.jpg", hash.finish()));
    if !target.is_file() {
        let mut reader = image::ImageReader::open(path).map_err(|e| e.to_string())?.with_guessed_format().map_err(|e| e.to_string())?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(128 * 1024 * 1024);
        limits.max_image_width = Some(16_384);
        limits.max_image_height = Some(16_384);
        reader.limits(limits);
        let image = reader.decode().map_err(|e| e.to_string())?;
        // Match the monitor's fill crop before the scene stretches its 16:9
        // transition texture back over that monitor.
        let ratio = width as f64 / height as f64;
        let cropped = image.resize_to_fill(1280, (1280.0 / ratio).round().clamp(1.0, 5120.0) as u32, image::imageops::FilterType::Triangle);
        let rgb = cropped.resize_exact(1280, 720, image::imageops::FilterType::Triangle).to_rgb8();
        let temp = target.with_extension(format!("{}.jpg", std::process::id()));
        rgb.save_with_format(&temp, image::ImageFormat::Jpeg).map_err(|e| e.to_string())?;
        if std::fs::rename(&temp, &target).is_err() { let _ = std::fs::remove_file(temp); }
        if !target.is_file() { return Err("could not save the wallpaper preview".into()); }
    }
    Ok(target.to_string_lossy().into_owned())
}

pub fn query(name: &str, args: &[SysValue]) -> Result<SysValue, String> {
    match (name, args) {
        ("wallpaper.state", []) => state(&desktop()?).map(|(_, value)| value),
        ("wallpaper.list", []) => {
            let (current, _) = state(&desktop()?)?;
            let items = catalog(&current).into_iter().map(|path| {
                let name = Path::new(&path).file_stem().unwrap_or_default().to_string_lossy().into_owned();
                SysValue::Map(vec![("name".into(), SysValue::Text(name)), ("path".into(), SysValue::Text(path))])
            }).collect();
            Ok(SysValue::Map(vec![("current".into(), SysValue::Text(current)), ("items".into(), SysValue::List(items))]))
        }
        ("wallpaper.preview", [SysValue::Text(path), SysValue::Num(width), SysValue::Num(height)]) => {
            if !width.is_finite() || !height.is_finite() || !(1.0..=16_384.0).contains(width) || !(1.0..=16_384.0).contains(height) {
                return Err("preview dimensions must be between 1 and 16384".into());
            }
            preview(Path::new(path), *width as u32, *height as u32).map(SysValue::Text)
        }
        _ => Err("unknown wallpaper query or invalid arguments".into()),
    }
}

pub fn command(name: &str, args: &[SysValue]) -> Result<(), String> {
    let ("wallpaper.set", [SysValue::Text(path)]) = (name, args) else { return Err("wallpaper.set takes an image path".into()); };
    if path.contains('\0') || !Path::new(path).is_absolute() || !Path::new(path).is_file() || !image_file(Path::new(path)) {
        return Err("wallpaper must be an existing absolute image path".into());
    }
    let desktop = desktop()?;
    unsafe {
        let previous = WallpaperSnapshot::capture(&desktop)?;
        let expected = previous.paths.iter().map(|(id, _)| (id.clone(), path.clone())).collect::<Vec<_>>();
        let result = (|| {
            desktop.SetPosition(DWPOS_FILL).map_err(|e| e.to_string())?;
            desktop.SetWallpaper(PCWSTR::null(), PCWSTR(wide(path).as_ptr())).map_err(|e| e.to_string())?;
            wait_for_wallpapers(&expected)
        })();
        if let Err(error) = result {
            return match previous.restore() {
                Ok(()) => Err(format!("could not change wallpaper: {error}; previous wallpaper restored")),
                Err(rollback) => Err(format!("could not change wallpaper: {error}; restoration also failed: {rollback}")),
            };
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_cache_depends_on_aspect_not_resolution_or_dpi() {
        assert_eq!(aspect(2560, 1440), aspect(1920, 1080));
        assert_eq!(aspect(2048, 1152), aspect(1536, 864));
        assert_ne!(aspect(2048, 1152), aspect(864, 1536));
        assert_eq!(aspect(1, 16384), (1, 16384));
    }
    #[test]
    fn invalid_requests_do_not_change_the_desktop() {
        for n in [f64::NAN, f64::INFINITY, 0.0, -1.0, 16_385.0] {
            assert!(query("wallpaper.preview", &[SysValue::Text("missing".into()), SysValue::Num(n), SysValue::Num(1080.0)]).is_err());
        }
        assert!(command("wallpaper.set", &[SysValue::Text("relative.jpg".into())]).is_err());
        assert!(command("wallpaper.set", &[]).is_err());
    }

    #[test]
    #[ignore = "requires a static desktop wallpaper; changes and restores each monitor's wallpaper and fill mode"]
    fn live_wallpaper_roundtrip() {
        let _apartment = super::super::windows_system::Apartment::new().unwrap();
        let desktop = desktop().unwrap();
        unsafe {
            assert_eq!(desktop.GetStatus().unwrap().0 & 2, 0, "a slideshow must not be interrupted by this test");
            let previous = WallpaperSnapshot::capture(&desktop).unwrap();
            for (_, path) in &previous.paths {
                assert!(path.is_empty() || Path::new(path).is_file(),
                    "refusing to change a wallpaper whose original image is missing: {path}");
            }
            struct Restore(Option<WallpaperSnapshot>);
            impl Drop for Restore {
                fn drop(&mut self) {
                    if let Some(saved) = &self.0 {
                        if let Err(error) = saved.restore() { eprintln!("wallpaper recovery failed: {error}"); }
                    }
                }
            }
            let mut restore = Restore(Some(previous));
            let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let folder = std::env::temp_dir().join(format!("pleamar wallpaper España 海 {} {unique}", std::process::id()));
            std::fs::create_dir(&folder).unwrap();
            let path = folder.join("roundtrip.png");
            image::RgbImage::from_pixel(64, 36, image::Rgb([24, 44, 38])).save(&path).unwrap();
            // Keep the image on any failure: Explorer may still be using it.
            command("wallpaper.set", &[SysValue::Text(path.to_string_lossy().into_owned())]).unwrap();
            let actual = WallpaperSnapshot::capture(&super::desktop().unwrap()).unwrap();
            assert!(actual.paths.iter().all(|(_, p)| p == &path.to_string_lossy()));
            assert_eq!(actual.position, DWPOS_FILL);
            let saved = restore.0.as_ref().unwrap();
            saved.restore().unwrap();
            // A separate reader after asynchronous shell work has settled
            // catches late commits which same-instance immediate reads missed.
            std::thread::sleep(std::time::Duration::from_secs(1));
            let actual = WallpaperSnapshot::capture(&super::desktop().unwrap()).unwrap();
            assert_eq!(actual.paths, saved.paths);
            assert_eq!(actual.position, saved.position);
            let invalid = folder.join("broken.png");
            std::fs::write(&invalid, b"not an image").unwrap();
            let error = command("wallpaper.set", &[SysValue::Text(invalid.to_string_lossy().into_owned())]).unwrap_err();
            assert!(error.contains("previous wallpaper restored"), "{error}");
            let actual = WallpaperSnapshot::capture(&super::desktop().unwrap()).unwrap();
            assert_eq!(actual.paths, saved.paths);
            assert_eq!(actual.position, saved.position);
            restore.0 = None;
            std::fs::remove_file(path).unwrap();
            std::fs::remove_file(invalid).unwrap();
            std::fs::remove_dir(folder).unwrap();
        }
    }
}
