//! Per-scene generic credentials. Values never have a query that returns them
//! to scene logic: desktop input consumes them inside the native service.
use super::SysValue;
use windows::{core::{PCWSTR, PWSTR}, Win32::{Foundation::ERROR_NOT_FOUND, Security::Credentials::*}};

fn hex(text: &str) -> String { text.as_bytes().iter().map(|b| format!("{b:02x}")).collect() }
fn prefix(owner: &str) -> Result<String, String> {
    if owner.is_empty() || owner.len() > 256 { return Err("invalid credential owner".into()); }
    Ok(format!("pleamar:scene:{}:", hex(owner)))
}
fn target(owner: &str, name: &str) -> Result<String, String> {
    if name.trim().is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
        return Err("credential names need 1–60 characters without control characters".into());
    }
    Ok(prefix(owner)? + &hex(&name.to_lowercase()))
}
fn wide(text: &str) -> Vec<u16> { text.encode_utf16().chain(Some(0)).collect() }
fn failure(error: windows::core::Error) -> String { format!("Windows credential operation failed ({})", error.code()) }
unsafe fn wipe(credential: *mut CREDENTIALW) {
    unsafe {
        if credential.is_null() { return; }
        let c = &*credential;
        if !c.CredentialBlob.is_null() {
            for i in 0..c.CredentialBlobSize as usize { c.CredentialBlob.add(i).write_volatile(0); }
        }
    }
}
struct Credential(*mut CREDENTIALW);
impl Drop for Credential { fn drop(&mut self) { unsafe { wipe(self.0); CredFree(self.0 as _); } } }
struct Credentials { values: *mut *mut CREDENTIALW, count: u32 }
impl Drop for Credentials { fn drop(&mut self) { unsafe {
    if self.values.is_null() { return; }
    for i in 0..self.count as usize { wipe(*self.values.add(i)); }
    CredFree(self.values as _);
} } }

fn names(owner: &str) -> Result<Vec<String>, String> {
    let prefix = prefix(owner)?;
    let filter = wide(&(prefix.clone() + "*"));
    let mut list = Credentials { values: std::ptr::null_mut(), count: 0 };
    if let Err(e) = unsafe { CredEnumerateW(PCWSTR(filter.as_ptr()), None, &mut list.count, &mut list.values) } {
        return if e.code() == ERROR_NOT_FOUND.to_hresult() { Ok(Vec::new()) } else { Err(failure(e)) };
    }
    let mut names = Vec::new();
    for i in 0..list.count as usize {
        let c = unsafe { &**list.values.add(i) };
        if c.Type != CRED_TYPE_GENERIC || c.UserName.is_null() || c.TargetName.is_null() { continue; }
        let name = unsafe { c.UserName.to_string() }.map_err(|_| "invalid credential label")?;
        let key = unsafe { c.TargetName.to_string() }.map_err(|_| "invalid credential target")?;
        if target(owner, &name).is_ok_and(|t| t == key) { names.push(name); }
    }
    names.sort_by_key(|s| s.to_lowercase());
    Ok(names)
}

