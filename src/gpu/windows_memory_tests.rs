//! Owned D3D12 textures only: no display capture, window or desktop input.
use super::*;

#[test]
#[ignore = "requires a native D3D12 adapter; no desktop input"]
fn retired_preview_capacity_reopens_with_fresh_pixels() {
    let instance = crate::platform::graphics_instance(wgpu::InstanceFlags::VALIDATION);
    let mut gpu = Gpu::new(&instance, None);
    assert_eq!(gpu.adapter.get_info().backend, wgpu::Backend::Dx12);
    assert!(!gpu.release_windows());
    for cycle in 0..4u8 {
        let size = (1280, 720);
        assert!(gpu.window_room(3, size));
        assert_eq!(gpu.windows_dims, (1280, 768, 4));
        for layer in 0..4 {
            let color = [cycle * 30 + 7, layer as u8 * 40 + 3, 111, 255];
            let pixels = color.repeat((size.0 * size.1) as usize);
            assert!(!gpu.upload_window(layer, size, &pixels));
            let read = gpu.read_window(&[(layer, (0, 0), size, false)], size).unwrap();
            assert_eq!(read, pixels, "fresh pixels in layer {layer}, cycle {cycle}");
        }
        // A close immediately after a queued CPU update must retire safely too.
        gpu.upload_window(3, (1, 1), &[2, 4, 6, 255]);
        assert!(gpu.release_windows());
        assert_eq!(gpu.windows_dims, (1, 1, 1));
        assert!(!gpu.release_windows(), "idle frames must not allocate again");
        assert_eq!(gpu.read_window(&[(0, (0, 0), (1, 1), false)], (1, 1)).unwrap(), [0; 4]);
        assert!(gpu.read_window(&[(3, (0, 0), (1, 1), false)], (1, 1)).is_none());
    }
    // A shared capture waiting for submission still refers to its destination
    // layer; it cannot be replaced by the 1-layer placeholder yet.
    let factory = crate::windows_texture::SharedDevice::from_wgpu(gpu.device()).unwrap();
    let image = factory.texture((17, 19)).unwrap();
    assert!(gpu.copy_windows_texture(2, &image).unwrap());
    assert!(!gpu.release_windows());
    assert_eq!(gpu.windows_dims, (256, 256, 3));
    gpu.flush_copies();
    assert!(gpu.release_windows());
    assert_eq!(gpu.windows_dims, (1, 1, 1));
    assert!(gpu.upload_window(0, (2, 2), &[31, 71, 91, 255].repeat(4)));
    assert_eq!(gpu.read_window(&[(0, (0, 0), (2, 2), false)], (2, 2)).unwrap(), [31, 71, 91, 255].repeat(4));
    eprintln!("PASS: 4 close/reopen cycles, 16 exact layer images, pending-copy guard; texture capacity 15 MiB -> 4 bytes (not process/driver memory)");
}
