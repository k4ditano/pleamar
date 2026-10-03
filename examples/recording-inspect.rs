//! Decode a local recording with Windows' own codecs; no desktop access.
#[cfg(not(target_os = "windows"))]
fn main() { eprintln!("recording-inspect requires Windows Media Foundation"); std::process::exit(1); }
#[cfg(target_os = "windows")]
fn main() {
    if let Err(e) = inspect() { eprintln!("{e}"); std::process::exit(1); }
}
#[cfg(target_os = "windows")]
fn inspect() -> Result<(), Box<dyn std::error::Error>> {
    use windows::{core::PCWSTR, Win32::{Media::MediaFoundation::*, System::WinRT::*}};
    use std::{hash::{Hash, Hasher}, os::windows::ffi::OsStrExt};
    let path = std::env::args_os().nth(1).ok_or("usage: recording-inspect <file.mp4> [first-frame.png]")?;
    let image_path = std::env::args_os().nth(2);
    let path: Vec<u16> = path.encode_wide().chain([0]).collect();
    unsafe {
        RoInitialize(RO_INIT_MULTITHREADED)?;
        MFStartup(MF_VERSION, MFSTARTUP_FULL)?;
        let mut report = serde_json::Map::new();
        for video in [true, false] {
            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 1)?;
            let attributes = attributes.unwrap();
            attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
            let reader = MFCreateSourceReaderFromURL(PCWSTR(path.as_ptr()), &attributes)?;
            reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
            let stream = if video { MF_SOURCE_READER_FIRST_VIDEO_STREAM } else { MF_SOURCE_READER_FIRST_AUDIO_STREAM }.0 as u32;
            reader.SetStreamSelection(stream, true)?;
            let kind = MFCreateMediaType()?;
            kind.SetGUID(&MF_MT_MAJOR_TYPE, if video { &MFMediaType_Video } else { &MFMediaType_Audio })?;
            kind.SetGUID(&MF_MT_SUBTYPE, if video { &MFVideoFormat_RGB32 } else { &MFAudioFormat_PCM })?;
            reader.SetCurrentMediaType(stream, None, &kind)?;
            let actual = reader.GetCurrentMediaType(stream)?;
            let dimensions = if video { actual.GetUINT64(&MF_MT_FRAME_SIZE)? } else { 0 };
            let (width, height) = ((dimensions >> 32) as u32, dimensions as u32);
            let (mut samples, mut bytes, mut nonzero, mut changes) = (0u64, 0u64, 0u64, 0u64);
            let (mut first, mut end, mut last_hash) = (0, 0, 0);
            loop {
                let (mut flags, mut timestamp, mut sample) = (0, 0, None);
                reader.ReadSample(stream, 0, None, Some(&mut flags), Some(&mut timestamp), Some(&mut sample))?;
                if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 { return Err("decoder error".into()); }
                if let Some(sample) = sample {
                    let buffer = sample.ConvertToContiguousBuffer()?;
                    let (mut data, mut length) = (std::ptr::null_mut(), 0);
                    buffer.Lock(&mut data, None, Some(&mut length))?;
                    let pixels = std::slice::from_raw_parts(data, length as usize);
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    pixels.hash(&mut hasher);
                    let hash = hasher.finish();
                    nonzero += pixels.iter().filter(|&&v| v != 0).count() as u64;
                    if samples > 0 && hash != last_hash { changes += 1; }
                    if video && samples == 0 {
                        if let Some(path) = &image_path {
                            let stride = actual.GetUINT32(&MF_MT_DEFAULT_STRIDE)? as i32;
                            let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
                            for y in 0..height as usize {
                                let row = if stride < 0 { height as usize - 1 - y } else { y } * stride.unsigned_abs() as usize;
                                for pixel in pixels[row..row + width as usize * 4].chunks_exact(4) { rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]); }
                            }
                            let file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
                            use image::ImageEncoder;
                            image::codecs::png::PngEncoder::new(file).write_image(&rgba, width, height, image::ExtendedColorType::Rgba8)?;
                        }
                    }
                    buffer.Unlock()?;
                    if samples == 0 { first = timestamp; }
                    end = timestamp + sample.GetSampleDuration().unwrap_or_default();
                    last_hash = hash;
                    bytes += length as u64;
                    samples += 1;
                }
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 { break; }
            }
            if samples == 0 { return Err("stream has no decoded samples".into()); }
            let mut stream_report = serde_json::json!({ "decoded_samples": samples, "bytes": bytes, "nonzero_bytes": nonzero,
                "changed_samples": changes, "first_100ns": first, "end_100ns": end, "seconds": (end - first) as f64 / 10_000_000.0 });
            if video { stream_report["width"] = width.into(); stream_report["height"] = height.into(); }
            else { stream_report["sample_rate"] = actual.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)?.into(); stream_report["channels"] = actual.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)?.into(); }
            report.insert(if video { "video" } else { "audio" }.into(), stream_report);
        }
        println!("{}", serde_json::to_string_pretty(&report)?);
        MFShutdown()?;
        RoUninitialize();
    }
    Ok(())
}
