//! Programs' buffers on the card with more than one plane, and video.
//!
//! A buffer on the card may be more than one piece of memory: an RGB image
//! whose layout keeps compression data beside it (AMD, Intel), or a video
//! frame as a decoder hands it over, NV12 —the light at full size, the colour
//! at half size, apart—. wgpu only imports buffers of one plane, so these
//! are imported here with Vulkan itself, and handed to wgpu as its own
//! textures. NV12 is then painted as RGB (`YuvToRgb`) where it is used.

use crate::scene::{DmabufPiece, DmabufPlane};
use ash::vk;
use std::os::fd::{AsRawFd, IntoRawFd};
use wgpu::hal::api::Vulkan;

/// NV12 as the programs name it (DRM fourcc).
pub const NV12: u32 = u32::from_le_bytes(*b"NV12");
pub const ARGB: u32 = u32::from_le_bytes(*b"AR24");
pub const XRGB: u32 = u32::from_le_bytes(*b"XR24");
/// RGBA in memory (Firefox's and Chromium's tiles).
pub const ABGR: u32 = u32::from_le_bytes(*b"AB24");
pub const XBGR: u32 = u32::from_le_bytes(*b"XB24");

/// Whether the render can take buffers in that format: BGRA, RGBA, or NV12
/// if the card was opened with it.
pub fn readable(device: &wgpu::Device, fourcc: u32) -> bool {
    [ARGB, XRGB, ABGR, XBGR].contains(&fourcc) || (fourcc == NV12 && device.features().contains(wgpu::Features::TEXTURE_FORMAT_NV12))
}

/// Whether a buffer shows no transparency: XRGB, XBGR and video.
pub fn opaque(fourcc: u32) -> bool {
    fourcc == XRGB || fourcc == XBGR || fourcc == NV12
}

/// Whether it is copied as it is into BGRA, or has to be painted there
/// (RGBA, video).
pub fn copied(fourcc: u32) -> bool {
    fourcc == ARGB || fourcc == XRGB
}

/// What a buffer's layout is when nobody says it: only the driver that made
/// it knows (DRM_FORMAT_MOD_INVALID).
pub const NO_LAYOUT: u64 = 0x00ff_ffff_ffff_ffff;

/// Whether this card's buffers go without a layout said: it cannot be told
/// one (no VK_EXT_image_drm_format_modifier, as AMD cards before Vega with
/// RADV), so it offers none to paint in. `PLEAMAR_NO_LAYOUTS=1` asks for it
/// on any card, to try that way.
pub fn without_layouts(device: &wgpu::Device, modifiers: &[u64]) -> bool {
    std::env::var_os("PLEAMAR_NO_LAYOUTS").is_some_and(|v| v == "1") || (modifiers.is_empty() && !device.features().contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF))
}

