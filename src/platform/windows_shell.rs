//! The shell's actual application catalog and Windows-owned session controls.
use super::SysValue;
use windows::core::{w, PCWSTR, PWSTR, Result};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

unsafe fn take_string(raw: PWSTR) -> Result<String> {
    let value = unsafe { raw.to_string() };
    unsafe { CoTaskMemFree(Some(raw.0 as _)); }
    Ok(value?)
}

pub fn apps() -> Result<SysValue> {
    unsafe {
        let folder: IShellItem = SHCreateItemFromParsingName(w!("shell:AppsFolder"), None)?;
        let items: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems)?;
        let mut rows = Vec::new();
        loop {
            let mut next = [None];
            let mut fetched = 0;
            items.Next(&mut next, Some(&mut fetched))?;
            if fetched == 0 { break; }
            let Some(item) = next[0].take() else { break; };
            let Ok(name) = item.GetDisplayName(SIGDN_NORMALDISPLAY).and_then(|p| take_string(p)) else { continue; };
            let Ok(id) = item.GetDisplayName(SIGDN_PARENTRELATIVEPARSING).and_then(|p| take_string(p)) else { continue; };
            rows.push((name, id));
        }
        rows.sort_by_cached_key(|(name, _)| name.to_lowercase());
        Ok(SysValue::List(rows.into_iter().map(|(name, id)| SysValue::Map(vec![
            ("name".into(), SysValue::Text(name)), ("exec".into(), SysValue::Text(id.clone())),
            ("icon".into(), SysValue::Text(format!("windows-app:{id}"))),
        ])).collect()))
    }
}

pub fn launch(id: &str) -> std::result::Result<(), String> {
    // Only catalog identifiers are accepted, never a shell command line.
    let SysValue::List(apps) = apps().map_err(|e| e.to_string())? else { unreachable!() };
    let known = apps.iter().any(|v| matches!(v, SysValue::Map(row) if row.iter().any(|(k, v)| k == "exec" && *v == SysValue::Text(id.into()))));
    if !known { return Err("the application is not in the Windows catalog".into()); }
    use std::os::windows::process::CommandExt;
    let system = std::env::var_os("SystemRoot").ok_or("SystemRoot is not set")?;
    let mut broker = std::process::Command::new(std::path::PathBuf::from(system).join("explorer.exe"));
    broker.arg(format!("shell:AppsFolder\\{id}"))
        .creation_flags(windows::Win32::System::Threading::CREATE_BREAKAWAY_FROM_JOB.0 | windows::Win32::System::Threading::CREATE_NO_WINDOW.0);
    let mut child = broker.spawn().map_err(|e| format!("could not ask Explorer to launch the application: {e}"))?;
    std::thread::spawn(move || { let _ = child.wait(); });
    Ok(())
}

pub fn open(uri: &str) -> std::result::Result<(), String> {
    let path: Vec<u16> = uri.encode_utf16().chain([0]).collect();
    let result = unsafe { ShellExecuteW(None, w!("open"), PCWSTR(path.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    if result.0 as isize > 32 { Ok(()) } else { Err(format!("Windows could not open {uri} (code {})", result.0 as isize)) }
}

/// The image workshop asks only for icons that a scene actually displays.
/// Extracting the entire AppsFolder catalog during startup is noticeably slow.
pub fn icon(name: &str) -> Option<std::path::PathBuf> {
    use std::hash::{Hash, Hasher};
    use windows::core::Interface;
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::*;
    let path = std::path::Path::new(name);
    if path.is_absolute() { return path.is_file().then(|| path.to_owned()); }
    let (id, source) = if let Some(id) = name.strip_prefix("windows-app:") {
        (id, format!("shell:AppsFolder\\{id}"))
    } else {
        let path = name.strip_prefix("windows-file:")?;
        if !std::path::Path::new(path).is_absolute() { return None; }
        (path, path.to_owned())
    };
    if id.contains('\0') { return None; }
    let mut hash = std::hash::DefaultHasher::new();
    name.hash(&mut hash);
    let cache = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("pleamar/icon-cache");
    let path = cache.join(format!("{:016x}.png", hash.finish()));
    if path.is_file() { return Some(path); }
    let _apartment = super::windows_system::Apartment::new().ok()?;
    let source: Vec<u16> = source.encode_utf16().chain([0]).collect();
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(source.as_ptr()), None).ok()?;
        let factory: IShellItemImageFactory = item.cast().ok()?;
        let bitmap = factory.GetImage(SIZE { cx: 64, cy: 64 }, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK).ok()?;
        struct Bitmap(HBITMAP);
        impl Drop for Bitmap { fn drop(&mut self) { unsafe { let _ = DeleteObject(self.0.into()); } } }
        let _bitmap = Bitmap(bitmap);
        let mut object = BITMAP::default();
        if GetObjectW(bitmap.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut object as *mut _ as _)) == 0 { return None; }
        let (w, h) = (object.bmWidth, object.bmHeight);
        if w <= 0 || h <= 0 || w > 512 || h > 512 { return None; }
        let dc = GetDC(None);
        if dc.is_invalid() { return None; }
        struct Dc(HDC);
        impl Drop for Dc { fn drop(&mut self) { unsafe { ReleaseDC(None, self.0); } } }
        let _dc = Dc(dc);
        let mut info = BITMAPINFO::default();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32,
            biCompression: BI_RGB.0, ..Default::default()
        };
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        if GetDIBits(dc, bitmap, 0, h as u32, Some(rgba.as_mut_ptr() as _), &mut info, DIB_RGB_COLORS) != h { return None; }
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            let alpha = pixel[3] as u32;
            if alpha > 0 { for channel in &mut pixel[..3] { *channel = ((*channel as u32 * 255 + alpha / 2) / alpha).min(255) as u8; } }
        }
        std::fs::create_dir_all(cache).ok()?;
        // A process-specific temporary keeps concurrent runtimes from reading
        // a half-written PNG from the shared cache.
        let temp = path.with_extension(format!("{}.png", std::process::id()));
        image::save_buffer_with_format(&temp, &rgba, w as u32, h as u32, image::ColorType::Rgba8, image::ImageFormat::Png).ok()?;
        if std::fs::rename(&temp, &path).is_err() { let _ = std::fs::remove_file(temp); }
        path.is_file().then_some(path)
    }
}
