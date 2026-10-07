//! Capture one HWND, never a rectangle of the user's other applications.
use super::{SysValue, check_active};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{io::Write, time::{Duration, Instant}};
use windows::{core::Interface, Graphics::{Capture::*, DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat}}, Win32::{Foundation::*, Graphics::{Direct3D::*, Direct3D11::*, Dxgi::{IDXGIDevice, Common::*}}, System::WinRT::{Direct3D11::*, Graphics::Capture::IGraphicsCaptureItemInterop}}};

#[path = "windows_desktop_print.rs"]
mod print;
pub(crate) use print::helper;

struct Session { pool: Direct3D11CaptureFramePool, session: GraphicsCaptureSession }
impl Drop for Session { fn drop(&mut self) { let _ = self.session.Close(); let _ = self.pool.Close(); } }
struct Frame(Direct3D11CaptureFrame);
impl Drop for Frame { fn drop(&mut self) { let _ = self.0.Close(); } }
struct Mapped<'a>(&'a ID3D11DeviceContext, &'a ID3D11Texture2D);
impl Drop for Mapped<'_> { fn drop(&mut self) { unsafe { self.0.Unmap(self.1, 0); } } }
#[derive(Debug, PartialEq, Eq)]
struct Crop { x: usize, y: usize, width: u32, height: u32 }
fn capture_crop(frame: RECT, outer: RECT, width: i32, height: i32) -> Option<Crop> {
    let fw = i64::from(frame.right) - i64::from(frame.left);
    let fh = i64::from(frame.bottom) - i64::from(frame.top);
    if width < 1 || height < 1 || fw < 1 || fh < 1
        || frame.left < outer.left || frame.top < outer.top || frame.right > outer.right || frame.bottom > outer.bottom { return None; }
    let (x, y) = if fw == i64::from(width) && fh == i64::from(height) { (0, 0) }
        else if i64::from(outer.right) - i64::from(outer.left) == i64::from(width)
            && i64::from(outer.bottom) - i64::from(outer.top) == i64::from(height) {
            ((i64::from(frame.left) - i64::from(outer.left)) as usize,
             (i64::from(frame.top) - i64::from(outer.top)) as usize)
        } else { return None; };
    Some(Crop { x, y, width: fw as u32, height: fh as u32 })
}
fn content_crop(frame: RECT, outer: RECT, pool: (i32, i32), content: (i32, i32)) -> Option<Crop> {
    // ContentSize describes the pixels in this frame, while the pool can retain
    // its original, larger allocation. Never read that undefined padding.
    if content.0 > pool.0 || content.1 > pool.1 { return None; }
    capture_crop(frame, outer, content.0, content.1)
}
struct BoundedPng(Vec<u8>);
impl Write for BoundedPng {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > 5 * 1024 * 1024 {
            return Err(std::io::Error::other("window image exceeds the agent's transport limit"));
        }
        self.0.extend_from_slice(bytes); Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}