/// A buffer of BGRA whose layout only its driver knows, one plane, as a
/// texture of this device. The image is made as the driver lays its own out
/// and takes the buffer as memory of its own (dedicated): the driver then
/// reads the real layout from the buffer itself, which is how cards without
/// layouts to name have always shared them.
pub fn import_without_layout(device: &wgpu::Device, fd: std::os::fd::OwnedFd, size: (u32, u32), uses: wgpu::TextureUses, usage: wgpu::TextureUsages, initial: wgpu::TextureUses) -> Result<wgpu::Texture, String> {
    let format = vk::Format::B8G8R8A8_UNORM;
    let extent = wgpu::Extent3d { width: size.0.max(1), height: size.1.max(1), depth_or_array_layers: 1 };
    let hal_desc = wgpu::hal::TextureDescriptor {
        label: Some("a buffer on the card, laid out by its driver"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: uses,
        memory_flags: wgpu::hal::MemoryFlags::empty(),
        view_formats: Vec::new(),
    };
    let mut vk_usage = vk::ImageUsageFlags::empty();
    for (has, flag) in [
        (wgpu::TextureUses::COLOR_TARGET, vk::ImageUsageFlags::COLOR_ATTACHMENT),
        (wgpu::TextureUses::COPY_SRC, vk::ImageUsageFlags::TRANSFER_SRC),
        (wgpu::TextureUses::COPY_DST, vk::ImageUsageFlags::TRANSFER_DST),
        (wgpu::TextureUses::RESOURCE, vk::ImageUsageFlags::SAMPLED),
    ] {
        if uses.contains(has) {
            vk_usage |= flag;
        }
    }
    // SAFETY: the fd is a dmabuf of that size, BGRA, as whoever made it
    // says; the image and its memory are handed to wgpu, which frees them
    // (the callback) once nothing uses them.
    let hal_texture = unsafe {
        let hal = device.as_hal::<Vulkan>().ok_or("the card is not driven with Vulkan")?;
        let wants = [ash::khr::external_memory_fd::NAME, ash::ext::external_memory_dma_buf::NAME];
        if let Some(missing) = wants.iter().find(|w| !hal.enabled_device_extensions().contains(w)) {
            return Err(format!("the card's driver has no {}: it shares no buffers", missing.to_string_lossy()));
        }
        let raw = hal.raw_device();
        let instance = hal.shared_instance().raw_instance();
        // Whether the driver takes such a buffer for those uses at all.
        let mut external_format = vk::PhysicalDeviceExternalImageFormatInfo::default().handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let format_info = vk::PhysicalDeviceImageFormatInfo2::default().format(format).ty(vk::ImageType::TYPE_2D).tiling(vk::ImageTiling::OPTIMAL).usage(vk_usage).push_next(&mut external_format);
        let mut external_props = vk::ExternalImageFormatProperties::default();
        let mut format_props = vk::ImageFormatProperties2::default().push_next(&mut external_props);
        if let Err(e) = instance.get_physical_device_image_format_properties2(hal.raw_physical_device(), &format_info, &mut format_props) {
            return Err(format!("the card's driver takes no buffer without a layout for that: {e}"));
        }
        if !external_props.external_memory_properties.external_memory_features.contains(vk::ExternalMemoryFeatureFlags::IMPORTABLE) {
            return Err("the card's driver does not read buffers without a layout".into());
        }
        let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D { width: extent.width, height: extent.height, depth: 1 })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk_usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .push_next(&mut external);
        let image = raw.create_image(&info, None).map_err(|e| format!("the card made no image for the buffer: {e}"))?;
        let fd_fn = ash::khr::external_memory_fd::Device::new(instance, raw);
        let req = raw.get_image_memory_requirements(image);
        let memory = match import_memory(raw, &fd_fn, fd.into_raw_fd(), req, Some(image)) {
            Ok(m) => m,
            Err(e) => {
                raw.destroy_image(image, None);
                return Err(e);
            }
        };
        if let Err(e) = raw.bind_image_memory(image, memory, 0) {
            raw.free_memory(memory, None);
            raw.destroy_image(image, None);
            return Err(format!("the buffer could not be bound: {e}"));
        }
        let raw = raw.clone();
        let free: wgpu::hal::DropCallback = Box::new(move || {
            raw.destroy_image(image, None);
            raw.free_memory(memory, None);
        });
        hal.texture_from_raw(image, &hal_desc, Some(free), wgpu::hal::vulkan::TextureMemory::External)
    };
    let desc = wgpu::TextureDescriptor { label: Some("a buffer on the card, laid out by its driver"), size: extent, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Bgra8Unorm, usage, view_formats: &[] };
    // SAFETY: made on this device, as `desc` says.
    Ok(unsafe { device.create_texture_from_hal::<Vulkan>(hal_texture, &desc, initial) })
}

