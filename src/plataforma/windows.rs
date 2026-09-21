//! Win32: transparent DirectComposition windows, monitor placement and input.
//!
//! The renderer still owns every pixel and every animation. This module only
//! turns the scene's surfaces into HWNDs, translates Win32 input into
//! `ARender`, and tells Windows which logical rectangles accept a click.

use super::Ventana;
use crate::escena::{ARender, Ancla, Cursor, Mods, Nivel, Pantallas, Superficie, Teclado};
use crate::gpu;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};
use std::ffi::c_void;
use std::mem::size_of;
use std::num::NonZeroIsize;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak, mpsc::Sender};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW,
    HBRUSH, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW, ScreenToClient,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor,
    MDT_EFFECTIVE_DPI, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyNameTextW, GetKeyState, GetKeyboardLayout, GetKeyboardState, TME_LEAVE, TRACKMOUSEEVENT,
    ToUnicodeEx, TrackMouseEvent, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END,
    VK_ESCAPE, VK_HOME, VK_INSERT, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_NEXT,
    VK_PRIOR, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::Shell::{
    ABE_BOTTOM, ABE_LEFT, ABE_RIGHT, ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS,
    APPBARDATA, SHAppBarMessage,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, PCWSTR, w};

const WM_PLEAMAR_CURSOR: u32 = WM_APP + 1;
const WM_PLEAMAR_TECLADO: u32 = WM_APP + 2;
const WM_PLEAMAR_RECOLOCAR: u32 = WM_APP + 3;
const WM_PLEAMAR_SALIR: u32 = WM_APP + 4;
const WM_MOUSELEAVE_MSG: u32 = 0x02a3;

#[derive(Clone)]
struct Monitor {
    rect: RECT,
    nombre: String,
    escala: f32,
    mhz: i32,
}

#[derive(Clone)]
struct Colocacion {
    monitor: RECT,
    ancla: Ancla,
    margen: [i32; 4],
    ancho: u32,
    alto: u32,
    reserva: i32,
    nivel: Nivel,
}

struct EstadoVentana {
    id: u32,
    cual: usize,
    hwnd: AtomicIsize,
    a_render: Sender<ARender>,
    origen: (f32, f32),
    escala: AtomicU32,
    cajas: Mutex<Vec<[i32; 4]>>,
    cursor: AtomicU8,
    teclado: AtomicU8,
    raton_dentro: AtomicBool,
    derecho_cierra: bool,
    es_ventana: bool,
    colocacion: Mutex<Option<Colocacion>>,
    appbar: AtomicBool,
    fuera: AtomicBool,
}

impl EstadoVentana {
    fn hwnd(&self) -> HWND {
        HWND(self.hwnd.load(Ordering::Relaxed) as *mut c_void)
    }

    fn escala(&self) -> f32 {
        f32::from_bits(self.escala.load(Ordering::Relaxed)).max(0.25)
    }

    fn puntero(&self, x_px: i32, y_px: i32) {
        let e = self.escala();
        let _ = self.a_render.send(ARender::Puntero(Some((
            x_px as f32 / e + self.origen.0,
            y_px as f32 / e + self.origen.1,
        ))));
    }

    fn se_fue(&self) {
        if !self.fuera.swap(true, Ordering::Relaxed) {
            let _ = self.a_render.send(ARender::LaminaFuera(self.id));
        }
    }
}

/// The renderer only needs an HWND it can talk to and the shared hit-test state.
struct VentanaWindows(Arc<EstadoVentana>);

impl Ventana for VentanaWindows {
    fn region_de_entrada(&self, cajas: &[[i32; 4]]) {
        *self.0.cajas.lock().unwrap() = cajas.to_vec();
    }

    fn cursor(&self, c: Cursor) {
        self.0.cursor.store(
            match c {
                Cursor::Normal => 0,
                Cursor::Mano => 1,
                Cursor::Texto => 2,
                Cursor::Agarrar => 3,
                Cursor::Agarrando => 4,
            },
            Ordering::Relaxed,
        );
        unsafe {
            let _ = PostMessageW(Some(self.0.hwnd()), WM_PLEAMAR_CURSOR, WPARAM(0), LPARAM(0));
        }
    }

