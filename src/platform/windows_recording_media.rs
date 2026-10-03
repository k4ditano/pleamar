//! H.264/AAC MP4 and WASAPI loopback. No microphone or external recorder.
use std::path::{Path, PathBuf};
use windows::{core::{Interface, PCWSTR, Result}, Win32::{Foundation::*, Graphics::Direct3D11::*, Media::{Audio::*, MediaFoundation::*}, System::{Com::*, Performance::*}, UI::Shell::*}};

pub struct Media;
impl Media { pub fn new() -> Result<Self> { unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL)?; } Ok(Self) } }
impl Drop for Media { fn drop(&mut self) { unsafe { let _ = MFShutdown(); } } }
pub fn clock() -> Result<i64> { unsafe {
    let (mut counter, mut frequency) = (0, 0);
    QueryPerformanceCounter(&mut counter)?;
    QueryPerformanceFrequency(&mut frequency)?;
    Ok((counter as i128 * 10_000_000 / frequency as i128) as i64)
} }
pub fn folder() -> Result<PathBuf> { unsafe {
    let raw = SHGetKnownFolderPath(&FOLDERID_Videos, KF_FLAG_DEFAULT, None)?;
    let result = raw.to_string();
    CoTaskMemFree(Some(raw.0 as _));
    Ok(PathBuf::from(result?).join("Marea"))
} }

pub struct Writer { sink: IMFSinkWriter, stream: IMFByteStream, video: u32, audio: u32, pub path: PathBuf, finalized: bool }
fn video_type(width: u32, height: u32, subtype: &windows::core::GUID) -> Result<IMFMediaType> { unsafe {
    let kind = MFCreateMediaType()?;
    kind.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
    kind.SetGUID(&MF_MT_SUBTYPE, subtype)?;
    kind.SetUINT64(&MF_MT_FRAME_SIZE, ((width as u64) << 32) | height as u64)?;
    kind.SetUINT64(&MF_MT_FRAME_RATE, (60u64 << 32) | 1)?;
    kind.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
    kind.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
    kind.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
    kind.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
    Ok(kind)
} }
fn audio_type(subtype: &windows::core::GUID) -> Result<IMFMediaType> { unsafe {
    let kind = MFCreateMediaType()?;
    kind.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
    kind.SetGUID(&MF_MT_SUBTYPE, subtype)?;
    kind.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
    kind.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, 48_000)?;
    kind.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
    Ok(kind)
} }