/// A program's buffer, all its planes, as a texture of this device: BGRA
/// for ARGB and XRGB, RGBA for ABGR and XBGR, NV12 as it is (paint the last
/// two into BGRA with `YuvToRgb`).
pub fn import(device: &wgpu::Device, d: DmabufPiece, size: (u32, u32), uses: wgpu::TextureUses, usage: wgpu::TextureUsages, initial: wgpu::TextureUses) -> Result<wgpu::Texture, String> {
    let nv12 = d.fourcc == NV12;
    if !readable(device, d.fourcc) {
        return Err(format!("the format {:#x} is not read", d.fourcc));
    }
    // One plane of BGRA: as wgpu does it.
    if copied(d.fourcc) && d.planes.len() == 1 {
        let p = d.planes.into_iter().next().unwrap();
        return crate::gpu::Gpu::import_dmabuf(device, p.fd, size, d.modifier, p.stride, p.offset, uses, usage, initial);
    }
    let (format, vk_format) = if nv12 {
        (wgpu::TextureFormat::NV12, vk::Format::G8_B8R8_2PLANE_420_UNORM)
    } else if d.fourcc == ABGR || d.fourcc == XBGR {
        (wgpu::TextureFormat::Rgba8Unorm, vk::Format::R8G8B8A8_UNORM)
    } else {
        (wgpu::TextureFormat::Bgra8Unorm, vk::Format::B8G8R8A8_UNORM)
    };
    let extent = wgpu::Extent3d { width: size.0.max(1), height: size.1.max(1), depth_or_array_layers: 1 };
    let hal_desc = wgpu::hal::TextureDescriptor {
        label: Some("a buffer on the card, in planes"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: uses,
        memory_flags: wgpu::hal::MemoryFlags::empty(),
        view_formats: Vec::new(),
    };
    // SAFETY: the planes are dmabufs of that size, format and layout, as the
    // program says; the image and its memory are handed to wgpu, which frees
    // them (the callback) once nothing uses them.
    let hal_texture = unsafe {
        let hal = device.as_hal::<Vulkan>().ok_or("the card is not driven with Vulkan")?;
        let (image, memories) = import_planes(&hal, d.planes, vk_format, extent, d.modifier, nv12, uses)?;
        let raw = hal.raw_device().clone();
        let free: wgpu::hal::DropCallback = Box::new(move || {
            raw.destroy_image(image, None);
            for m in memories {
                raw.free_memory(m, None);
            }
        });
        hal.texture_from_raw(image, &hal_desc, Some(free), wgpu::hal::vulkan::TextureMemory::External)
    };
    let desc = wgpu::TextureDescriptor { label: Some("a buffer on the card, in planes"), size: extent, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] };
    // SAFETY: made on this device, as `desc` says.
    Ok(unsafe { device.create_texture_from_hal::<Vulkan>(hal_texture, &desc, initial) })
}

