//! WGC keeps desktop pixels on the GPU; the video processor converts BGRA to NV12.
use std::mem::ManuallyDrop;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
use windows::{core::{Interface, Result}, Graphics::{Capture::*, DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat}}, Win32::{Foundation::*, Graphics::{Direct3D::*, Direct3D11::*, Dxgi::{IDXGIDevice, Common::*}, Gdi::*}, System::WinRT::{Direct3D11::*, Graphics::Capture::IGraphicsCaptureItemInterop}}};

pub struct Capture {
    pub device: ID3D11Device,
    context: ID3D11DeviceContext,
    video: ID3D11VideoDevice,
    video_context: ID3D11VideoContext1,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    frame: Option<Direct3D11CaptureFrame>,
    pub width: u32,
    pub height: u32,
    source_size: windows::Graphics::SizeInt32,
    closed: Arc<AtomicBool>,
}

fn monitor(name: &str) -> Result<HMONITOR> {
    struct Find<'a> { name: &'a str, found: HMONITOR }
    unsafe extern "system" fn visit(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> windows::core::BOOL { unsafe {
        let find = &mut *(data.0 as *mut Find<'_>);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(m, &mut info as *mut _ as _).as_bool() {
            let len = info.szDevice.iter().position(|&v| v == 0).unwrap_or(info.szDevice.len());
            if String::from_utf16_lossy(&info.szDevice[..len]) == find.name { find.found = m; return false.into(); }
        }
        true.into()
    } }
    unsafe {
        if name.is_empty() {
            let mut point = POINT::default();
            GetCursorPos(&mut point)?;
            return Ok(MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST));
        }
        let mut find = Find { name, found: HMONITOR::default() };
        let _ = EnumDisplayMonitors(None, None, Some(visit), LPARAM(&mut find as *mut _ as isize));
        if find.found.0.is_null() { Err(windows::core::Error::new(E_INVALIDARG, "the recording monitor is disconnected")) } else { Ok(find.found) }
    }
}

impl Capture {
    pub fn new(name: &str) -> Result<Self> { unsafe {
        if !crate::platform::windows_capture_winrt::supported()? { return Err(windows::core::Error::new(E_NOTIMPL, "Windows Graphics Capture is unavailable")); }
        let interop: IGraphicsCaptureItemInterop = windows::core::factory::<GraphicsCaptureItem, _>()?;
        let item: GraphicsCaptureItem = interop.CreateForMonitor(monitor(name)?)?;
        let source_size = item.Size()?;
        // NV12 requires even dimensions. Scale an odd last column/row instead of
        // reading padding outside the captured content.
        let (width, height) = (source_size.Width as u32 & !1, source_size.Height as u32 & !1);
        if width < 2 || height < 2 || width > 8192 || height > 8192 { return Err(E_INVALIDARG.into()); }
        let (mut device, mut context) = (None, None);
        D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut context))?;
        let device = device.ok_or(E_POINTER)?;
        let context = context.ok_or(E_POINTER)?;
        // WGC and the encoder access this same D3D device from their own threads.
        let _ = context.cast::<ID3D11Multithread>()?.SetMultithreadProtected(true);
        let video: ID3D11VideoDevice = device.cast()?;
        let video_context: ID3D11VideoContext1 = context.cast()?;
        let rate = DXGI_RATIONAL { Numerator: 60, Denominator: 1 };
        let enumerator = video.CreateVideoProcessorEnumerator(&D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, InputFrameRate: rate,
            InputWidth: source_size.Width as u32, InputHeight: source_size.Height as u32,
            OutputFrameRate: rate, OutputWidth: width, OutputHeight: height,
            Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
        })?;
        let processor = video.CreateVideoProcessor(&enumerator, 0)?;
        video_context.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
        video_context.VideoProcessorSetStreamColorSpace1(&processor, 0, DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709);
        video_context.VideoProcessorSetOutputColorSpace1(&processor, DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709);
        let runtime: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&device.cast::<IDXGIDevice>()?)?.cast()?;
        let pool = crate::platform::windows_capture_winrt::frame_pool(&runtime, DirectXPixelFormat::B8G8R8A8UIntNormalized, 3, source_size)?;
        let closed = Arc::new(AtomicBool::new(false));
        let flag = closed.clone();
        item.Closed(&windows::Foundation::TypedEventHandler::new(move |_, _| { flag.store(true, Ordering::Release); Ok(()) }))?;
        let session = pool.CreateCaptureSession(&item)?;
        session.StartCapture()?;
        Ok(Self { device, context, video, video_context, enumerator, processor, pool, session, frame: None, width, height, source_size, closed })
    } }

    pub fn update(&mut self) -> Result<bool> {
        if self.closed.load(Ordering::Acquire) { return Err(windows::core::Error::new(E_FAIL, "Windows ended the display capture")); }
        // Drain only the finite pool; capture cannot starve audio or stopping.
        for _ in 0..3 {
            match self.pool.TryGetNextFrame() {
                Ok(frame) => {
                    if frame.ContentSize()? != self.source_size { return Err(windows::core::Error::new(E_FAIL, "the recording monitor changed size; start a new recording")); }
                    if let Some(old) = self.frame.replace(frame) { let _ = old.Close(); }
                }
                // windows-rs represents a successful null interface (empty
                // frame pool) as Error::empty(), whose HRESULT is S_OK.
                Err(e) if e.code() == E_POINTER || e.code() == S_OK => break,
                Err(e) => return Err(e),
            }
        }
        Ok(self.frame.is_some())
    }

    pub fn nv12(&self) -> Result<ID3D11Texture2D> { unsafe {
        let frame = self.frame.as_ref().ok_or(E_POINTER)?;
        let texture: ID3D11Texture2D = frame.Surface()?.cast::<IDirect3DDxgiInterfaceAccess>()?.GetInterface()?;
        let mut input = None;
        self.video.CreateVideoProcessorInputView(&texture, &self.enumerator, &D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D, ..Default::default()
        }, Some(&mut input))?;
        let mut output = None;
        self.device.CreateTexture2D(&D3D11_TEXTURE2D_DESC {
            Width: self.width, Height: self.height, MipLevels: 1, ArraySize: 1,
            Format: DXGI_FORMAT_NV12, SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT, BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32, ..Default::default()
        }, None, Some(&mut output))?;
        let output = output.ok_or(E_POINTER)?;
        let mut view = None;
        self.video.CreateVideoProcessorOutputView(&output, &self.enumerator, &D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D, ..Default::default()
        }, Some(&mut view))?;
        let mut stream = D3D11_VIDEO_PROCESSOR_STREAM { Enable: true.into(), pInputSurface: ManuallyDrop::new(input), ..Default::default() };
        let result = self.video_context.VideoProcessorBlt(&self.processor, &view.ok_or(E_POINTER)?, 0, std::slice::from_ref(&stream));
        ManuallyDrop::drop(&mut stream.pInputSurface);
        result?;
        self.context.Flush();
        Ok(output)
    } }
}
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        if let Some(frame) = self.frame.take() { let _ = frame.Close(); }
        let _ = self.pool.Close();
    }
}
