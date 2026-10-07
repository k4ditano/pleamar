use super::*;
use std::{sync::mpsc, time::{Duration,Instant}};
use windows::Win32::Graphics::{Direct3D::*, Dxgi::*};

#[test]
#[ignore = "requires a native D3D12 adapter; performs no desktop input"]
fn shared_capture_reuses_native_pixels_without_cpu_upload() { unsafe {
    let instance=wgpu::Instance::new(wgpu::InstanceDescriptor {backends:wgpu::Backends::DX12,..wgpu::InstanceDescriptor::new_without_display_handle()});
    let adapter=pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device,queue)=pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let factory=SharedDevice::from_wgpu(&device).unwrap();
    assert!(factory.texture((0,1)).is_err());
    assert!(factory.texture((8192,8192)).is_err());
    let dxgi:IDXGIFactory4=CreateDXGIFactory1().unwrap();
    let native_adapter:IDXGIAdapter=dxgi.EnumAdapterByLuid(factory.adapter()).unwrap();
    let (mut producer,mut context)=(None,None);
    D3D11CreateDevice(&native_adapter,D3D_DRIVER_TYPE_UNKNOWN,HMODULE::default(),D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        None,D3D11_SDK_VERSION,Some(&mut producer),None,Some(&mut context)).unwrap();
    let producer=producer.unwrap();let context=context.unwrap();
    let (other_device,_other_queue)=pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let context4:ID3D11DeviceContext4=context.cast().unwrap();
    let mut fence:Option<ID3D11Fence>=None;
    producer.cast::<ID3D11Device5>().unwrap().CreateFence(0,D3D11_FENCE_FLAG_NONE,&mut fence).unwrap();
    let fence=fence.unwrap();
    for (width,height) in [(17,19),(65,23)] {
        let image=factory.texture((width,height)).unwrap();
        assert!(SharedTexture::import(&image,&other_device).is_err(),"another logical wgpu device must not reuse this import");
        let destination=image.open(&producer).unwrap();
        let mut source:Option<ID3D11Texture2D>=None;
        producer.CreateTexture2D(&D3D11_TEXTURE2D_DESC {Width:width,Height:height,MipLevels:1,ArraySize:1,
            Format:DXGI_FORMAT_B8G8R8A8_UNORM,SampleDesc:DXGI_SAMPLE_DESC {Count:1,Quality:0},
            Usage:D3D11_USAGE_DEFAULT,..Default::default()},None,Some(&mut source)).unwrap();
        let source=source.unwrap();
        for round in 1..=6u64 {
            assert_eq!(Arc::strong_count(&image),1,"producer must have exclusive ownership before writing");
            let pixels:Vec<u8>=(0..width*height).flat_map(|i|[i as u8,(i/width) as u8,round as u8,255]).collect();
            context.UpdateSubresource(&source,0,None,pixels.as_ptr().cast(),width*4,0);
            context.CopyResource(&destination,&source);
            // Monotonic across both sizes, including the old resources' retirement.
            let value=width as u64*10+round;
            context4.Signal(&fence,value).unwrap();context.Flush();
            let deadline=Instant::now()+Duration::from_secs(5);
            while fence.GetCompletedValue()<value {
                assert!(Instant::now()<deadline,"producer GPU copy timed out");
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_ne!(fence.GetCompletedValue(),u64::MAX);
            let texture=SharedTexture::import(&image,&device).unwrap();
            let pitch=(width*4).div_ceil(256)*256;
            let buffer=device.create_buffer(&wgpu::BufferDescriptor {label:None,size:pitch as u64*height as u64,
                usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
            let mut encoder=device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(texture.as_image_copy(),wgpu::TexelCopyBufferInfo {buffer:&buffer,
                layout:wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(pitch),rows_per_image:None}},texture.size());
            queue.submit([encoder.finish()]);
            let (tx,rx)=mpsc::channel();
            let loan=image.clone();
            queue.on_submitted_work_done(move ||drop(loan));
            buffer.slice(..).map_async(wgpu::MapMode::Read,move |result|{let _=tx.send(result);});
            let deadline=Instant::now()+Duration::from_secs(5);
            loop {
                let _=device.poll(wgpu::PollType::Poll);
                if let Ok(result)=rx.try_recv() {result.unwrap();break;}
                assert!(Instant::now()<deadline,"consumer GPU copy timed out");
                std::thread::sleep(Duration::from_millis(1));
            }
            let view=buffer.slice(..).get_mapped_range().unwrap();
            for y in 0..height as usize {
                assert_eq!(&view[y*pitch as usize..y*pitch as usize+width as usize*4],
                    &pixels[y*width as usize*4..(y+1)*width as usize*4],"shared image changed at row {y}, round {round}");
            }
            drop(view);buffer.unmap();
        }
        let retired=Arc::downgrade(&image);
        drop(image);
        assert!(retired.upgrade().is_none(),"the import must not retain its owner");
    }
    eprintln!("PASS: 12 D3D11 -> D3D12 GPU copies, exact BGRA pixels, reuse, resize and retired imports");
} }