/// The image, with its memory: one piece if all the planes are in the same
/// buffer (the usual), one per plane if not (a «disjoint» image).
unsafe fn import_planes(hal: &wgpu::hal::vulkan::Device, planes: Vec<DmabufPlane>, format: vk::Format, extent: wgpu::Extent3d, modifier: u64, nv12: bool, uses: wgpu::TextureUses) -> Result<(vk::Image, Vec<vk::DeviceMemory>), String> {
    let raw = hal.raw_device();
    let instance = hal.shared_instance().raw_instance();
    let fd_fn = ash::khr::external_memory_fd::Device::new(instance, raw);
    // The same buffer if every plane's fd is the same file.
    let id = |p: &DmabufPlane| {
        // SAFETY: fstat into a zeroed struct of the right type.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        (unsafe { libc::fstat(p.fd.as_raw_fd(), &mut st) } == 0).then_some((st.st_dev, st.st_ino))
    };
    let first = id(&planes[0]);
    let disjoint = first.is_none() || planes.iter().any(|p| id(p) != first);
    let layouts: Vec<vk::SubresourceLayout> = planes.iter().map(|p| vk::SubresourceLayout { offset: p.offset as u64, row_pitch: p.stride as u64, size: 0, array_pitch: 0, depth_pitch: 0 }).collect();
    let mut drm = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default().drm_format_modifier(modifier).plane_layouts(&layouts);
    let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    // NV12 is read one plane at a time: light as R8, colour as RG8.
    let view_formats = [format, vk::Format::R8_UNORM, vk::Format::R8G8_UNORM];
    let mut list = vk::ImageFormatListCreateInfo::default().view_formats(&view_formats);
    let mut flags = vk::ImageCreateFlags::empty();
    if nv12 {
        flags |= vk::ImageCreateFlags::MUTABLE_FORMAT;
    }
    if disjoint {
        flags |= vk::ImageCreateFlags::DISJOINT;
    }
    let mut usage = vk::ImageUsageFlags::empty();
    if uses.contains(wgpu::TextureUses::COPY_SRC) {
        usage |= vk::ImageUsageFlags::TRANSFER_SRC;
    }
    if uses.contains(wgpu::TextureUses::RESOURCE) {
        usage |= vk::ImageUsageFlags::SAMPLED;
    }
    let mut info = vk::ImageCreateInfo::default()
        .flags(flags)
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D { width: extent.width, height: extent.height, depth: 1 })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut external)
        .push_next(&mut drm);
    if nv12 {
        info = info.push_next(&mut list);
    }
    // SAFETY: a valid create info for this device.
    let image = unsafe { raw.create_image(&info, None) }.map_err(|e| format!("the card did not take the buffer's layout: {e}"))?;
    let aspects = [vk::ImageAspectFlags::MEMORY_PLANE_0_EXT, vk::ImageAspectFlags::MEMORY_PLANE_1_EXT, vk::ImageAspectFlags::MEMORY_PLANE_2_EXT, vk::ImageAspectFlags::MEMORY_PLANE_3_EXT];
    let mut memories = Vec::new();
    let fail = |memories: &[vk::DeviceMemory], e: String| {
        // SAFETY: made here and not handed to anyone.
        unsafe {
            for m in memories {
                raw.free_memory(*m, None);
            }
            raw.destroy_image(image, None);
        }
        Err(e)
    };
    if disjoint {
        let mut binds = Vec::new();
        for (k, p) in planes.into_iter().enumerate().take(4) {
            let mut plane = vk::ImagePlaneMemoryRequirementsInfo::default().plane_aspect(aspects[k]);
            let req_info = vk::ImageMemoryRequirementsInfo2::default().image(image).push_next(&mut plane);
            let mut req = vk::MemoryRequirements2::default();
            // SAFETY: the image was just made, disjoint, with that plane.
            unsafe { raw.get_image_memory_requirements2(&req_info, &mut req) };
            match unsafe { import_memory(raw, &fd_fn, p.fd.into_raw_fd(), req.memory_requirements, None) } {
                Ok(m) => {
                    memories.push(m);
                    binds.push((m, aspects[k]));
                }
                Err(e) => return fail(&memories, e),
            }
        }
        let mut plane_infos: Vec<vk::BindImagePlaneMemoryInfo> = binds.iter().map(|(_, a)| vk::BindImagePlaneMemoryInfo::default().plane_aspect(*a)).collect();
        let infos: Vec<vk::BindImageMemoryInfo> = binds.iter().zip(plane_infos.iter_mut()).map(|((m, _), pi)| vk::BindImageMemoryInfo::default().image(image).memory(*m).memory_offset(0).push_next(pi)).collect();
        // SAFETY: each plane's memory, imported for it.
        if let Err(e) = unsafe { raw.bind_image_memory2(&infos) } {
            return fail(&memories, format!("the planes could not be bound: {e}"));
        }
    } else {
        // SAFETY: the image was just made on this device.
        let req = unsafe { raw.get_image_memory_requirements(image) };
        let fd = planes.into_iter().next().unwrap().fd.into_raw_fd();
        match unsafe { import_memory(raw, &fd_fn, fd, req, Some(image)) } {
            Ok(m) => memories.push(m),
            Err(e) => return fail(&memories, e),
        }
        // SAFETY: memory imported for this image.
        if let Err(e) = unsafe { raw.bind_image_memory(image, memories[0], 0) } {
            return fail(&memories, format!("the buffer could not be bound: {e}"));
        }
    }
    Ok((image, memories))
}

