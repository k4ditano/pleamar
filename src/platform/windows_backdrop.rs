//! Cropped desktop samples for the renderer's existing glass/unmix pipeline.
//! GDI/DWM work stays off the UI and render threads. Only an outstanding glass
//! request captures pixels; nothing is saved and Classic never starts a worker.
use std::sync::{Arc, Mutex, mpsc::{self, Sender, SyncSender}};
use super::Backdrop;
use crate::scene::ToRender;
use windows::Win32::Graphics::{Gdi::*, Dwm::DwmFlush};

struct Request { rect: [i32; 4], generation: u64, watching: bool }
#[derive(Default)]
struct State { generation: u64, busy: bool, stopped: bool }
pub struct Capture {
    requests: SyncSender<Request>,
    state: Arc<Mutex<State>>,
}
impl Capture {
    pub fn new(sheet: u32, to_render: Sender<ToRender>) -> std::io::Result<Self> {
        let (requests, receiver) = mpsc::sync_channel::<Request>(1);
        let state = Arc::new(Mutex::new(State::default()));
        let shared = state.clone();
        std::thread::Builder::new().name("desktop-glass".into()).spawn(move || {
            let mut buffer = None;
            let mut warned = false;
            while let Ok(request) = receiver.recv() {
                if request.watching { std::thread::sleep(std::time::Duration::from_millis(33)); }
                let active = { let s = shared.lock().unwrap(); !s.stopped && s.generation == request.generation };
                let result = if active { sample(&mut buffer, request.rect) } else { Err("cancelled".into()) };
                let mut state = shared.lock().unwrap();
                state.busy = false;
                if state.stopped { break; }
                if state.generation != request.generation { continue; }
                let data = match result {
                    Ok(pixels) => { warned = false; Some(Arc::new(pixels) as Arc<dyn AsRef<[u8]> + Send + Sync>) }
                    Err(error) => {
                        if !warned { eprintln!("windows · desktop glass unavailable: {error}"); warned = true; }
                        None
                    }
                };
                let [_, _, width, height] = request.rect;
                if to_render.send(ToRender::Backdrop(Box::new(Backdrop {
                    sheet, width: width as u32, height: height as u32,
                    stride: width as u32 * 4, opacity: 1.0, data,
                }))).is_err() { break; }
            }
        })?;
        Ok(Self { requests, state })
    }
    pub fn request(&self, rect: [i32; 4], watching: bool) -> bool {
        if rect[2] <= 0 || rect[3] <= 0 || rect[2] > 16384 || rect[3] > 16384 { return false; }
        let mut state = self.state.lock().unwrap();
        if state.busy { return false; }
        state.busy = true;
        if self.requests.try_send(Request { rect, generation: state.generation, watching }).is_err() {
            state.busy = false;
            return false;
        }
        true
    }
    pub fn cancel(&self) { self.state.lock().unwrap().generation += 1; }
}
impl Drop for Capture {
    fn drop(&mut self) { self.state.lock().unwrap().stopped = true; }
}

struct Buffer { dc: HDC, bitmap: HBITMAP, previous: HGDIOBJ, pixels: *mut u8, size: (i32, i32) }
impl Drop for Buffer {
    fn drop(&mut self) { unsafe {
        SelectObject(self.dc, self.previous);
        let _ = DeleteObject(self.bitmap.into());
        let _ = DeleteDC(self.dc);
    } }
}
struct Screen(HDC);
impl Drop for Screen { fn drop(&mut self) { unsafe { ReleaseDC(None, self.0); } } }
fn sample(buffer: &mut Option<Buffer>, [x, y, width, height]: [i32; 4]) -> Result<Vec<u8>, String> {
    if width <= 0 || height <= 0 || width > 16384 || height > 16384 || width as u64 * height as u64 > 64 * 1024 * 1024 {
        return Err("desktop sample exceeds the 256 MiB capture limit".into());
    }
    unsafe {
        // Include our composition canvas: Lens::receive subtracts exactly the
        // canvas held while this capture is outstanding. Never hide the window.
        DwmFlush().map_err(|e| e.to_string())?;
        let screen = Screen(GetDC(None));
        if screen.0.is_invalid() { return Err("the desktop is unavailable".into()); }
        if buffer.as_ref().is_none_or(|b| b.size != (width, height)) {
            let dc = CreateCompatibleDC(Some(screen.0));
            if dc.is_invalid() { return Err(windows::core::Error::from_thread().to_string()); }
            let mut info = BITMAPINFO::default();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width, biHeight: -height, biPlanes: 1, biBitCount: 32,
                biCompression: BI_RGB.0, ..Default::default()
            };
            let mut pixels = std::ptr::null_mut();
            let bitmap = match CreateDIBSection(Some(screen.0), &info, DIB_RGB_COLORS, &mut pixels, None, 0) {
                Ok(bitmap) => bitmap,
                Err(error) => { let _ = DeleteDC(dc); return Err(error.to_string()); }
            };
            let previous = SelectObject(dc, bitmap.into());
            if previous.is_invalid() || pixels.is_null() {
                let _ = DeleteObject(bitmap.into()); let _ = DeleteDC(dc);
                return Err("could not select the desktop sample buffer".into());
            }
            *buffer = Some(Buffer { dc, bitmap, previous, pixels: pixels.cast(), size: (width, height) });
        }
        let buffer = buffer.as_ref().unwrap();
        BitBlt(buffer.dc, 0, 0, width, height, Some(screen.0), x, y, SRCCOPY | CAPTUREBLT).map_err(|e| e.to_string())?;
        if !GdiFlush().as_bool() { return Err(windows::core::Error::from_thread().to_string()); }
        let mut data = std::slice::from_raw_parts(buffer.pixels, width as usize * height as usize * 4).to_vec();
        for pixel in data.chunks_exact_mut(4) { pixel[3] = 255; }
        Ok(data)
    }
}

pub(super) fn snapshot(rect: [i32; 4]) -> Result<Vec<u8>, String> { sample(&mut None, rect) }
