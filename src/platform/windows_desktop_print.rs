//! Bounded, window-only GDI fallback for HWNDs WGC cannot open. Asking the
//! application to paint can hang, so its DC and bitmap live in a disposable child.
use super::*;
use std::{io::Read, mem::size_of, os::windows::process::CommandExt, process::{Child, Command, Stdio}};
use windows::Win32::{Graphics::{Dwm::*, Gdi::*}, Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS}, UI::{HiDpi::*, WindowsAndMessaging::*}};

const FLAG: &str = "--internal-window-print";
const LIMIT: u64 = 5 * 1024 * 1024;
const MARKER: [u8; 4] = [173, 83, 197, 127];

struct Process(Child);
impl Drop for Process { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }
struct Buffer { dc: HDC, bitmap: HBITMAP, previous: HGDIOBJ, pixels: *mut u8 }
impl Drop for Buffer { fn drop(&mut self) { unsafe {
    SelectObject(self.dc, self.previous);
    let _ = DeleteObject(self.bitmap.into());
    let _ = DeleteDC(self.dc);
} } }

fn rectangle(values: &[i64]) -> Result<RECT, String> {
    let values: Vec<i32> = values.iter().map(|v| i32::try_from(*v)).collect::<Result<_, _>>().map_err(|_| "invalid capture coordinates")?;
    let [left, top, right, bottom] = values.as_slice() else { return Err("expected four capture coordinates".into()); };
    let (width, height) = (i64::from(*right) - i64::from(*left), i64::from(*bottom) - i64::from(*top));
    if width < 1 || height < 1 || width > 8192 || height > 8192 || width * height > 16_777_216 {
        return Err("window capture dimensions exceed the 16-megapixel limit".into());
    }
    Ok(RECT { left: *left, top: *top, right: *right, bottom: *bottom })
}
fn identity(hwnd: HWND) -> (u32, u32) {
    let mut process = 0;
    let thread = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process)) };
    (process, thread)
}
fn validate(hwnd: HWND, expected: (u32, u32), frame: RECT, outer: RECT) -> Result<(), String> { unsafe {
    if expected.0 == 0 || expected.1 == 0 || identity(hwnd) != expected || !IsWindowVisible(hwnd).as_bool()
        || IsIconic(hwnd).as_bool() || GetAncestor(hwnd, GA_ROOT) != hwnd {
        return Err("the capture window is no longer a visible top-level window".into());
    }
    let mut affinity = 0;
    GetWindowDisplayAffinity(hwnd, &mut affinity).map_err(|e| format!("cannot determine window capture protection: {e}"))?;
    if affinity != WDA_NONE.0 { return Err("the window excludes capture".into()); }
    let mut actual = RECT::default();
    GetWindowRect(hwnd, &mut actual).map_err(|e| e.to_string())?;
    if actual != outer || crate::platform::windows_desktop::bounds(hwnd)? != frame {
        return Err("the window frame changed during capture; look again".into());
    }
    let mut cloaked = 0u32;
    DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as _, size_of::<u32>() as u32).map_err(|e| e.to_string())?;
    if cloaked != 0 { return Err("the capture window is cloaked".into()); }
    Ok(())
} }

pub(super) fn window(hwnd: HWND, frame: RECT) -> Result<SysValue, String> {
    let (process, thread) = identity(hwnd);
    let mut outer = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut outer) }.map_err(|e| e.to_string())?;
    validate(hwnd, (process, thread), frame, outer)?;
    let mut args = vec![FLAG.to_owned(), (hwnd.0 as usize).to_string(), process.to_string(), thread.to_string()];
    args.extend([frame.left, frame.top, frame.right, frame.bottom, outer.left, outer.top, outer.right, outer.bottom].map(|v| v.to_string()));
    let child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?).args(args)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .creation_flags(0x08000000 | 0x00004000).spawn().map_err(|e| e.to_string())?;
    let mut child = Process(child);
    let stdout = child.0.stdout.take().ok_or("capture helper has no output")?;
    let stderr = child.0.stderr.take().ok_or("capture helper has no diagnostic pipe")?;
    let image = std::thread::spawn(move || { let mut bytes = Vec::new(); stdout.take(LIMIT + 1).read_to_end(&mut bytes).map(|_| bytes) });
    let diagnostic = std::thread::spawn(move || { let mut bytes = Vec::new(); stderr.take(4097).read_to_end(&mut bytes).map(|_| bytes) });
    let started = Instant::now();
    let result = loop {
        if let Err(error) = check_active() { break Err(error); }
        match child.0.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(error) => break Err(error.to_string()),
            _ => {}
        }
        if started.elapsed() >= Duration::from_secs(3) { break Err("window print capture timed out".into()); }
        std::thread::sleep(Duration::from_millis(10));
    };
    // Always retire the child before joining its bounded pipe readers, even on
    // cancellation. No abandoned print worker or GDI allocation remains resident.
    drop(child);
    let bytes = image.join().map_err(|_| "capture output reader failed")?.map_err(|e| e.to_string())?;
    let diagnostic = diagnostic.join().map_err(|_| "capture diagnostic reader failed")?.map_err(|e| e.to_string())?;
    let status = result?;
    if !status.success() { return Err(String::from_utf8_lossy(&diagnostic).trim().chars().take(1024).collect()); }
    if bytes.len() as u64 > LIMIT { return Err("window image exceeds the transport limit".into()); }
    let dimensions = image::ImageReader::with_format(std::io::Cursor::new(&bytes), image::ImageFormat::Png)
        .into_dimensions().map_err(|e| e.to_string())?;
    if dimensions != ((frame.right - frame.left) as u32, (frame.bottom - frame.top) as u32) {
        return Err("printed image dimensions do not match the target".into());
    }
    check_active()?;
    validate(hwnd, (process, thread), frame, outer)?;
    Ok(SysValue::Map(vec![("width".into(), SysValue::Num(dimensions.0 as f64)),
        ("height".into(), SysValue::Num(dimensions.1 as f64)), ("method".into(), SysValue::Text("window-print".into())),
        ("data".into(), SysValue::Text(STANDARD.encode(bytes)))]))
}