/// A dmabuf as memory of the card, for that image (dedicated, if it is not
/// disjoint). The fd is the card's from here on, or closed if it fails.
unsafe fn import_memory(raw: &ash::Device, fd_fn: &ash::khr::external_memory_fd::Device, fd: i32, req: vk::MemoryRequirements, dedicated: Option<vk::Image>) -> Result<vk::DeviceMemory, String> {
    let close = || {
        // SAFETY: the fd is ours until the card takes it.
        unsafe { libc::close(fd) };
    };
    let mut props = vk::MemoryFdPropertiesKHR::default();
    // SAFETY: a dmabuf fd, asked about.
    if let Err(e) = unsafe { fd_fn.get_memory_fd_properties(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT, fd, &mut props) } {
        close();
        return Err(format!("the card does not know the buffer: {e}"));
    }
    let bits = req.memory_type_bits & props.memory_type_bits;
    if bits == 0 {
        close();
        return Err("no kind of the card's memory fits the buffer".into());
    }
    let mut import = vk::ImportMemoryFdInfoKHR::default().handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT).fd(fd);
    let mut one = vk::MemoryDedicatedAllocateInfo::default();
    if let Some(image) = dedicated {
        one = one.image(image);
    }
    let mut info = vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(bits.trailing_zeros()).push_next(&mut import);
    if dedicated.is_some() {
        info = info.push_next(&mut one);
    }
    // SAFETY: a valid allocation that imports the fd.
    unsafe { raw.allocate_memory(&info, None) }.map_err(|e| {
        close();
        format!("the buffer could not be imported: {e}")
    })
}

/// Video (NV12) painted as RGB: light and colour, read apart, to the colours
/// a monitor shows. BT.709 for high definition, BT.601 below, limited range,
/// as decoders hand them over.
pub struct YuvToRgb {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// RGBA painted as it is: into BGRA, which a copy cannot do.
    plain: wgpu::RenderPipeline,
    plain_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

const PLAIN_SHADER: &str = r#"
struct V { @builtin(position) p: vec4<f32>, @location(0) uv: vec2<f32> };
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
@vertex fn vs(@builtin(vertex_index) i: u32) -> V {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return V(vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0), uv);
}
@fragment fn fs(v: V) -> @location(0) vec4<f32> {
    return textureSample(t, s, v.uv);
}
"#;

