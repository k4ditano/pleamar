//! Pixel checks for the shared compositing texture; no desktop capture or input.
use super::*;
use std::sync::{Arc, Mutex};

struct Canvas(wgpu::Texture);
impl Frames for Canvas {
    fn acquire(&mut self, _: &wgpu::Device, _: &[u64]) -> Option<(usize, wgpu::Texture)> { Some((0, self.0.clone())) }
    fn present(&mut self, _: usize, _: wgpu::SubmissionIndex, _: &wgpu::Device, _: &wgpu::Queue, _: Option<[u32; 4]>) {}
}
struct NoWindow;
impl PlatformWindow for NoWindow {
    fn update_input_region(&self, _: &[[i32; 4]]) {}
    fn cursor(&self, _: Cursor) {}
    fn keyboard(&self, _: Keyboard) {}
}

fn read(gpu: &Gpu, texture: &wgpu::Texture, size: (u32, u32)) -> Vec<u8> {
    let row = (size.0 * 4).div_ceil(256) * 256;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("effect pixels"),
        size: (row * size.1) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None } },
        wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 });
    gpu.queue.submit(Some(encoder.finish()));
    let result = Arc::new(Mutex::new(None));
    let done = result.clone();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| *done.lock().unwrap() = Some(r));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while result.lock().unwrap().is_none() && std::time::Instant::now() < deadline {
        let _ = gpu.device.poll(wgpu::PollType::Poll);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    result.lock().unwrap().take().expect("GPU readback timed out").unwrap();
    let pixels = buffer.slice(..).get_mapped_range().unwrap().chunks_exact(row as usize)
        .flat_map(|r| r[..size.0 as usize * 4].iter().copied()).collect();
    buffer.unmap();
    pixels
}

#[test]
#[ignore = "requires a real GPU; PLEAMAR_EFFECT_IMAGE optionally exports the fixture"]
fn many_groups_keep_pixels_and_one_texture_across_close_reopen() {
    let instance = crate::platform::graphics_instance(wgpu::InstanceFlags::VALIDATION);
    let mut gpu = Gpu::new(&instance, None);
    let size = (640u32, 320u32);
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("owned effect fixture"),
        size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 }, mip_level_count: 1,
        sample_count: 1, dimension: wgpu::TextureDimension::D2, format: gpu.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[] });
    let mut sheet = gpu.sheet(NewSheet { id: 0, target: Target::Frames(Box::new(Canvas(texture.clone()))),
        window: Box::new(NoWindow), scale: 1.0, size, mhz: 60_000, name: "owned canvas".into(),
        view: View { surface: 0, popup: None, origin: (0.0, 0.0), size: (size.0 as f32, size.1 as f32) } },
        (size.0 as f32, size.1 as f32));
    let (tx, _rx) = std::sync::mpsc::channel();
    let mut text = Texts::open(tx);
    let mut instrs = Vec::new();
    for _ in 0..128 { instrs.extend([tests::effect_group(), Instr::Opacity(None)]); }
    for k in 0..128 {
        instrs.push(if k % 2 == 0 { tests::effect_group() } else { Instr::Opacity(Some(0.5.into())) });
        for (dx, color) in [(-3.0, [1.0, 0.2, 0.0]), (3.0, [0.0, 0.5, 1.0])] {
            instrs.push(Instr::Solid { shape: Shape::circle((((k % 16) as f32 * 40.0 + 20.0 + dx).into(), ((k / 16) as f32 * 40.0 + 20.0).into()), 10.0),
                color: color.map(Expr::from), alpha: 1.0.into(), glass_spec: None });
        }
        instrs.push(Instr::Opacity(None));
    }
    let mut draw = DrawList::default();
    draw.compose(&instrs, Ctx { props: &[], facts: &[] }, &[], &mut text, None, (640.0, 320.0), false);
    gpu.upload(&draw);
    let uniforms = vec![0.0; N_UNIFORMS];
    let mut reference = Vec::new();
    for cycle in 0..4 {
        sheet.open = true;
        assert!(gpu.paint(&mut sheet, &draw, &uniforms, false, None));
        assert_eq!(sheet.layers, 1);
        let pixels = read(&gpu, &texture, size);
        if cycle == 0 {
            for k in 0..128 {
                let (x, y) = (k % 16 * 40, k / 16 * 40);
                for dy in 0..40 { for dx in 0..40 {
                    let a = ((y + dy) * size.0 as usize + x + dx) * 4;
                    let b = (dy * size.0 as usize + k % 2 * 40 + dx) * 4;
                    for c in 0..4 { assert!(pixels[a+c].abs_diff(pixels[b+c]) <= 1, "tile {k} differs at {dx},{dy} channel {c}"); }
                }}
            }
            assert!(pixels[(20 * size.0 as usize + 20) * 4 + 3] > 240);
            assert!((126..=129).contains(&pixels[(20 * size.0 as usize + 60) * 4 + 3]));
            if let Some(path) = std::env::var_os("PLEAMAR_EFFECT_IMAGE") {
                let mut rgba = pixels.clone();
                for pixel in rgba.chunks_exact_mut(4) { pixel.swap(0, 2); }
                image::RgbaImage::from_raw(size.0, size.1, rgba).unwrap().save(path).unwrap();
            }
            reference = pixels;
        } else { assert_eq!(pixels, reference); }
        sheet.open = false;
        assert!(gpu.paint(&mut sheet, &draw, &uniforms, false, None));
        assert_eq!(sheet.layers, 0, "closing releases the full-size compositing texture");
        assert!(read(&gpu, &texture, size).iter().all(|b| *b == 0));
    }
}
