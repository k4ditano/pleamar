//! OLE text/file transfers on the HWND's STA thread. Never execute dropped data.
//! Copy is the only offered effect: the source keeps its files on cancellation,
//! rejection and success. The renderer continues while DoDragDrop pumps messages.
use super::{WindowState, WM_PLEAMAR_DRAG};
use crate::scene::ToRender;
use std::{cell::Cell, sync::{Arc, Weak, Mutex, atomic::{AtomicIsize, Ordering}}};
use windows::core::{self as windows_core, implement, Ref, BOOL, HRESULT};
use windows::Win32::{Foundation::*, Graphics::Gdi::ScreenToClient, System::{Com::*, Memory::*, Ole::*, SystemServices::*}, UI::{Shell::*, WindowsAndMessaging::*, Input::KeyboardAndMouse::*}};

const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_FILES: usize = 1024;
static PRESS: AtomicIsize = AtomicIsize::new(0);
static REQUEST: Mutex<Option<(isize, String)>> = Mutex::new(None);
pub(super) struct Apartment;
impl Apartment {
    pub fn new() -> windows_core::Result<Self> { unsafe { OleInitialize(None)?; } Ok(Self) }
}
impl Drop for Apartment { fn drop(&mut self) { unsafe { OleUninitialize(); } } }

pub(super) fn press(hwnd: HWND, down: bool) {
    if down { PRESS.store(hwnd.0 as isize, Ordering::Release); }
    else { let _ = PRESS.compare_exchange(hwnd.0 as isize, 0, Ordering::AcqRel, Ordering::Relaxed); }
}
pub(super) fn queue(text: &str) -> bool {
    if text.trim().is_empty() || text.len() > MAX_BYTES / 2 || text.contains('\0') { return false; }
    let hwnd = PRESS.load(Ordering::Acquire);
    if hwnd == 0 { return false; }
    let mut request = REQUEST.lock().unwrap();
    if request.is_some() { return false; }
    *request = Some((hwnd, text.to_owned()));
    if unsafe { PostMessageW(Some(HWND(hwnd as _)), WM_PLEAMAR_DRAG, WPARAM(0), LPARAM(0)) }.is_err() {
        *request = None;
        return false;
    }
    true
}
pub(super) fn perform(window: &WindowState) {
    let request = REQUEST.lock().unwrap().take();
    let Some((hwnd, text)) = request else { return };
    if hwnd != window.input().0 as isize || window.gone.load(Ordering::Relaxed) { return; }
    unsafe {
        // The renderer queues this after detecting a drag. A very fast release
        // can arrive first; do not start an unrelated drag from a stale press.
        let held = PRESS.load(Ordering::Acquire) == hwnd && GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0;
        let _ = ReleaseCapture();
        if held {
            match source_data(&text) {
                Ok(data) => {
                    let source: IDropSource = Source { hwnd }.into();
                    let mut effect = DROPEFFECT_NONE;
                    let result = DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect);
                    if result.is_err() { eprintln!("windows · drag failed: {result:?}"); }
                }
                Err(error) => eprintln!("windows · drag data unavailable: {error}"),
            }
        }
        PRESS.store(0, Ordering::Release);
        let _ = window.to_render.send(ToRender::Button(0, false));
        let _ = window.to_render.send(ToRender::Pointer(None));
    }
}