const YUV_SHADER: &str = r#"
struct V { @builtin(position) p: vec4<f32>, @location(0) uv: vec2<f32> };
@group(0) @binding(0) var light: texture_2d<f32>;
@group(0) @binding(1) var colour: texture_2d<f32>;
@group(0) @binding(2) var s: sampler;
@group(0) @binding(3) var<uniform> hd: vec4<f32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> V {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return V(vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0), uv);
}
@fragment fn fs(v: V) -> @location(0) vec4<f32> {
    let y = (textureSample(light, s, v.uv).r - 16.0 / 255.0) * (255.0 / 219.0);
    let c = (textureSample(colour, s, v.uv).rg - vec2<f32>(128.0 / 255.0)) * (255.0 / 224.0);
    var rgb: vec3<f32>;
    if hd.x > 0.5 {
        rgb = vec3<f32>(y + 1.5748 * c.y, y - 0.1873 * c.x - 0.4681 * c.y, y + 1.8556 * c.x);
    } else {
        rgb = vec3<f32>(y + 1.402 * c.y, y - 0.344136 * c.x - 0.714136 * c.y, y + 1.772 * c.x);
    }
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
"#;

impl YuvToRgb {
    /// Painting into textures of that format.
    pub fn new(device: &wgpu::Device, target: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("video to RGB"), source: wgpu::ShaderSource::Wgsl(YUV_SHADER.into()) });
        let tex = |binding| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("video to RGB"),
            entries: &[
                tex(0),
                tex(1),
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("video to RGB"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("video to RGB"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs"), targets: &[Some(wgpu::ColorTargetState { format: target, blend: None, write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let plain_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("RGBA to BGRA"), source: wgpu::ShaderSource::Wgsl(PLAIN_SHADER.into()) });
        let plain_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RGBA to BGRA"),
            entries: &[tex(0), wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None }],
        });
        let ppl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("RGBA to BGRA"), bind_group_layouts: &[Some(&plain_layout)], immediate_size: 0 });
        let plain = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("RGBA to BGRA"),
            layout: Some(&ppl),
            vertex: wgpu::VertexState { module: &plain_shader, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &plain_shader, entry_point: Some("fs"), targets: &[Some(wgpu::ColorTargetState { format: target, blend: None, write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        // Nearest: pixel for pixel.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Nearest, min_filter: wgpu::FilterMode::Nearest, ..Default::default() });
        YuvToRgb { pipeline, layout, plain, plain_layout, sampler }
    }

    /// The video frame `source` (NV12), painted into the corner of `target`
    /// at its size.
    pub fn convert(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, source: &wgpu::Texture, target: &wgpu::TextureView, size: (u32, u32)) {
        if source.format() != wgpu::TextureFormat::NV12 {
            let view = source.create_view(&Default::default());
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.plain_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) }],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("RGBA to BGRA"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.plain);
            pass.set_bind_group(0, &group, &[]);
            pass.set_viewport(0.0, 0.0, size.0.max(1) as f32, size.1.max(1) as f32, 0.0, 1.0);
            pass.draw(0..3, 0..1);
            return;
        }
        let plane = |format, aspect| source.create_view(&wgpu::TextureViewDescriptor { format: Some(format), aspect, dimension: Some(wgpu::TextureViewDimension::D2), ..Default::default() });
        let light = plane(wgpu::TextureFormat::R8Unorm, wgpu::TextureAspect::Plane0);
        let colour = plane(wgpu::TextureFormat::Rg8Unorm, wgpu::TextureAspect::Plane1);
        let hd: [f32; 4] = [if size.1 >= 720 { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0];
        let buffer = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16, usage: wgpu::BufferUsages::UNIFORM, mapped_at_creation: true });
        if let Ok(mut view) = buffer.slice(..).get_mapped_range_mut() {
            view.copy_from_slice(bytemuck::cast_slice(&hd));
        }
        buffer.unmap();
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&light) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&colour) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: buffer.as_entire_binding() },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("video to RGB"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.set_viewport(0.0, 0.0, size.0.max(1) as f32, size.1.max(1) as f32, 0.0, 1.0);
        pass.draw(0..3, 0..1);
    }
}

/// The modifiers the card can read a format with, for that use (any number of planes).
pub fn modifiers(adapter: &wgpu::Adapter, format: vk::Format, need: vk::FormatFeatureFlags) -> Vec<u64> {
    // SAFETY: only reading properties of the physical device wgpu uses.
    unsafe {
        let Some(hal) = adapter.as_hal::<Vulkan>() else { return Vec::new() };
        let instance = hal.shared_instance().raw_instance();
        let pd = hal.raw_physical_device();
        let mut list = vk::DrmFormatModifierPropertiesListEXT::default();
        let mut fp = vk::FormatProperties2::default().push_next(&mut list);
        instance.get_physical_device_format_properties2(pd, format, &mut fp);
        let n = list.drm_format_modifier_count as usize;
        let mut mods = vec![vk::DrmFormatModifierPropertiesEXT::default(); n];
        let mut list = vk::DrmFormatModifierPropertiesListEXT::default().drm_format_modifier_properties(&mut mods);
        let mut fp = vk::FormatProperties2::default().push_next(&mut list);
        instance.get_physical_device_format_properties2(pd, format, &mut fp);
        mods.iter().filter(|m| m.drm_format_modifier_tiling_features.contains(need)).map(|m| m.drm_format_modifier).collect()
    }
}