    fn teclado(&self, t: Teclado) {
        self.0.teclado.store(teclado_num(t), Ordering::Relaxed);
        unsafe {
            let _ = PostMessageW(
                Some(self.0.hwnd()),
                WM_PLEAMAR_TECLADO,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

struct Plataforma {
    ventanas: Mutex<Vec<Weak<EstadoVentana>>>,
}

static PLATAFORMA: OnceLock<Plataforma> = OnceLock::new();

fn teclado_num(t: Teclado) -> u8 {
    match t {
        Teclado::Nunca => 0,
        Teclado::AlPulsar => 1,
        Teclado::Siempre => 2,
    }
}

fn escala_de(monitor: HMONITOR) -> f32 {
    let (mut x, mut y) = (96, 96);
    unsafe {
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y).is_err() {
            return 1.0;
        }
    }
    x as f32 / 96.0
}

unsafe extern "system" fn contar_monitor(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    dato: LPARAM,
) -> BOOL {
    let monitores = unsafe { &mut *(dato.0 as *mut Vec<Monitor>) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if !unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo as *mut MONITORINFO) }.as_bool() {
        return BOOL(1);
    }
    let fin = info
        .szDevice
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(info.szDevice.len());
    let nombre = String::from_utf16_lossy(&info.szDevice[..fin]);
    let mut nombre_w = info.szDevice[..fin].to_vec();
    nombre_w.push(0);
    let mut modo = DEVMODEW {
        dmSize: size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    let hz = if unsafe {
        EnumDisplaySettingsW(PCWSTR(nombre_w.as_ptr()), ENUM_CURRENT_SETTINGS, &mut modo)
    }
    .as_bool()
    {
        modo.dmDisplayFrequency as i32 * 1000
    } else {
        0
    };
    monitores.push(Monitor {
        rect: info.monitorInfo.rcMonitor,
        nombre,
        escala: escala_de(monitor),
        mhz: hz,
    });
    BOOL(1)
}

fn monitores() -> Vec<Monitor> {
    let mut resultado = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(contar_monitor),
            LPARAM((&mut resultado as *mut Vec<Monitor>) as isize),
        );
    }
    resultado
}

fn quiere_en(s: &Superficie, nombre: &str, k: usize) -> usize {
    match &s.pantallas {
        Pantallas::Todas => 1,
        Pantallas::Estas(n) => n.iter().filter(|x| x.as_str() == nombre).count(),
        Pantallas::Numero(x) => (*x == k) as usize,
    }
}

fn px(n: i32, escala: f32) -> i32 {
    (n as f32 * escala).round() as i32
}

fn rect_panel(c: &Colocacion, escala: f32) -> (RECT, (u32, u32)) {
    let m = c.monitor;
    let margen = [
        px(c.margen[0], escala),
        px(c.margen[1], escala),
        px(c.margen[2], escala),
        px(c.margen[3], escala),
    ];
    let area = RECT {
        left: m.left + margen[3],
        top: m.top + margen[0],
        right: m.right - margen[1],
        bottom: m.bottom - margen[2],
    };
    let monitor_w = (area.right - area.left).max(1);
    let monitor_h = (area.bottom - area.top).max(1);
    let w = if c.ancho == 0 {
        monitor_w
    } else {
        px(c.ancho as i32, escala).max(1)
    };
    let h = px(c.alto as i32, escala).max(1);
    let centro_x = area.left + (monitor_w - w) / 2;
    let centro_y = area.top + (monitor_h - h) / 2;
    let (x, y) = match c.ancla {
        Ancla::Arriba => (centro_x, area.top),
        Ancla::Abajo => (centro_x, area.bottom - h),
        Ancla::Izquierda => (area.left, centro_y),
        Ancla::Derecha => (area.right - w, centro_y),
        Ancla::ArribaIzquierda => (area.left, area.top),
        Ancla::ArribaDerecha => (area.right - w, area.top),
        Ancla::AbajoIzquierda => (area.left, area.bottom - h),
        Ancla::AbajoDerecha => (area.right - w, area.bottom - h),
        Ancla::Centro => (centro_x, centro_y),
    };
    (
        RECT {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        },
        ((w as f32 / escala).round() as u32, c.alto),
    )
}

fn borde_appbar(a: Ancla) -> u32 {
    match a {
        Ancla::Abajo | Ancla::AbajoIzquierda | Ancla::AbajoDerecha => ABE_BOTTOM,
        Ancla::Izquierda => ABE_LEFT,
        Ancla::Derecha => ABE_RIGHT,
        _ => ABE_TOP,
    }
}

unsafe fn quitar_appbar(e: &EstadoVentana) {
    if e.appbar.swap(false, Ordering::Relaxed) {
        let mut d = APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            hWnd: e.hwnd(),
            ..Default::default()
        };
        unsafe { SHAppBarMessage(ABM_REMOVE, &mut d) };
    }
}

