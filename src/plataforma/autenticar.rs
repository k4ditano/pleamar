//! Comprobar que quien está sentado es quien dice: la contraseña de la sesión,
//! que es lo que necesita una pantalla de bloqueo para poder abrirse.
//!
//! En Linux es PAM, el mismo camino que usa `login`: la cuenta la comprueba el
//! sistema —con su ayudante con permisos, `unix_chkpwd`— y aquí solo se hace de
//! mensajero. Va con un FFI escrito a mano y no con un crate: son cuatro
//! funciones de una biblioteca que está en todas las máquinas, y así pleamar no
//! gana una dependencia que compile C por una pregunta de sí o no.
//!
//! En Windows será `LogonUser`; en macOS, `SecKeychainUnlock` o la autorización
//! de `LocalAuthentication`.
//!
//! Tarda: una contraseña mala se paga con un par de segundos de espera, que
//! pone PAM a propósito. Se llama desde la lógica, que para eso puede esperar;
//! el render no se entera.

use libc::{c_char, c_int, c_void};
use std::ffi::CString;

#[repr(C)]
struct Mensaje {
    estilo: c_int,
    texto: *const c_char,
}
#[repr(C)]
struct Respuesta {
    texto: *mut c_char,
    codigo: c_int,
}
#[repr(C)]
struct Conversacion {
    habla: extern "C" fn(c_int, *const *const Mensaje, *mut *mut Respuesta, *mut c_void) -> c_int,
    datos: *mut c_void,
}

#[link(name = "pam")]
unsafe extern "C" {
    fn pam_start(servicio: *const c_char, usuario: *const c_char, conv: *const Conversacion, asa: *mut *mut c_void) -> c_int;
    fn pam_authenticate(asa: *mut c_void, banderas: c_int) -> c_int;
    fn pam_acct_mgmt(asa: *mut c_void, banderas: c_int) -> c_int;
    fn pam_end(asa: *mut c_void, estado: c_int) -> c_int;
}

const BIEN: c_int = 0;
const SIN_MEMORIA: c_int = 5;
const PIDE_SIN_ECO: c_int = 1;
const PIDE_CON_ECO: c_int = 2;

/// Lo que PAM pregunte, se le contesta con la contraseña: no hay más que decir.
/// Las respuestas las libera él, así que van con el `malloc` de C.
extern "C" fn habla(cuantos: c_int, mensajes: *const *const Mensaje, respuestas: *mut *mut Respuesta, datos: *mut c_void) -> c_int {
    unsafe {
        let r = libc::calloc(cuantos.max(1) as usize, std::mem::size_of::<Respuesta>()) as *mut Respuesta;
        if r.is_null() {
            return SIN_MEMORIA;
        }
        for k in 0..cuantos as usize {
            let m = *mensajes.add(k);
            if !m.is_null() && matches!((*m).estilo, PIDE_SIN_ECO | PIDE_CON_ECO) {
                (*r.add(k)).texto = libc::strdup(datos as *const c_char);
            }
        }
        *respuestas = r;
    }
    BIEN
}

/// Quién está sentado: el dueño del proceso, que es a quien se le pregunta.
fn usuario() -> Option<CString> {
    unsafe {
        let p = libc::getpwuid(libc::getuid());
        if p.is_null() || (*p).pw_name.is_null() {
            return None;
        }
        Some(std::ffi::CStr::from_ptr((*p).pw_name).to_owned())
    }
}

/// `true` si esa es la contraseña de quien tiene la sesión.
pub fn comprobar(clave: &str) -> Result<bool, String> {
    let usuario = usuario().ok_or("I cannot tell who owns this session")?;
    let clave = CString::new(clave).map_err(|_| "a password cannot carry a zero byte")?;
    // Con fichero propio en /etc/pam.d, el suyo; si no, el de `login`, que está siempre.
    let servicio = CString::new(if std::path::Path::new("/etc/pam.d/pleamar").exists() { "pleamar" } else { "login" }).unwrap();
    let conv = Conversacion { habla, datos: clave.as_ptr() as *mut c_void };
    let mut asa: *mut c_void = std::ptr::null_mut();
    let vale = unsafe {
        let r = pam_start(servicio.as_ptr(), usuario.as_ptr(), &conv, &mut asa);
        if r != BIEN {
            return Err(format!("PAM would not start (code {r})"));
        }
        let mut r = pam_authenticate(asa, 0);
        if r == BIEN {
            // Una cuenta caducada o cerrada no entra aunque sepa su contraseña.
            r = pam_acct_mgmt(asa, 0);
        }
        pam_end(asa, r);
        r == BIEN
    };
    // Lo que quedaba de ella en memoria, a cero antes de soltarlo.
    let mut bytes = clave.into_bytes();
    bytes.iter_mut().for_each(|b| *b = 0);
    Ok(vale)
}