pub(super) fn window(hwnd: HWND, bounds: RECT) -> Result<SysValue, String> {
    let _apartment = crate::platform::windows_system::Apartment::new()?;
    capture(hwnd, bounds).map_err(|e| e.to_string())
}
fn capture(hwnd: HWND, bounds: RECT) -> windows::core::Result<SysValue> { unsafe {
    let failure = |message: &str| windows::core::Error::new(E_FAIL, message);
    let mut affinity = 0;
    if windows::Win32::UI::WindowsAndMessaging::GetWindowDisplayAffinity(hwnd, &mut affinity).is_ok() && affinity != 0 {
        return Err(failure("the window excludes capture"));
    }
    if !crate::platform::windows_capture_winrt::supported()? { return Err(failure("Windows Graphics Capture is unavailable")); }
    let interop: IGraphicsCaptureItemInterop = windows::core::factory::<GraphicsCaptureItem, _>()?;
    let item: GraphicsCaptureItem = match interop.CreateForWindow(hwnd) {
        Ok(item) => item,
        Err(error) if error.code() == E_INVALIDARG => return print::window(hwnd, bounds)
            .map_err(|message| failure(&format!("Windows Graphics Capture rejected this window (CreateForWindow: {error}); native print capture: {message}"))),
        Err(error) => return Err(windows::core::Error::new(error.code(), format!("Windows Graphics Capture cannot capture this window (CreateForWindow): {error}"))),
    };
    let size = item.Size()?;
    if size.Width < 1 || size.Height < 1 || size.Width > 8192 || size.Height > 8192 || i64::from(size.Width) * i64::from(size.Height) > 16_777_216 {
        return Err(failure("window capture dimensions exceed the 16-megapixel limit"));
    }
    // WGC can include GetWindowRect's invisible resize borders. Crop only that
    // exact enclosing rectangle so every output pixel still maps to DWM's frame.
    // Unknown dimensions must never be scaled into input coordinates.
    let mut outer = RECT::default();
    windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut outer)?;
    capture_crop(bounds, outer, size.Width, size.Height)
        .ok_or_else(|| failure("capture and window frame coordinates differ; look again after the DPI or size change"))?;
    let (mut device, mut context) = (None, None);
    D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        None, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut context))?;
    let device = device.ok_or(E_POINTER)?;
    let context = context.ok_or(E_POINTER)?;
    let _ = context.cast::<ID3D11Multithread>()?.SetMultithreadProtected(true);
    let runtime: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&device.cast::<IDXGIDevice>()?)?.cast()?;
    let pool = crate::platform::windows_capture_winrt::frame_pool(&runtime, DirectXPixelFormat::B8G8R8A8UIntNormalized, 1, size)?;
    let session = match pool.CreateCaptureSession(&item) { Ok(s) => s, Err(e) => { let _ = pool.Close(); return Err(e); } };
    let session = Session { pool, session };
    session.session.SetIsCursorCaptureEnabled(false)?;
    session.session.StartCapture()?;
    let start = Instant::now();
    let frame = loop {
        check_active().map_err(|e| failure(&e))?;
        if start.elapsed() > Duration::from_secs(3) { return Err(failure("the application did not provide a capture frame")); }
        match session.pool.TryGetNextFrame() {
            Ok(frame) => break Frame(frame),
            Err(e) if e.code() == E_POINTER || e.code() == S_OK => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(e),
        }
    };
    let content = frame.0.ContentSize()?;
    let crop = content_crop(bounds, outer, (size.Width, size.Height), (content.Width, content.Height))
        .ok_or_else(|| failure(&format!("capture content does not match the window frame; look again (pool {}x{}, content {}x{})",
            size.Width, size.Height, content.Width, content.Height)))?;
    let mut current_outer = RECT::default();
    windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut current_outer)?;
    if current_outer != outer { return Err(failure("the window frame changed during capture; look again")); }
    let source: ID3D11Texture2D = frame.0.Surface()?.cast::<IDirect3DDxgiInterfaceAccess>()?.GetInterface()?;
    let mut description = D3D11_TEXTURE2D_DESC::default();
    source.GetDesc(&mut description);
    if description.Width != size.Width as u32 || description.Height != size.Height as u32
        || description.Format != DXGI_FORMAT_B8G8R8A8_UNORM || description.SampleDesc.Count != 1
        || description.MipLevels != 1 || description.ArraySize != 1 {
        return Err(failure("unexpected window capture surface description"));
    }
    let mut target = None;
    device.CreateTexture2D(&D3D11_TEXTURE2D_DESC { Width: size.Width as u32, Height: size.Height as u32, MipLevels: 1, ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM, SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 }, Usage: D3D11_USAGE_STAGING,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32, ..Default::default() }, None, Some(&mut target))?;
    let target = target.ok_or(E_POINTER)?;
    context.CopyResource(&target, &source);
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    loop {
        check_active().map_err(|e| failure(&e))?;
        match context.Map(&target, 0, D3D11_MAP_READ, D3D11_MAP_FLAG_DO_NOT_WAIT.0 as u32, Some(&mut mapped)) {
            Ok(()) => break,
            Err(e) if e.code() == windows::Win32::Graphics::Dxgi::DXGI_ERROR_WAS_STILL_DRAWING && start.elapsed() < Duration::from_secs(5) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => return Err(e),
        }
    }
    let mapping = Mapped(&context, &target);
    let stride = crop.width as usize * 4;
    if mapped.pData.is_null() || mapped.RowPitch < size.Width as u32 * 4 { return Err(failure("invalid window capture row pitch")); }
    let mut rgba = Vec::with_capacity(stride * crop.height as usize);
    for y in 0..crop.height as usize {
        let row = std::slice::from_raw_parts(mapped.pData.cast::<u8>().add((y + crop.y) * mapped.RowPitch as usize + crop.x * 4), stride);
        for bgra in row.chunks_exact(4) { rgba.extend_from_slice(&[bgra[2], bgra[1], bgra[0], bgra[3]]); }
    }
    drop(mapping);
    drop(frame);
    drop(session);
    check_active().map_err(|e| failure(&e))?;
    use image::ImageEncoder;
    let mut png = BoundedPng(Vec::new());
    image::codecs::png::PngEncoder::new(&mut png).write_image(&rgba, crop.width, crop.height, image::ExtendedColorType::Rgba8).map_err(|e| failure(&e.to_string()))?;
    drop(rgba);
    Ok(SysValue::Map(vec![
        ("width".into(), SysValue::Num(crop.width as f64)), ("height".into(), SysValue::Num(crop.height as f64)),
        ("method".into(), SysValue::Text("windows-graphics-capture".into())),
        ("data".into(), SysValue::Text(STANDARD.encode(png.0))),
    ]))
} }

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn ordinary_window_borders_are_cropped_without_scaling() {
        // Actual Windows Server 2022 WGC/DWM geometry from the owned input CI.
        let outer = RECT { left: 80, top: 90, right: 640, bottom: 510 };
        let frame = RECT { left: 87, top: 90, right: 633, bottom: 503 };
        assert_eq!(capture_crop(frame, outer, 560, 420), Some(Crop { x: 7, y: 0, width: 546, height: 413 }));
        assert_eq!(capture_crop(frame, outer, 546, 413), Some(Crop { x: 0, y: 0, width: 546, height: 413 }));
        for size in [(559, 420), (560, 419), (1120, 840), (0, 0)] {
            assert_eq!(capture_crop(frame, outer, size.0, size.1), None);
        }
    }
    #[test] fn negative_monitor_origins_and_asymmetric_dpi_borders_keep_pixel_coordinates() {
        let outer = RECT { left: -1930, top: -20, right: -900, bottom: 810 };
        let frame = RECT { left: -1920, top: -18, right: -910, bottom: 800 };
        let crop = capture_crop(frame, outer, 1030, 830).unwrap();
        assert_eq!(crop, Crop { x: 10, y: 2, width: 1010, height: 818 });
        assert_eq!(outer.left + crop.x as i32 + 20, frame.left + 20);
        assert_eq!(outer.top + crop.y as i32 + 40, frame.top + 40);
        assert_eq!(capture_crop(outer, frame, 1030, 830), None);
    }
    #[test] fn invalid_or_overflowing_rectangles_never_make_a_crop() {
        let wide = RECT { left: i32::MIN, top: 0, right: i32::MAX, bottom: 100 };
        assert_eq!(capture_crop(wide, wide, 640, 100), None);
        let empty = RECT::default();
        assert_eq!(capture_crop(empty, wide, 640, 100), None);
    }
    #[test] fn content_can_exclude_borders_while_the_pool_keeps_its_initial_size() {
        let outer = RECT { left: 80, top: 90, right: 640, bottom: 510 };
        let frame = RECT { left: 87, top: 90, right: 633, bottom: 503 };
        assert_eq!(content_crop(frame, outer, (560, 420), (546, 413)),
            Some(Crop { x: 0, y: 0, width: 546, height: 413 }));
        assert_eq!(content_crop(frame, outer, (560, 420), (560, 420)),
            Some(Crop { x: 7, y: 0, width: 546, height: 413 }));
        // A larger content size would be clipped by the allocation, even if it
        // otherwise matches the enclosing window. A smaller unknown image is
        // not stretched to make an input coordinate map.
        assert_eq!(content_crop(frame, outer, (546, 413), (560, 420)), None);
        assert_eq!(content_crop(frame, outer, (560, 420), (545, 413)), None);
        assert_eq!(content_crop(frame, outer, (560, 420), (0, 0)), None);
    }
}