unsafe fn poner_appbar(e: &EstadoVentana, c: &Colocacion, escala: f32) -> Option<RECT> {
    if c.reserva <= 0 || e.es_ventana {
        unsafe { quitar_appbar(e) };
        return None;
    }
    let edge = borde_appbar(c.ancla);
    let grosor = px(c.reserva, escala).max(1);
    let mut d = APPBARDATA {
        cbSize: size_of::<APPBARDATA>() as u32,
        hWnd: e.hwnd(),
        uEdge: edge,
        rc: c.monitor,
        ..Default::default()
    };
    if !e.appbar.load(Ordering::Relaxed) {
        if unsafe { SHAppBarMessage(ABM_NEW, &mut d) } == 0 {
            eprintln!("windows · the shell refused the AppBar reservation");
            return None;
        }
        e.appbar.store(true, Ordering::Relaxed);
    }
    match edge {
        ABE_BOTTOM => d.rc.top = d.rc.bottom - grosor,
        ABE_LEFT => d.rc.right = d.rc.left + grosor,
        ABE_RIGHT => d.rc.left = d.rc.right - grosor,
        _ => d.rc.bottom = d.rc.top + grosor,
    }
    unsafe { SHAppBarMessage(ABM_QUERYPOS, &mut d) };
    match edge {
        ABE_BOTTOM => d.rc.top = d.rc.bottom - grosor,
        ABE_LEFT => d.rc.right = d.rc.left + grosor,
        ABE_RIGHT => d.rc.left = d.rc.right - grosor,
        _ => d.rc.bottom = d.rc.top + grosor,
    }
    unsafe { SHAppBarMessage(ABM_SETPOS, &mut d) };
    Some(d.rc)
}

unsafe fn recolocar(e: &EstadoVentana) {
    let Some(c) = e.colocacion.lock().unwrap().clone() else {
        return;
    };
    let escala = e.escala();
    let reservada = unsafe { poner_appbar(e, &c, escala) };
    let (mut r, _) = rect_panel(&c, escala);
    // The shell may move our reservation away from another AppBar (normally
    // the Windows taskbar). Keep the panel inside the rectangle it granted.
    if let Some(a) = reservada {
        let w = r.right - r.left;
        let h = r.bottom - r.top;
        match borde_appbar(c.ancla) {
            ABE_BOTTOM => {
                r.top = a.bottom - h;
                r.bottom = a.bottom;
            }
            ABE_LEFT => {
                r.left = a.left;
                r.right = a.left + w;
            }
            ABE_RIGHT => {
                r.left = a.right - w;
                r.right = a.right;
            }
            _ => {
                r.top = a.top;
                r.bottom = a.top + h;
            }
        }
    }
    let z = match c.nivel {
        Nivel::Fondo | Nivel::Debajo => Some(HWND_BOTTOM),
        Nivel::Encima | Nivel::SobreTodo => Some(HWND_TOPMOST),
    };
    let _ = unsafe {
        SetWindowPos(
            e.hwnd(),
            z,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            SWP_NOACTIVATE,
        )
    };
}