impl Writer {
    pub fn new(device: &ID3D11Device, width: u32, height: u32, directory: &Path) -> Result<Self> { unsafe {
        std::fs::create_dir_all(directory).map_err(windows::core::Error::from)?;
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f");
        let mut created = None;
        for index in 0..100 {
            let path = directory.join(format!("marea-{stamp}-{index}.mp4"));
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
            match MFCreateFile(MF_ACCESSMODE_WRITE, MF_OPENMODE_FAIL_IF_EXIST, MF_FILEFLAGS_NONE, PCWSTR(wide.as_ptr())) {
                Ok(stream) => { created = Some((path, stream)); break; }
                Err(e) if e.code() == windows::core::HRESULT::from_win32(ERROR_FILE_EXISTS.0) || e.code() == windows::core::HRESULT::from_win32(ERROR_ALREADY_EXISTS.0) => continue,
                Err(e) => return Err(e),
            }
        }
        let (path, stream) = created.ok_or(E_FAIL)?;
        let configure = || -> Result<(IMFSinkWriter, u32, u32)> {
            let (mut manager, mut token) = (None, 0);
            MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
            let manager = manager.ok_or(E_POINTER)?;
            manager.ResetDevice(device, token)?;
            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 3)?;
            let attributes = attributes.ok_or(E_POINTER)?;
            attributes.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
            attributes.SetUnknown(&MF_SINK_WRITER_D3D_MANAGER, &manager)?;
            attributes.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
            let sink = MFCreateSinkWriterFromURL(PCWSTR::null(), &stream, &attributes)?;
            let output = video_type(width, height, &MFVideoFormat_H264)?;
            output.SetUINT32(&MF_MT_AVG_BITRATE, (width * height * 6).clamp(8_000_000, 40_000_000))?;
            let video = sink.AddStream(&output)?;
            sink.SetInputMediaType(video, &video_type(width, height, &MFVideoFormat_NV12)?, None)?;
            let output = audio_type(&MFAudioFormat_AAC)?;
            output.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 16_000)?;
            output.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)?;
            let audio = sink.AddStream(&output)?;
            let input = audio_type(&MFAudioFormat_PCM)?;
            input.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)?;
            input.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 192_000)?;
            sink.SetInputMediaType(audio, &input, None)?;
            sink.BeginWriting()?;
            Ok((sink, video, audio))
        };
        match configure() {
            Ok((sink, video, audio)) => Ok(Self { sink, stream, video, audio, path, finalized: false }),
            Err(e) => { let _ = stream.Close(); drop(stream); let _ = std::fs::remove_file(path); Err(e) }
        }
    } }
    pub fn video(&self, texture: &ID3D11Texture2D, frame: u64) -> Result<()> { unsafe {
        let buffer = MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, texture, 0, false)?;
        buffer.SetCurrentLength(buffer.cast::<IMF2DBuffer>()?.GetContiguousLength()?)?;
        let sample = MFCreateSample()?;
        sample.AddBuffer(&buffer)?;
        let time = (frame * 10_000_000 / 60) as i64;
        sample.SetSampleTime(time)?;
        sample.SetSampleDuration(((frame + 1) * 10_000_000 / 60) as i64 - time)?;
        self.sink.WriteSample(self.video, &sample)
    } }
    pub fn audio(&self, bytes: &[u8], first_frame: u64) -> Result<()> { unsafe {
        let buffer = MFCreateMemoryBuffer(bytes.len() as u32)?;
        let mut data = std::ptr::null_mut();
        buffer.Lock(&mut data, None, None)?;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
        buffer.Unlock()?;
        buffer.SetCurrentLength(bytes.len() as u32)?;
        let sample = MFCreateSample()?;
        sample.AddBuffer(&buffer)?;
        let time = (first_frame * 10_000_000 / 48_000) as i64;
        sample.SetSampleTime(time)?;
        sample.SetSampleDuration(((first_frame + bytes.len() as u64 / 4) * 10_000_000 / 48_000) as i64 - time)?;
        self.sink.WriteSample(self.audio, &sample)
    } }
    pub fn finish(&mut self) -> Result<()> { unsafe {
        if !self.finalized {
            self.finalized = true;
            self.sink.Finalize()?;
            // The MPEG-4 sink owns and closes its byte stream at finalization.
            // Flushing that already-closed stream returns E_INVALIDARG even
            // though the MP4 and both codecs have finished successfully.
        }
        Ok(())
    } }
}
impl Drop for Writer { fn drop(&mut self) { let _ = self.finish(); unsafe { let _ = self.stream.Close(); } } }
use std::os::windows::ffi::OsStrExt;