#[implement(IDropSource, Agile = false)]
struct Source { hwnd: isize }
impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() || crate::RENDER_DONE.load(Ordering::Relaxed)
            || !unsafe { IsWindowVisible(HWND(self.hwnd as _)) }.as_bool() { DRAGDROP_S_CANCEL }
        else if keys.0 & MK_LBUTTON.0 == 0 { DRAGDROP_S_DROP }
        else { S_OK }
    }
    fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT { DRAGDROP_S_USEDEFAULTCURSORS }
}
fn format(kind: CLIPBOARD_FORMAT) -> FORMATETC {
    FORMATETC { cfFormat: kind.0, dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32, ..Default::default() }
}
struct Medium(STGMEDIUM);
impl Drop for Medium { fn drop(&mut self) { unsafe { ReleaseStgMedium(&mut self.0); } } }
struct Locked(HGLOBAL, *mut u8);
impl Locked {
    fn new(handle: HGLOBAL) -> windows_core::Result<Self> {
        let ptr = unsafe { GlobalLock(handle) }.cast::<u8>();
        if ptr.is_null() { Err(windows_core::Error::from_thread()) } else { Ok(Self(handle, ptr)) }
    }
}
impl Drop for Locked { fn drop(&mut self) { unsafe { let _ = GlobalUnlock(self.0); } } }
fn medium_bytes(bytes: &[u8]) -> windows_core::Result<STGMEDIUM> {
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes.len())?;
        let mut medium = Medium(STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: memory }, ..Default::default() });
        let locked = Locked::new(memory)?;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), locked.1, bytes.len());
        drop(locked);
        // The caller now owns this allocation through ReleaseStgMedium.
        Ok(std::mem::take(&mut medium.0))
    }
}
#[implement(IDataObject)]
struct Data { offers: Vec<(CLIPBOARD_FORMAT, Vec<u8>)> }
impl Data {
    fn offered(&self, requested: *const FORMATETC) -> windows_core::Result<&[u8]> {
        let requested = unsafe { requested.as_ref() }.ok_or_else(|| windows_core::Error::from(E_POINTER))?;
        if requested.tymed & TYMED_HGLOBAL.0 as u32 == 0 { return Err(DV_E_TYMED.into()); }
        if requested.dwAspect != DVASPECT_CONTENT.0 { return Err(DV_E_DVASPECT.into()); }
        if requested.lindex != -1 { return Err(DV_E_LINDEX.into()); }
        self.offers.iter().find(|v| v.0.0 == requested.cfFormat).map(|v| v.1.as_slice()).ok_or_else(|| DV_E_FORMATETC.into())
    }
}
impl IDataObject_Impl for Data_Impl {
    fn GetData(&self, requested: *const FORMATETC) -> windows_core::Result<STGMEDIUM> { medium_bytes(self.offered(requested)?) }
    fn GetDataHere(&self, _: *const FORMATETC, _: *mut STGMEDIUM) -> windows_core::Result<()> { Err(E_NOTIMPL.into()) }
    fn QueryGetData(&self, requested: *const FORMATETC) -> HRESULT { self.offered(requested).map(|_| ()).into() }
    fn GetCanonicalFormatEtc(&self, _: *const FORMATETC, out: *mut FORMATETC) -> HRESULT {
        if out.is_null() { return E_POINTER; }
        unsafe { (*out).ptd = std::ptr::null_mut(); }
        DATA_S_SAMEFORMATETC
    }
    fn SetData(&self, _: *const FORMATETC, _: *const STGMEDIUM, _: BOOL) -> windows_core::Result<()> { Err(E_NOTIMPL.into()) }
    fn EnumFormatEtc(&self, direction: u32) -> windows_core::Result<IEnumFORMATETC> {
        if direction != DATADIR_GET.0 as u32 { return Err(E_NOTIMPL.into()); }
        let formats: Vec<_> = self.offers.iter().map(|v| format(v.0)).collect();
        unsafe { SHCreateStdEnumFmtEtc(&formats) }
    }
    fn DAdvise(&self, _: *const FORMATETC, _: u32, _: Ref<IAdviseSink>) -> windows_core::Result<u32> { Err(OLE_E_ADVISENOTSUPPORTED.into()) }
    fn DUnadvise(&self, _: u32) -> windows_core::Result<()> { Err(OLE_E_ADVISENOTSUPPORTED.into()) }
    fn EnumDAdvise(&self) -> windows_core::Result<IEnumSTATDATA> { Err(OLE_E_ADVISENOTSUPPORTED.into()) }
}
fn wide_bytes(text: &str) -> Vec<u8> { text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect() }
fn source_data(text: &str) -> windows_core::Result<IDataObject> {
    if text.len() > MAX_BYTES / 2 || text.contains('\0') { return Err(E_INVALIDARG.into()); }
    let mut offers = vec![(CF_UNICODETEXT, wide_bytes(text))];
    if let Some(paths) = file_paths(text) {
        // DROPFILES is five DWORDs, followed by double-NUL UTF-16 file names.
        let mut bytes: Vec<u8> = [20u32, 0, 0, 0, 1].into_iter().flat_map(u32::to_le_bytes).collect();
        for path in paths { bytes.extend(wide_bytes(&path)); }
        bytes.extend([0, 0]);
        if bytes.len() <= MAX_BYTES { offers.push((CF_HDROP, bytes)); }
    }
    Ok(Data { offers }.into())
}
fn file_paths(text: &str) -> Option<Vec<String>> {
    let paths: Vec<_> = text.lines().map(str::trim).filter(|v| !v.is_empty()).map(|v| {
        let path = if let Some(uri) = v.strip_prefix("file://") {
            let mut bytes = Vec::new();
            let mut input = uri.as_bytes().iter().copied();
            while let Some(c) = input.next() {
                if c == b'%' {
                    let hi = (input.next()? as char).to_digit(16)?;
                    let lo = (input.next()? as char).to_digit(16)?;
                    bytes.push((hi * 16 + lo) as u8);
                } else { bytes.push(c); }
            }
            let value = String::from_utf8(bytes).ok()?;
            if value.starts_with('/') { value.trim_start_matches('/').to_owned() }
            else { format!("//{value}") }
        } else { v.to_owned() };
        let path = path.replace('/', "\\");
        if path.contains('\0') || !std::path::Path::new(&path).is_absolute() { None } else { Some(path) }
    }).collect::<Option<_>>()?;
    (!paths.is_empty() && paths.len() <= MAX_FILES).then_some(paths)
}
fn file_uri(path: &str) -> String {
    let path = path.replace('\\', "/");
    let encoded: String = path.bytes().map(|b| if b.is_ascii_alphanumeric() || b"/:-._~".contains(&b) {
        (b as char).to_string()
    } else { format!("%{b:02X}") }).collect();
    if encoded.starts_with("//") { format!("file:{encoded}") } else { format!("file:///{encoded}") }
}
fn validate_file_list(bytes: &[u8]) -> windows_core::Result<()> {
    // Validate the bounded HGLOBAL before handing its DROPFILES header to Shell.
    // An external IDataObject need not have constructed a well-formed payload.
    if bytes.len() < 22 { return Err(E_INVALIDARG.into()); }
    let offset = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
    let wide = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) != 0;
    if offset < 20 || offset >= bytes.len() { return Err(E_INVALIDARG.into()); }
    let rest = &bytes[offset..];
    let terminated = if wide {
        offset % 2 == 0 && rest.len() % 2 == 0 && rest.windows(4).step_by(2).any(|p| p == [0, 0, 0, 0])
    } else { rest.windows(2).any(|p| p == [0, 0]) };
    if !terminated { return Err(E_INVALIDARG.into()); }
    Ok(())
}
fn extract(data: &IDataObject, kind: CLIPBOARD_FORMAT) -> windows_core::Result<String> {
    unsafe {
        let medium = Medium(data.GetData(&format(kind))?);
        if medium.0.tymed != TYMED_HGLOBAL.0 as u32 { return Err(E_INVALIDARG.into()); }
        let memory = medium.0.u.hGlobal;
        let length = GlobalSize(memory);
        if length == 0 || length > MAX_BYTES { return Err(E_INVALIDARG.into()); }
        if kind == CF_HDROP {
            let locked = Locked::new(memory)?;
            validate_file_list(std::slice::from_raw_parts(locked.1, length))?;
            drop(locked);
            let drop = HDROP(memory.0);
            let count = DragQueryFileW(drop, u32::MAX, None) as usize;
            if count == 0 || count > MAX_FILES { return Err(E_INVALIDARG.into()); }
            let mut uris = String::new();
            for index in 0..count {
                let len = DragQueryFileW(drop, index as u32, None) as usize;
                if len == 0 || len > 32767 { return Err(E_INVALIDARG.into()); }
                let mut text = vec![0; len + 1];
                if DragQueryFileW(drop, index as u32, Some(&mut text)) as usize != len { return Err(E_INVALIDARG.into()); }
                let path = String::from_utf16(&text[..len]).map_err(|_| windows_core::Error::from(E_INVALIDARG))?;
                uris.push_str(&file_uri(&path)); uris.push_str("\r\n");
                if uris.len() > MAX_BYTES { return Err(E_INVALIDARG.into()); }
            }
            Ok(uris)
        } else {
            if length % 2 != 0 { return Err(E_INVALIDARG.into()); }
            let locked = Locked::new(memory)?;
            let text = std::slice::from_raw_parts(locked.1.cast::<u16>(), length / 2);
            let end = text.iter().position(|v| *v == 0).ok_or_else(|| windows_core::Error::from(E_INVALIDARG))?;
            String::from_utf16(&text[..end]).map_err(|_| E_INVALIDARG.into())
        }
    }
}