fn paint(args: &[String]) -> Result<Vec<u8>, String> { unsafe {
    if args.len() != 11 { return Err("invalid window print request".into()); }
    let hwnd = HWND(args[0].parse::<usize>().map_err(|_| "invalid capture handle")? as _);
    let expected = (args[1].parse::<u32>().map_err(|_| "invalid capture PID")?, args[2].parse::<u32>().map_err(|_| "invalid capture thread")?);
    let coordinates = args[3..].iter().map(|v| v.parse::<i64>()).collect::<Result<Vec<_>, _>>().map_err(|_| "invalid capture coordinates")?;
    let (frame, outer) = (rectangle(&coordinates[..4])?, rectangle(&coordinates[4..])?);
    if SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).is_invalid() {
        return Err("could not enable physical capture coordinates".into());
    }
    validate(hwnd, expected, frame, outer)?;
    let (width, height) = (outer.right - outer.left, outer.bottom - outer.top);
    let crop = capture_crop(frame, outer, width, height).ok_or("invalid window print crop")?;
    let dc = CreateCompatibleDC(None);
    if dc.is_invalid() { return Err("could not create capture DC".into()); }
    let mut info = BITMAPINFO::default();
    info.bmiHeader = BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: width,
        biHeight: -height, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() };
    let mut pixels = std::ptr::null_mut();
    let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut pixels, None, 0) {
        Ok(bitmap) => bitmap,
        Err(error) => { let _ = DeleteDC(dc); return Err(error.to_string()); }
    };
    let previous = SelectObject(dc, bitmap.into());
    if previous.is_invalid() || pixels.is_null() {
        let _ = DeleteObject(bitmap.into()); let _ = DeleteDC(dc);
        return Err("could not select capture bitmap".into());
    }
    let buffer = Buffer { dc, bitmap, previous, pixels: pixels.cast() };
    let pixels = std::slice::from_raw_parts_mut(buffer.pixels, width as usize * height as usize * 4);
    for pixel in pixels.chunks_exact_mut(4) { pixel.copy_from_slice(&MARKER); }
    // Windows must arrange drawing across processes. A raw WM_PRINT leaves
    // the application unable to use this helper's memory DC.
    if !PrintWindow(hwnd, dc, PRINT_WINDOW_FLAGS(0)).as_bool() {
        return Err("Windows rejected the window print request".into());
    }
    if !GdiFlush().as_bool() { return Err("window print pixels did not complete".into()); }
    validate(hwnd, expected, frame, outer)?;
    let mut rgba = Vec::with_capacity(crop.width as usize * crop.height as usize * 4);
    for y in 0..crop.height as usize {
        let start = ((y + crop.y) * width as usize + crop.x) * 4;
        for (x, pixel) in pixels[start..start + crop.width as usize * 4].chunks_exact(4).enumerate() {
            if pixel == MARKER { return Err(format!("the application did not paint the entire window (first missing pixel {x},{y}); print capture is unavailable")); }
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
        }
    }
    let mut png = BoundedPng(Vec::new());
    use image::ImageEncoder;
    image::codecs::png::PngEncoder::new(&mut png).write_image(&rgba, crop.width, crop.height, image::ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    Ok(png.0)
} }

pub(crate) fn helper(args: &[String]) -> Option<i32> {
    if args.first().map(String::as_str) != Some(FLAG) { return None; }
    Some(match paint(&args[1..]) {
        Ok(png) => if std::io::stdout().lock().write_all(&png).is_ok() { 0 } else { 1 },
        Err(error) => { eprintln!("{error}"); 1 }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn print_dimensions_are_bounded_before_allocating() {
        assert!(rectangle(&[-1900, -20, -1200, 800]).is_ok());
        for rect in [[0, 0, 0, 100], [0, 0, 8193, 1], [0, 0, 8192, 8192], [i64::MIN, 0, 50, 50], [0, 0, i64::MAX, 50]] {
            assert!(rectangle(&rect).is_err());
        }
    }
    #[test] fn unrelated_or_malformed_arguments_never_capture() {
        assert_eq!(helper(&["--version".into()]), None);
        assert_eq!(helper(&[FLAG.into()]), Some(1));
        assert!(paint(&std::array::from_fn::<_, 11, _>(|_| "0".into())).is_err());
    }
}