pub fn query(owner: &str, name: &str, args: &[SysValue]) -> Result<SysValue, String> {
    match (name, args) {
        ("credentials.list", []) => Ok(SysValue::List(names(owner)?.into_iter().map(SysValue::Text).collect())),
        _ => Err("credentials only exposes names through credentials.list".into()),
    }
}
pub fn command(owner: &str, command: &str, args: &[SysValue]) -> Result<(), String> {
    match (command, args) {
        ("credentials.set", [SysValue::Text(name), SysValue::Text(value)]) => {
            let mut key = wide(&target(owner, name)?);
            if value.is_empty() || value.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize || value.contains('\0') {
                return Err("a credential needs 1–2560 UTF-8 bytes without NUL".into());
            }
            let mut label = wide(name);
            let c = CREDENTIALW { Type: CRED_TYPE_GENERIC, TargetName: PWSTR(key.as_mut_ptr()),
                UserName: PWSTR(label.as_mut_ptr()), CredentialBlob: value.as_ptr() as _,
                CredentialBlobSize: value.len() as u32, Persist: CRED_PERSIST_LOCAL_MACHINE, ..Default::default() };
            unsafe { CredWriteW(&c, 0) }.map_err(failure)
        }
        ("credentials.remove", [SysValue::Text(name)]) => {
            let key = wide(&target(owner, name)?);
            match unsafe { CredDeleteW(PCWSTR(key.as_ptr()), CRED_TYPE_GENERIC, None) } {
                Err(e) if e.code() == ERROR_NOT_FOUND.to_hresult() => Ok(()),
                result => result.map_err(failure),
            }
        }
        _ => Err("use credentials.set(name, value) or credentials.remove(name)".into()),
    }
}

pub(super) fn with_secret<T>(owner: &str, name: &str, use_value: impl FnOnce(&str) -> Result<T, String>) -> Result<T, String> {
    let key = wide(&target(owner, name)?);
    let mut value = std::ptr::null_mut();
    unsafe { CredReadW(PCWSTR(key.as_ptr()), CRED_TYPE_GENERIC, None, &mut value) }.map_err(failure)?;
    if value.is_null() { return Err("Windows returned no saved credential".into()); }
    let credential = Credential(value);
    let c = unsafe { &*credential.0 };
    if c.CredentialBlob.is_null() || c.CredentialBlobSize == 0 || c.CredentialBlobSize > CRED_MAX_CREDENTIAL_BLOB_SIZE {
        return Err("the saved credential is not supported".into());
    }
    let bytes = unsafe { std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize) };
    let text = std::str::from_utf8(bytes).map_err(|_| "the saved credential is not UTF-8")?;
    if text.contains('\0') { return Err("the saved credential contains NUL".into()); }
    use_value(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_are_scoped_and_values_cannot_be_queried() {
        assert_ne!(target("a:b", "c").unwrap(), target("a", "b:c").unwrap());
        assert_eq!(target("a", "Café").unwrap(), target("a", "CAFÉ").unwrap());
        for name in ["", " ", "line\nname", "nul\0"] { assert!(target("a", name).is_err()); }
        assert!(query("a", "credentials.read", &[SysValue::Text("any".into())]).is_err());
        assert!(command("a", "credentials.set", &[SysValue::Text("long".into()), SysValue::Text("x".repeat(2561))]).is_err());
    }
    #[test]
    #[ignore = "writes only uniquely scoped dummy credentials to the current Windows user's vault"]
    fn native_scoped_credential_roundtrip() {
        let owner = format!("credential-test-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
        let name = "Prueba café 日本語";
        struct Cleanup(String, String);
        impl Drop for Cleanup { fn drop(&mut self) { let _ = command(&self.0, "credentials.remove", &[SysValue::Text(self.1.clone())]); } }
        let _cleanup = Cleanup(owner.clone(), name.into());
        let set = |value: &str| command(&owner, "credentials.set", &[SysValue::Text(name.into()), SysValue::Text(value.into())]).unwrap();
        set("dummy αβ café 🪼");
        assert_eq!(names(&owner).unwrap(), [name]);
        assert!(names(&(owner.clone()+"-other")).unwrap().is_empty());
        with_secret(&owner, name, |v| { assert!(v == "dummy αβ café 🪼"); Ok(()) }).unwrap();
        set("changed dummy");
        with_secret(&owner, &name.to_uppercase(), |v| { assert!(v == "changed dummy"); Ok(()) }).unwrap();
        command(&owner, "credentials.remove", &[SysValue::Text(name.into())]).unwrap();
        assert!(names(&owner).unwrap().is_empty());
        assert!(with_secret(&owner, name, |_| Ok(())).is_err());
    }
}