fn cursor_de(n: u8) -> PCWSTR {
    match n {
        1 => IDC_HAND,
        2 => IDC_IBEAM,
        3 => IDC_SIZEALL,
        4 => IDC_SIZEALL,
        _ => IDC_ARROW,
    }
}

unsafe fn aplicar_cursor(e: &EstadoVentana) {
    if let Ok(c) = unsafe { LoadCursorW(None, cursor_de(e.cursor.load(Ordering::Relaxed))) } {
        unsafe { SetCursor(Some(c)) };
    }
}

unsafe fn aplicar_teclado(e: &EstadoVentana) {
    if e.es_ventana {
        return;
    }
    let hwnd = e.hwnd();
    let mut ex = WINDOW_EX_STYLE(unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32);
    if e.teclado.load(Ordering::Relaxed) == 0 {
        ex |= WS_EX_NOACTIVATE;
    } else {
        ex &= !WS_EX_NOACTIVATE;
    }
    unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex.0 as isize) };
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED | SWP_NOACTIVATE,
        )
    };
}

fn mods() -> Mods {
    let down = |v: VIRTUAL_KEY| unsafe { GetKeyState(v.0 as i32) < 0 };
    Mods {
        ctrl: down(VK_CONTROL),
        alt: down(VK_MENU),
        mayus: down(VK_SHIFT),
        logo: down(VK_LWIN) || down(VK_RWIN),
    }
}

fn nombre_tecla(vk: u32, lparam: LPARAM) -> String {
    match vk as u16 {
        x if x == VK_ESCAPE.0 => "Escape".into(),
        x if x == VK_RETURN.0 => "Return".into(),
        x if x == VK_BACK.0 => "BackSpace".into(),
        x if x == VK_TAB.0 => "Tab".into(),
        x if x == VK_SPACE.0 => "space".into(),
        x if x == VK_LEFT.0 => "Left".into(),
        x if x == VK_RIGHT.0 => "Right".into(),
        x if x == VK_UP.0 => "Up".into(),
        x if x == VK_DOWN.0 => "Down".into(),
        x if x == VK_HOME.0 => "Home".into(),
        x if x == VK_END.0 => "End".into(),
        x if x == VK_PRIOR.0 => "Page_Up".into(),
        x if x == VK_NEXT.0 => "Page_Down".into(),
        x if x == VK_INSERT.0 => "Insert".into(),
        x if x == VK_DELETE.0 => "Delete".into(),
        0x41..=0x5a => char::from_u32(vk + 32).unwrap().to_string(),
        0x30..=0x39 => char::from_u32(vk).unwrap().to_string(),
        _ => {
            let mut b = [0u16; 64];
            let n = unsafe { GetKeyNameTextW(lparam.0 as i32, &mut b) };
            if n > 0 {
                String::from_utf16_lossy(&b[..n as usize])
            } else {
                format!("0x{vk:x}")
            }
        }
    }
}

fn texto_tecla(vk: u32, lparam: LPARAM, m: Mods) -> Option<String> {
    if m.ctrl
        || m.alt
        || matches!(vk as u16, x if [VK_SHIFT.0, VK_LSHIFT.0, VK_RSHIFT.0, VK_CONTROL.0, VK_MENU.0, VK_LMENU.0, VK_RMENU.0, VK_LWIN.0, VK_RWIN.0].contains(&x))
    {
        return None;
    }
    let mut estado = [0u8; 256];
    unsafe { GetKeyboardState(&mut estado).ok()? };
    let mut b = [0u16; 8];
    let scan = ((lparam.0 as u32) >> 16) & 0xff;
    let n = unsafe { ToUnicodeEx(vk, scan, &estado, &mut b, 0, Some(GetKeyboardLayout(0))) };
    (n > 0)
        .then(|| String::from_utf16_lossy(&b[..n as usize]))
        .filter(|t| !t.chars().any(char::is_control))
}

fn estado(hwnd: HWND) -> Option<&'static EstadoVentana> {
    let p = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const EstadoVentana;
    unsafe { p.as_ref() }
}