pub struct Loopback { client: IAudioClient, capture: IAudioCaptureClient, pub frames: u64, pub nonzero: u64, pub discontinuities: u64 }
impl Loopback {
    pub fn new() -> Result<Self> { unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let client: IAudioClient = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?.Activate(CLSCTX_ALL, None)?;
        let format = WAVEFORMATEX { wFormatTag: 1, nChannels: 2, nSamplesPerSec: 48_000, nAvgBytesPerSec: 192_000, nBlockAlign: 4, wBitsPerSample: 16, cbSize: 0 };
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, 10_000_000, 0, &format, None)?;
        let capture = client.GetService()?;
        Ok(Self { client, capture, frames: 0, nonzero: 0, discontinuities: 0 })
    } }
    pub fn start(&self) -> Result<()> { unsafe { self.client.Start() } }
    pub fn silence_until(&mut self, writer: &Writer, frame: u64) -> Result<()> {
        let zeros = [0u8; 19_200];
        while self.frames < frame {
            let count = (frame - self.frames).min(4800);
            writer.audio(&zeros[..count as usize * 4], self.frames)?;
            self.frames += count;
        }
        Ok(())
    }
    pub fn drain(&mut self, writer: &Writer, start: i64, now: i64) -> Result<()> { unsafe {
        for _ in 0..256 {
            if self.capture.GetNextPacketSize()? == 0 { break; }
            let (mut data, mut frames, mut flags, mut qpc) = (std::ptr::null_mut(), 0, 0, 0);
            self.capture.GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc))?;
            // Release the device's packet before a codec can block.
            let bytes = if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 { vec![0; frames as usize * 4] }
                else { std::slice::from_raw_parts(data, frames as usize * 4).to_vec() };
            self.capture.ReleaseBuffer(frames)?;
            if qpc as i128 > now as i128 + 10_000_000 { return Err(windows::core::Error::new(E_FAIL, "the audio timestamp is ahead of the system clock")); }
            if flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 { self.discontinuities += 1; }
            if flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0 { return Err(windows::core::Error::new(E_FAIL, "the audio endpoint supplied an invalid timestamp")); }
            let packet = ((qpc as i128 - start as i128) * 48_000 / 10_000_000) as i64;
            self.silence_until(writer, packet.max(0) as u64)?;
            let skip = (self.frames as i64 - packet).max(0).min(frames as i64) as usize;
            if skip < frames as usize {
                let bytes = &bytes[skip * 4..];
                self.nonzero += bytes.chunks_exact(2).filter(|b| b[0] != 0 || b[1] != 0).count() as u64;
                writer.audio(bytes, self.frames)?;
                self.frames += bytes.len() as u64 / 4;
            }
        }
        // Loopback produces no packets when all applications are silent. Hold
        // 100 ms for packets in flight, then synthesize actual silence only.
        self.silence_until(writer, ((now - start - 1_000_000).max(0) as u64 * 48_000) / 10_000_000)
    } }
}
impl Drop for Loopback { fn drop(&mut self) { unsafe { let _ = self.client.Stop(); } } }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a D3D11 GPU and Windows H.264/AAC codecs; does not capture the desktop"]
    fn native_recording_unicode_roundtrip() { unsafe {
        use windows::Win32::{Graphics::{Direct3D::*, Dxgi::Common::*}, System::WinRT::*};
        RoInitialize(RO_INIT_MULTITHREADED).unwrap();
        let media = Media::new().unwrap();
        let (mut device, mut context) = (None, None);
        D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut context)).unwrap();
        let device = device.unwrap();
        let context = context.unwrap();
        let _ = context.cast::<ID3D11Multithread>().unwrap().SetMultithreadProtected(true);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let directory = std::env::temp_dir().join(format!("pleamar recording España 海 🚀 {stamp}"));
        std::fs::create_dir(&directory).unwrap();
        let mut pixels = vec![128u8; 320 * 240 * 3 / 2];
        pixels[..320 * 240].fill(16);
        let data = D3D11_SUBRESOURCE_DATA { pSysMem: pixels.as_ptr() as _, SysMemPitch: 320, SysMemSlicePitch: pixels.len() as u32 };
        let mut texture = None;
        device.CreateTexture2D(&D3D11_TEXTURE2D_DESC {
            Width: 320, Height: 240, MipLevels: 1, ArraySize: 1, Format: DXGI_FORMAT_NV12,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 }, Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32, ..Default::default()
        }, Some(&data), Some(&mut texture)).unwrap();
        let texture = texture.unwrap();
        let mut writer = Writer::new(&device, 320, 240, &directory).unwrap();
        let path = writer.path.clone();
        for frame in 0..60 {
            writer.video(&texture, frame).unwrap();
            writer.audio(&[0u8; 3200], frame * 800).unwrap();
        }
        writer.finish().unwrap();
        drop(writer);
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let reader = MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), None).unwrap();
        let video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let audio = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
        assert_eq!(reader.GetNativeMediaType(video, 0).unwrap().GetGUID(&MF_MT_SUBTYPE).unwrap(), MFVideoFormat_H264);
        assert_eq!(reader.GetNativeMediaType(audio, 0).unwrap().GetGUID(&MF_MT_SUBTYPE).unwrap(), MFAudioFormat_AAC);
        reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false).unwrap();
        reader.SetStreamSelection(video, true).unwrap();
        let mut count = 0;
        loop {
            let (mut flags, mut sample) = (0, None);
            reader.ReadSample(video, 0, None, Some(&mut flags), None, Some(&mut sample)).unwrap();
            if sample.is_some() { count += 1; }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 { break; }
        }
        assert_eq!(count, 60);
        drop(reader);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
        drop(texture); drop(context); drop(device); drop(media);
        RoUninitialize();
    } }
}
