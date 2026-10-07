//! Same-adapter D3D11 capture images consumed by the D3D12 renderer.
use std::{os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle}, sync::{Arc, OnceLock}};
use windows::{core::{Interface, Result}, Win32::{Foundation::*, Graphics::{Direct3D11::*, Direct3D12::*, Dxgi::Common::*}}};

#[derive(Clone, Debug)]
pub struct SharedDevice(ID3D12Device, wgpu::Device);

impl SharedDevice {
    pub fn from_wgpu(device: &wgpu::Device) -> Option<Self> {
        unsafe { device.as_hal::<wgpu::hal::api::Dx12>().map(|d| Self(d.raw_device().clone(), device.clone())) }
    }

    pub fn adapter(&self) -> LUID { unsafe { self.0.GetAdapterLuid() } }

    pub fn texture(&self, size: (u32, u32)) -> Result<Arc<SharedTexture>> { unsafe {
        if size.0 == 0 || size.1 == 0 || size.0 > 8192 || size.1 > 8192
            || size.0 as u64 * size.1 as u64 > 16_777_216 { return Err(E_INVALIDARG.into()); }
        let mut resource: Option<ID3D12Resource> = None;
        self.0.CreateCommittedResource(&D3D12_HEAP_PROPERTIES {
            Type:D3D12_HEAP_TYPE_DEFAULT, CreationNodeMask:1, VisibleNodeMask:1, ..Default::default()
        }, D3D12_HEAP_FLAG_SHARED, &D3D12_RESOURCE_DESC {
            Dimension:D3D12_RESOURCE_DIMENSION_TEXTURE2D, Width:size.0 as u64, Height:size.1,
            DepthOrArraySize:1, MipLevels:1, Format:DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc:DXGI_SAMPLE_DESC { Count:1, Quality:0 },
            // D3D11's shared BGRA contract needs shader-resource and render-target
            // binding even when this producer only copies into the image.
            Flags:D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS | D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET, ..Default::default()
        }, D3D12_RESOURCE_STATE_COMMON, None, &mut resource)?;
        let resource = resource.ok_or(E_POINTER)?;
        // Unnamed, non-inheritable, same-process handle. RAII closes it on every path.
        let handle = self.0.CreateSharedHandle(&resource, None, GENERIC_ALL.0, None)?;
        Ok(Arc::new(SharedTexture { resource, device:self.0.clone(), owner:self.1.clone(),
            handle:OwnedHandle::from_raw_handle(handle.0), size, imported:OnceLock::new() }))
    } }
}

#[derive(Debug)]
pub struct SharedTexture {
    resource: ID3D12Resource,
    device: ID3D12Device,
    owner: wgpu::Device,
    handle: OwnedHandle,
    size: (u32, u32),
    imported: OnceLock<wgpu::Texture>,
}
impl SharedTexture {
    pub fn size(&self) -> (u32, u32) { self.size }

    /// Open the producer's view on the same adapter.
    ///
    /// # Safety
    /// Write only while the producer holds the sole strong Arc. Complete all
    /// producer GPU writes before publishing an Arc, and keep published pixels
    /// immutable until every consumer (including GPU submissions) releases it.
    pub unsafe fn open(&self, device: &ID3D11Device) -> Result<ID3D11Texture2D> {
        let dxgi: windows::Win32::Graphics::Dxgi::IDXGIDevice = device.cast()?;
        let adapter = unsafe { dxgi.GetAdapter()?.GetDesc()? }.AdapterLuid;
        let own = unsafe { self.device.GetAdapterLuid() };
        if adapter.LowPart != own.LowPart || adapter.HighPart != own.HighPart { return Err(E_INVALIDARG.into()); }
        unsafe { device.cast::<ID3D11Device1>()?.OpenSharedResource1(HANDLE(self.handle.as_raw_handle())) }
    }
}

impl SharedTexture {
    pub(crate) fn import(image: &Arc<Self>, device: &wgpu::Device) -> Result<wgpu::Texture> {
        // D3D12 may return the same native device to separate wgpu devices. Its
        // COM pointer alone cannot identify the owner of a cached wgpu texture.
        if device != &image.owner { return Err(E_INVALIDARG.into()); }
        // The image owns its import: retirement needs no future repaint to sweep
        // a cache. This also prevents an idle scene retaining closed windows.
        Ok(image.imported.get_or_init(|| {
        let size = wgpu::Extent3d { width:image.size.0, height:image.size.1, depth_or_array_layers:1 };
        // Factory controls device, size, format and flags. A simultaneous-access
        // COPY_SOURCE image decays to COMMON after each submission and promotes
        // on the next copy. External writes finish before its Arc is published.
        let raw = unsafe { wgpu::hal::dx12::Device::texture_from_raw(image.resource.clone(),
            wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureDimension::D2, size, 1, 1) };
        unsafe { device.create_texture_from_hal::<wgpu::hal::api::Dx12>(raw,
            &wgpu::TextureDescriptor { label:Some("native window capture"), size, mip_level_count:1,
                sample_count:1, dimension:wgpu::TextureDimension::D2, format:wgpu::TextureFormat::Bgra8Unorm,
                usage:wgpu::TextureUsages::COPY_SRC, view_formats:&[] }, wgpu::TextureUses::PRESENT) }
        }).clone())
    }
}

#[cfg(test)]
#[path = "windows_texture_tests.rs"]
mod tests;
