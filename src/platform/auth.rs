//! Checking that whoever is sitting there is who they say: the session's password,
//! which is what a lock screen needs in order to open.
//!
//! On Linux it is PAM, the same path `login` uses: the system checks the account
//! —with its privileged helper, `unix_chkpwd`— and here we only act as the
//! messenger. It goes through a hand-written FFI rather than a crate: it is four
//! functions from a library that is on every machine, and this way pleamar does
//! not gain a dependency that compiles C for a yes-or-no question.
//!
//! On Windows it will be `LogonUser`; on macOS, `SecKeychainUnlock` or the
//! `LocalAuthentication` authorization.
//!
//! It is slow: a wrong password costs a couple of seconds of waiting, which PAM
//! adds on purpose. It is called from the logic, which can afford to wait; the
//! render never notices.

use libc::{c_char, c_int, c_void};
use std::ffi::CString;

#[repr(C)]
struct Message {
    style: c_int,
    text: *const c_char,
}
#[repr(C)]
struct Response {
    text: *mut c_char,
    code: c_int,
}
#[repr(C)]
struct Conversation {
    converse: extern "C" fn(c_int, *const *const Message, *mut *mut Response, *mut c_void) -> c_int,
    data: *mut c_void,
}

#[link(name = "pam")]
unsafe extern "C" {
    fn pam_start(service: *const c_char, user: *const c_char, conv: *const Conversation, handle: *mut *mut c_void) -> c_int;
    fn pam_authenticate(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_acct_mgmt(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_end(handle: *mut c_void, status: c_int) -> c_int;
}

const SUCCESS: c_int = 0;
const BUF_ERR: c_int = 5;
const PROMPT_ECHO_OFF: c_int = 1;
const PROMPT_ECHO_ON: c_int = 2;

/// Whatever PAM asks, it gets the password as the answer: there is nothing else to say.
/// It frees the responses itself, so they go with C's `malloc`.
extern "C" fn converse(count: c_int, messages: *const *const Message, responses: *mut *mut Response, data: *mut c_void) -> c_int {
    unsafe {
        let r = libc::calloc(count.max(1) as usize, std::mem::size_of::<Response>()) as *mut Response;
        if r.is_null() {
            return BUF_ERR;
        }
        for k in 0..count as usize {
            let m = *messages.add(k);
            if !m.is_null() && matches!((*m).style, PROMPT_ECHO_OFF | PROMPT_ECHO_ON) {
                (*r.add(k)).text = libc::strdup(data as *const c_char);
            }
        }
        *responses = r;
    }
    SUCCESS
}

/// Who is sitting there: the owner of the process, who is the one being asked.
fn user() -> Option<CString> {
    unsafe {
        let p = libc::getpwuid(libc::getuid());
        if p.is_null() || (*p).pw_name.is_null() {
            return None;
        }
        Some(std::ffi::CStr::from_ptr((*p).pw_name).to_owned())
    }
}

/// `true` if that is the password of whoever holds the session.
pub fn verify(password: &str) -> Result<bool, String> {
    let user = user().ok_or("I cannot tell who owns this session")?;
    let password = CString::new(password).map_err(|_| "a password cannot carry a zero byte")?;
    // With its own file in /etc/pam.d, that one; otherwise `login`'s, which is always there.
    let service = CString::new(if std::path::Path::new("/etc/pam.d/pleamar").exists() { "pleamar" } else { "login" }).unwrap();
    let conv = Conversation { converse, data: password.as_ptr() as *mut c_void };
    let mut handle: *mut c_void = std::ptr::null_mut();
    let ok = unsafe {
        let r = pam_start(service.as_ptr(), user.as_ptr(), &conv, &mut handle);
        if r != SUCCESS {
            return Err(format!("PAM would not start (code {r})"));
        }
        let mut r = pam_authenticate(handle, 0);
        if r == SUCCESS {
            // An expired or locked account does not get in even if it knows its password.
            r = pam_acct_mgmt(handle, 0);
        }
        pam_end(handle, r);
        r == SUCCESS
    };
    // Whatever was left of it in memory, zeroed before letting it go.
    let mut bytes = password.into_bytes();
    bytes.iter_mut().for_each(|b| *b = 0);
    Ok(ok)
}
