//! Keep WGC factories inside the calling apartment. The generated static
//! cache can retain a factory after the last capture worker uninitializes COM;
//! starting a second capture then dereferences the retired factory on Windows.
use windows::{core::{Interface, Result}, Graphics::{Capture::*, DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat}, SizeInt32}};

fn keep_capture_code(factory: &IGraphicsCaptureSessionStatics) -> Result<()> {
    use std::sync::OnceLock;
    use windows::{core::{HRESULT, PCWSTR}, Win32::{Foundation::HMODULE, System::LibraryLoader::*}};
    static PINNED: OnceLock<std::result::Result<(), HRESULT>> = OnceLock::new();
    // Windows 11 can return from Close while internal WGC callbacks still use
    // GraphicsCapture.dll. Retiring the last MTA worker then unloads their code
    // (0xc0000005 in GraphicsCapture.dll_unloaded). Keep only that loaded module
    // until process exit, not its factories, COM apartments or GPU resources.
    // Resolve by the activated factory's code address, never a DLL search path.
    let result = PINNED.get_or_init(|| unsafe {
        let mut module = HMODULE::default();
        GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
            PCWSTR(factory.vtable().IsSupported as *const () as *const u16), &mut module).map_err(|e| e.code())
    });
    (*result).map_err(Into::into)
}

pub(super) fn supported() -> Result<bool> { unsafe {
    let factory = windows::core::factory::<GraphicsCaptureSession, IGraphicsCaptureSessionStatics>()?;
    keep_capture_code(&factory)?;
    let mut value = false;
    (factory.vtable().IsSupported)(factory.as_raw(), &mut value).map(|| value)
} }

pub(super) fn frame_pool(device: &IDirect3DDevice, format: DirectXPixelFormat, buffers: i32, size: SizeInt32) -> Result<Direct3D11CaptureFramePool> { unsafe {
    let factory = windows::core::factory::<Direct3D11CaptureFramePool, IDirect3D11CaptureFramePoolStatics2>()?;
    let mut raw = std::ptr::null_mut();
    (factory.vtable().CreateFreeThreaded)(factory.as_raw(), device.as_raw(), format, buffers, size, &mut raw).ok()?;
    if raw.is_null() { return Err(windows::Win32::Foundation::E_POINTER.into()); }
    Ok(Direct3D11CaptureFramePool::from_raw(raw))
} }