// The target owns Cells and calls its HWND: COM must marshal foreign callers
// back to the registered STA, not expose the macro's default agile object.
#[implement(IDropTarget, Agile = false)]
struct Target { window: Weak<WindowState>, kind: Cell<Option<CLIPBOARD_FORMAT>> }
impl Target {
    fn position(&self, point: &POINTL, effect: *mut DROPEFFECT) -> windows_core::Result<()> {
        if effect.is_null() { return Err(E_POINTER.into()); }
        let Some(window) = self.window.upgrade().filter(|w| !w.gone.load(Ordering::Relaxed)) else {
            unsafe { *effect = DROPEFFECT_NONE; } return Ok(());
        };
        let mut p = POINT { x: point.x, y: point.y };
        unsafe {
            if !ScreenToClient(window.input(), &mut p).as_bool() { *effect = DROPEFFECT_NONE; return Ok(()); }
            *effect = if self.kind.get().is_some() { *effect & DROPEFFECT_COPY } else { DROPEFFECT_NONE };
        }
        window.pointer(p.x, p.y);
        Ok(())
    }
    fn leave(&self) {
        self.kind.set(None);
        if let Some(window) = self.window.upgrade() {
            let _ = window.to_render.send(ToRender::Fact("drag.over", 0.0));
            let _ = window.to_render.send(ToRender::Pointer(None));
        }
    }
}
impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, data: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, point: &POINTL, effect: *mut DROPEFFECT) -> windows_core::Result<()> {
        let kind = data.as_ref().and_then(|data| [CF_HDROP, CF_UNICODETEXT].into_iter().find(|k| unsafe { data.QueryGetData(&format(*k)) }.is_ok()));
        self.kind.set(kind);
        if let Some(window) = self.window.upgrade() {
            let _ = window.to_render.send(ToRender::Fact("drag.over", if kind.is_some() { 1.0 } else { 0.0 }));
        }
        self.position(point, effect)
    }
    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, point: &POINTL, effect: *mut DROPEFFECT) -> windows_core::Result<()> { self.position(point, effect) }
    fn DragLeave(&self) -> windows_core::Result<()> { self.leave(); Ok(()) }
    fn Drop(&self, data: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, point: &POINTL, effect: *mut DROPEFFECT) -> windows_core::Result<()> {
        self.position(point, effect)?;
        let result = (|| {
            if unsafe { *effect } == DROPEFFECT_NONE { return Ok(()); }
            let kind = self.kind.get().ok_or_else(|| windows_core::Error::from(E_INVALIDARG))?;
            let text = extract(data.ok()?, kind)?;
            let window = self.window.upgrade().ok_or_else(|| windows_core::Error::from(E_FAIL))?;
            let mime = if kind == CF_HDROP { "text/uri-list" } else { "text/plain;charset=utf-8" };
            window.to_render.send(ToRender::Dropped(mime.into(), text)).map_err(|_| windows_core::Error::from(E_FAIL))
        })();
        if result.is_err() { unsafe { *effect = DROPEFFECT_NONE; } }
        // Renderer defers these two messages until it has handled the drop.
        self.leave();
        result
    }
}
pub(super) fn register(window: &Arc<WindowState>) -> windows_core::Result<()> {
    let target: IDropTarget = Target { window: Arc::downgrade(window), kind: Cell::new(None) }.into();
    unsafe { RegisterDragDrop(window.input(), &target) }
}
pub(super) fn revoke(hwnd: HWND) {
    press(hwnd, false);
    let mut request = REQUEST.lock().unwrap();
    if request.as_ref().is_some_and(|r| r.0 == hwnd.0 as isize) { *request = None; }
    drop(request);
    unsafe { let _ = RevokeDragDrop(hwnd); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_urls_roundtrip_spaces_unicode_and_unc() {
        for path in [r"C:\Images ñ\海 🚀.png", r"\\server\share\file #1%.txt"] {
            assert_eq!(file_paths(&file_uri(path)), Some(vec![path.into()]));
        }
        for value in ["relative.txt", "https://example.com/test", "file:///C:/bad%GG", "C:\\nul\0.txt"] {
            assert!(file_paths(value).is_none(), "{value:?}");
        }
    }
    #[test]
    fn native_data_object_roundtrip() {
        let _ole = Apartment::new().unwrap();
        let text = "España ñ 世界 🚀";
        let data = source_data(text).unwrap();
        assert_eq!(extract(&data, CF_UNICODETEXT).unwrap(), text);
        assert!(unsafe { data.QueryGetData(&format(CF_HDROP)) }.is_err());
        let paths = "C:\\Test ñ\\a b.txt\n\\\\server\\share\\世界.png";
        let data = source_data(paths).unwrap();
        let uris = extract(&data, CF_HDROP).unwrap();
        assert_eq!(file_paths(&uris), file_paths(paths));
        // Malformed offsets and missing terminators from external data objects.
        let mut malformed = [1u8; 24];
        assert!(validate_file_list(&malformed).is_err());
        malformed[..4].copy_from_slice(&20u32.to_le_bytes());
        assert!(validate_file_list(&malformed).is_err());
        let data: IDataObject = Data { offers: vec![(CF_HDROP, malformed.to_vec())] }.into();
        assert!(extract(&data, CF_HDROP).is_err());
    }
}