unsafe extern "system" fn ventana_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let c = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        let p = c.lpCreateParams as *const EstadoVentana;
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, p as isize) };
        if let Some(e) = unsafe { p.as_ref() } {
            e.hwnd.store(hwnd.0 as isize, Ordering::Relaxed);
        }
    }
    let Some(e) = estado(hwnd) else {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    match msg {
        WM_NCHITTEST => {
            if e.es_ventana {
                return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
            }
            let mut p = POINT {
                x: bajo_i16(lparam.0),
                y: alto_i16(lparam.0),
            };
            let _ = unsafe { ScreenToClient(hwnd, &mut p) };
            let s = e.escala();
            let (x, y) = (p.x as f32 / s, p.y as f32 / s);
            if e.cajas
                .lock()
                .unwrap()
                .iter()
                .any(|b| x >= b[0] as f32 && x < b[2] as f32 && y >= b[1] as f32 && y < b[3] as f32)
            {
                return LRESULT(HTCLIENT as isize);
            }
            return LRESULT(HTTRANSPARENT as isize);
        }
        WM_MOUSEMOVE => {
            if !e.raton_dentro.swap(true, Ordering::Relaxed) {
                let mut t = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = unsafe { TrackMouseEvent(&mut t) };
            }
            e.puntero(bajo_i16(lparam.0), alto_i16(lparam.0));
            return LRESULT(0);
        }
        WM_MOUSELEAVE_MSG => {
            e.raton_dentro.store(false, Ordering::Relaxed);
            let _ = e.a_render.send(ARender::Puntero(None));
            return LRESULT(0);
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONUP
        | WM_MBUTTONUP => {
            let abajo = matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN);
            let boton = if matches!(msg, WM_LBUTTONDOWN | WM_LBUTTONUP) {
                0
            } else if matches!(msg, WM_RBUTTONDOWN | WM_RBUTTONUP) {
                1
            } else {
                2
            };
            if boton == 1 && abajo && e.derecho_cierra {
                unsafe { PostQuitMessage(0) };
                return LRESULT(0);
            }
            e.puntero(bajo_i16(lparam.0), alto_i16(lparam.0));
            let _ = e.a_render.send(ARender::Boton(boton, abajo));
            if abajo && e.teclado.load(Ordering::Relaxed) != 0 {
                let _ = unsafe { SetForegroundWindow(hwnd) };
            }
            return LRESULT(0);
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let delta = alto_u16(wparam.0 as isize) as i16 as f32 / 120.0;
            let _ = e.a_render.send(ARender::Rueda(if msg == WM_MOUSEWHEEL {
                delta
            } else {
                -delta
            }));
            return LRESULT(0);
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let m = mods();
            let nombre = nombre_tecla(wparam.0 as u32, lparam);
            let escribe = texto_tecla(wparam.0 as u32, lparam, m);
            let _ = e.a_render.send(ARender::Tecla(nombre, escribe, m));
            return LRESULT(0);
        }
        WM_KEYUP | WM_SYSKEYUP => {
            let _ = e
                .a_render
                .send(ARender::TeclaSuelta(nombre_tecla(wparam.0 as u32, lparam)));
            return LRESULT(0);
        }
        WM_SETFOCUS => {
            let _ = e.a_render.send(ARender::FocoTeclado(true));
            return LRESULT(0);
        }
        WM_KILLFOCUS => {
            let _ = e.a_render.send(ARender::FocoTeclado(false));
            return LRESULT(0);
        }
        WM_SIZE if e.es_ventana => {
            let s = e.escala();
            let nuevo = (bajo_u16(lparam.0) as f32 / s, alto_u16(lparam.0) as f32 / s);
            if nuevo.0 > 0.0 && nuevo.1 > 0.0 {
                let _ = e.a_render.send(ARender::TamLamina(e.id, nuevo));
            }
            return LRESULT(0);
        }
        WM_DPICHANGED => {
            let nueva = bajo_u16(wparam.0 as isize) as f32 / 96.0;
            e.escala.store(nueva.to_bits(), Ordering::Relaxed);
            let _ = e.a_render.send(ARender::Escala(e.id, nueva));
            if e.es_ventana {
                let r = unsafe { &*(lparam.0 as *const RECT) };
                let _ = unsafe {
                    SetWindowPos(
                        hwnd,
                        None,
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOACTIVATE | SWP_NOZORDER,
                    )
                };
            } else {
                unsafe { recolocar(e) };
            }
            return LRESULT(0);
        }
        WM_DISPLAYCHANGE => {
            if !e.es_ventana {
                unsafe { recolocar(e) };
            }
        }
        WM_SETCURSOR | WM_PLEAMAR_CURSOR => {
            unsafe { aplicar_cursor(e) };
            return LRESULT(1);
        }
        WM_PLEAMAR_TECLADO => {
            unsafe { aplicar_teclado(e) };
            return LRESULT(0);
        }
        WM_PLEAMAR_RECOLOCAR => {
            unsafe { recolocar(e) };
            return LRESULT(0);
        }
        WM_PLEAMAR_SALIR => {
            unsafe { PostQuitMessage(0) };
            return LRESULT(0);
        }
        WM_CLOSE => {
            let _ = unsafe { DestroyWindow(hwnd) };
            return LRESULT(0);
        }
        WM_DESTROY => {
            unsafe { quitar_appbar(e) };
            e.se_fue();
            if e.es_ventana {
                unsafe { PostQuitMessage(0) };
            }
            return LRESULT(0);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_NCDESTROY => {
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            let p = e as *const EstadoVentana;
            unsafe { drop(Arc::from_raw(p)) };
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn bajo_u16(v: isize) -> u16 {
    v as u16
}
fn alto_u16(v: isize) -> u16 {
    ((v as usize >> 16) & 0xffff) as u16
}
fn bajo_i16(v: isize) -> i32 {
    bajo_u16(v) as i16 as i32
}
fn alto_i16(v: isize) -> i32 {
    alto_u16(v) as i16 as i32
}

fn registrar_clase(instancia: HINSTANCE) -> Result<(), String> {
    let clase = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW | CS_OWNDC,
        lpfnWndProc: Some(ventana_proc),
        hInstance: instancia,
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
        hbrBackground: HBRUSH(std::ptr::null_mut()),
        lpszClassName: w!("PleamarWindow"),
        ..Default::default()
    };
    let a = unsafe { RegisterClassExW(&clase) };
    if a == 0 {
        Err("Win32 could not register the pleamar window class".into())
    } else {
        Ok(())
    }
}

fn crear(
    p: &Superficie,
    cual: usize,
    monitor: &Monitor,
    alto_extra: u32,
    id: u32,
    instancia_win: HINSTANCE,
    instancia_gpu: &wgpu::Instance,
    a_render: &Sender<ARender>,
) -> Result<Arc<EstadoVentana>, String> {
    let es_ventana = p.ventana.is_some();
    let alto = p.alto + if cual == 0 { alto_extra } else { 0 };
    let colocacion = (!es_ventana).then(|| Colocacion {
        monitor: monitor.rect,
        ancla: p.ancla,
        margen: p.margen,
        ancho: p.ancho,
        alto,
        reserva: p.reserva,
        nivel: p.nivel,
    });
    let (r, tam_logico) = if let Some(c) = &colocacion {
        rect_panel(c, monitor.escala)
    } else {
        let w = p.ancho.max(320);
        let h = alto.max(200);
        let mut r = RECT {
            left: 0,
            top: 0,
            right: px(w as i32, monitor.escala),
            bottom: px(h as i32, monitor.escala),
        };
        let _ = unsafe {
            AdjustWindowRectExForDpi(
                &mut r,
                WS_OVERLAPPEDWINDOW,
                false,
                WINDOW_EX_STYLE::default(),
                (monitor.escala * 96.0).round() as u32,
            )
        };
        let ancho = r.right - r.left;
        let alto = r.bottom - r.top;
        let x = monitor.rect.left + (monitor.rect.right - monitor.rect.left - ancho) / 2;
        let y = monitor.rect.top + (monitor.rect.bottom - monitor.rect.top - alto) / 2;
        (
            RECT {
                left: x,
                top: y,
                right: x + ancho,
                bottom: y + alto,
            },
            (w, h),
        )
    };
    let teclado_inicial = if p.teclado_mientras {
        Teclado::Nunca
    } else {
        p.teclado
    };
    let estado = Arc::new(EstadoVentana {
        id,
        cual,
        hwnd: AtomicIsize::new(0),
        a_render: a_render.clone(),
        origen: p.origen,
        escala: AtomicU32::new(monitor.escala.to_bits()),
        cajas: Mutex::default(),
        cursor: AtomicU8::new(0),
        teclado: AtomicU8::new(teclado_num(teclado_inicial)),
        raton_dentro: AtomicBool::new(false),
        derecho_cierra: p.derecho_cierra,
        es_ventana,
        colocacion: Mutex::new(colocacion),
        appbar: AtomicBool::new(false),
        fuera: AtomicBool::new(false),
    });
    let titulo: Vec<u16> = p
        .ventana
        .as_deref()
        .filter(|t| !t.is_empty())
        .unwrap_or("pleamar")
        .encode_utf16()
        .chain([0])
        .collect();
    let estilo = if es_ventana {
        WS_OVERLAPPEDWINDOW
    } else {
        WS_POPUP
    };
    let mut ex = if es_ventana {
        WS_EX_APPWINDOW
    } else {
        WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP
    };
    if !es_ventana && teclado_inicial == Teclado::Nunca {
        ex |= WS_EX_NOACTIVATE;
    }
    if !es_ventana && matches!(p.nivel, Nivel::Encima | Nivel::SobreTodo) {
        ex |= WS_EX_TOPMOST;
    }
    let raw = Arc::into_raw(estado.clone()) as *const c_void;
    let hwnd = unsafe {
        CreateWindowExW(
            ex,
            w!("PleamarWindow"),
            PCWSTR(titulo.as_ptr()),
            estilo,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            None,
            None,
            Some(instancia_win),
            Some(raw),
        )
    }
    .map_err(|e| format!("Win32 could not create a pleamar window: {e}"))?;
    estado.hwnd.store(hwnd.0 as isize, Ordering::Relaxed);
    if !es_ventana {
        unsafe { recolocar(&estado) };
    }
    unsafe {
        let _ = ShowWindow(
            hwnd,
            if es_ventana {
                SW_SHOW
            } else {
                SW_SHOWNOACTIVATE
            },
        );
    }
    let mut handle = Win32WindowHandle::new(NonZeroIsize::new(hwnd.0 as isize).unwrap());
    handle.hinstance = NonZeroIsize::new(instancia_win.0 as isize);
    let superficie = unsafe {
        instancia_gpu.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(RawDisplayHandle::Windows(WindowsDisplayHandle::new())),
            raw_window_handle: RawWindowHandle::Win32(handle),
        })
    }
    .map_err(|e| format!("wgpu could not create the DirectComposition surface: {e}"))?;
    let _ = a_render.send(ARender::Lamina(Box::new(gpu::NuevaLamina {
        id,
        superficie,
        ventana: Box::new(VentanaWindows(estado.clone())),
        escala: monitor.escala,
        tam: tam_logico,
        mhz: monitor.mhz,
        nombre: monitor.nombre.clone(),
        vista: gpu::Vista {
            superficie: cual,
            emergente: None,
            origen: p.origen,
            tam: (tam_logico.0 as f32, tam_logico.1 as f32),
        },
    })));
    Ok(estado)
}

/// Creates every requested surface and owns the Win32 message loop.
pub fn atender(
    pide: Vec<Superficie>,
    alto_extra: u32,
    instancia: wgpu::Instance,
    a_render: Sender<ARender>,
) {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let modulo = match unsafe { GetModuleHandleW(None) } {
        Ok(m) => HINSTANCE(m.0),
        Err(e) => {
            eprintln!("windows · cannot get this process module: {e}");
            return;
        }
    };
    if let Err(e) = registrar_clase(modulo) {
        eprintln!("windows · {e}");
        return;
    }
    let pantallas = monitores();
    if pantallas.is_empty() {
        eprintln!("windows · there are no monitors");
        return;
    }
    let plataforma = PLATAFORMA.get_or_init(|| Plataforma {
        ventanas: Mutex::default(),
    });
    let mut vivas: Vec<Arc<EstadoVentana>> = Vec::new();
    let mut siguiente_id = 0u32;
    for (cual, p) in pide.iter().enumerate() {
        for (numero, monitor) in pantallas.iter().enumerate() {
            let cuantas = quiere_en(p, &monitor.nombre, numero);
            if p.ventana.is_some() && !vivas.iter().all(|v| v.cual != cual) {
                continue;
            }
            for _ in 0..cuantas {
                match crear(
                    p,
                    cual,
                    monitor,
                    alto_extra,
                    siguiente_id,
                    modulo,
                    &instancia,
                    &a_render,
                ) {
                    Ok(v) => {
                        plataforma.ventanas.lock().unwrap().push(Arc::downgrade(&v));
                        vivas.push(v);
                        siguiente_id += 1;
                    }
                    Err(e) => eprintln!("windows · {e}"),
                }
            }
        }
    }
    if vivas.is_empty() {
        eprintln!(
            "windows · none of the requested monitors ({:?}) is present",
            pide.iter().map(|s| &s.pantallas).collect::<Vec<_>>()
        );
        return;
    }
    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    for v in &vivas {
        unsafe {
            quitar_appbar(v);
            let _ = DestroyWindow(v.hwnd());
        }
        v.se_fue();
    }
}

pub fn anclar(cual: usize, ancla: Ancla) {
    let Some(p) = PLATAFORMA.get() else { return };
    let mut ventanas = p.ventanas.lock().unwrap();
    ventanas.retain(|v| v.strong_count() > 0);
    for v in ventanas
        .iter()
        .filter_map(Weak::upgrade)
        .filter(|v| v.cual == cual)
    {
        if let Some(c) = v.colocacion.lock().unwrap().as_mut() {
            c.ancla = ancla;
        }
        unsafe {
            let _ = PostMessageW(Some(v.hwnd()), WM_PLEAMAR_RECOLOCAR, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn pedir_salir() {
    if let Some(p) = PLATAFORMA.get() {
        if let Some(v) = p.ventanas.lock().unwrap().iter().find_map(Weak::upgrade) {
            unsafe {
                let _ = PostMessageW(Some(v.hwnd()), WM_PLEAMAR_SALIR, WPARAM(0), LPARAM(0));
            }
        }
    }
}

/// Popup surfaces need their own HWND and lifetime handshake with the render
/// thread. The base Windows backend deliberately leaves them unavailable until
/// that handshake is implemented; ordinary and named surfaces already work.
pub fn emergente(_: usize, _: Option<([i32; 4], (f32, f32))>) {
    static AVISADO: AtomicBool = AtomicBool::new(false);
    if !AVISADO.swap(true, Ordering::Relaxed) {
        eprintln!("popup  · popup surfaces are not implemented on Windows yet");
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn colocacion(ancla: Ancla, ancho: u32, margen: [i32; 4]) -> Colocacion {
        Colocacion {
            monitor: RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            ancla,
            margen,
            ancho,
            alto: 72,
            reserva: 0,
            nivel: Nivel::Encima,
        }
    }

    #[test]
    fn todo_el_ancho_respeta_margenes_y_escala() {
        let (r, tam) = rect_panel(&colocacion(Ancla::Abajo, 0, [8, 12, 10, 14]), 1.25);
        assert_eq!((r.left, r.top, r.right, r.bottom), (18, 977, 1905, 1067));
        assert_eq!(tam, (1510, 72));
    }

    #[test]
    fn ancho_fijo_se_centra_en_el_area_disponible() {
        let (r, tam) = rect_panel(&colocacion(Ancla::Arriba, 400, [8, 12, 10, 14]), 1.25);
        assert_eq!((r.left, r.top, r.right, r.bottom), (711, 10, 1211, 100));
        assert_eq!(tam, (400, 72));
    }
}
