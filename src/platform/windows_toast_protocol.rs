//! A per-installation URI only carries an opaque token to the native broker.
use std::{path::Path, time::Duration};
use windows::{core::PCWSTR, Win32::{Foundation::*, System::Registry::*}};
use super::windows_toast_activation as activation;

fn wide(text:&str)->Vec<u16> {text.encode_utf16().chain([0]).collect()}
pub(super) fn scheme(id:&str)->String {format!("pleamar-notify-{:016x}",u128::from(activation::clsid(id)) as u64)}
fn key(id:&str)->String {format!("Software\\Classes\\{}",scheme(id))}
fn command(engine:&Path)->Result<String,String> {
    let broker=engine.with_file_name("pleamar-notifications.exe");
    let broker=broker.to_str().ok_or("notification broker path is not Unicode")?;
    if broker.contains(['"','\0']) {return Err("invalid notification broker path".into());}
    Ok(format!("\"{broker}\" --activate-notification \"%1\""))
}
fn read(key:&str,name:&str)->Result<Option<String>,String> {unsafe {
    let key=wide(key);let name=wide(name);let mut buf=vec![0u16;32768];let mut bytes=(buf.len()*2) as u32;
    let status=RegGetValueW(HKEY_CURRENT_USER,PCWSTR(key.as_ptr()),PCWSTR(name.as_ptr()),RRF_RT_REG_SZ,None,Some(buf.as_mut_ptr().cast()),Some(&mut bytes));
    if status==ERROR_FILE_NOT_FOUND || status==ERROR_PATH_NOT_FOUND {return Ok(None);}
    status.ok().map_err(|e|e.to_string())?;
    let end=buf.iter().position(|c|*c==0).ok_or("unterminated notification protocol value")?;
    String::from_utf16(&buf[..end]).map(Some).map_err(|e|e.to_string())
}}
fn write(key:&str,name:&str,value:&str)->Result<(),String> {
    let key=wide(key);let name=wide(name);let value=wide(value);
    unsafe {RegSetKeyValueW(HKEY_CURRENT_USER,PCWSTR(key.as_ptr()),PCWSTR(name.as_ptr()),REG_SZ.0,Some(value.as_ptr().cast()),(value.len()*2) as u32)}.ok().map_err(|e|e.to_string())
}
pub(super) fn ready(engine:&Path,id:&str)->bool {
    let root=key(id);
    command(engine).ok().is_some_and(|want|read(&format!("{root}\\shell\\open\\command"),"").ok().flatten()==Some(want))
        && read(&root,"URL Protocol").ok().flatten().as_deref()==Some("")
}
pub(super) fn register(engine:&Path,id:&str)->Result<(),String> {
    let root=key(id);let leaf=format!("{root}\\shell\\open\\command");let want=command(engine)?;
    if read(&leaf,"")?.is_some_and(|old|old!=want) {return Err("another application owns the notification protocol".into());}
    // No browser, script host or command shell participates in activation.
    write(&leaf,"",&want)?;
    write(&root,"URL Protocol","")?;
    Ok(())
}
fn remove_value(key:&str,name:&str)->Result<(),String> {
    let key=wide(key);let name=wide(name);
    let status=unsafe {RegDeleteKeyValueW(HKEY_CURRENT_USER,PCWSTR(key.as_ptr()),PCWSTR(name.as_ptr()))};
    if status==ERROR_FILE_NOT_FOUND || status==ERROR_PATH_NOT_FOUND {return Ok(());}
    status.ok().map_err(|e|e.to_string())
}
fn remove_empty(key:&str) {unsafe {
    let path=wide(key);let mut handle=HKEY::default();
    if RegOpenKeyExW(HKEY_CURRENT_USER,PCWSTR(path.as_ptr()),None,KEY_READ,&mut handle).is_err() {return;}
    let (mut subkeys,mut values)=(0,0);
    let result=RegQueryInfoKeyW(handle,None,None,None,Some(&mut subkeys),None,None,Some(&mut values),None,None,None,None);
    let _=RegCloseKey(handle);
    if result.is_ok() && subkeys==0 && values==0 {let _=RegDeleteKeyW(HKEY_CURRENT_USER,PCWSTR(path.as_ptr()));}
}}
pub(super) fn unregister(engine:&Path,id:&str)->Result<(),String> {
    let root=key(id);let leaf=format!("{root}\\shell\\open\\command");
    if read(&leaf,"")?.as_ref()!=Some(&command(engine)?) {return Ok(());}
    remove_value(&leaf,"")?;
    if read(&root,"URL Protocol")?.as_deref()==Some("") {remove_value(&root,"URL Protocol")?;}
    // Preserve unrelated values/subkeys even inside our installation's scheme.
    for suffix in ["\\shell\\open\\command","\\shell\\open","\\shell",""] {remove_empty(&format!("{root}{suffix}"));}
    Ok(())
}
pub(super) fn uri(id:&str,token:&str)->String {format!("{}:{token}",scheme(id))}
fn token<'a>(id:&str,uri:&'a str)->Result<&'a str,String> {
    let value=uri.strip_prefix(&format!("{}:",scheme(id))).ok_or("notification protocol does not match this installation")?;
    activation::route(value).ok_or("invalid notification activation token")?;
    Ok(value)
}
pub(super) fn activate(id:&str,uri:&str)->Result<(),String> {
    let value=token(id,uri)?;let route=activation::route(value).unwrap();
    // An expired endpoint is intentionally inert, including after scene exit.
    let _=super::windows_ipc::ask_path(&activation::pipe(id,route),value,Duration::from_millis(750));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_accepts_only_its_exact_opaque_token() {
        let value=format!("v1.{}.{}",activation::random().unwrap(),activation::random().unwrap());
        let valid=uri("owned",&value);assert_eq!(token("owned",&valid).unwrap(),value);
        assert!(token("other",&valid).is_err());
        for bad in [format!("{valid}/"),format!("{valid}?run=anything"),format!("{valid}\" --other"),format!("{valid}%00"),valid.replace("v1.","v1%2e"),format!("{}://{value}",scheme("owned"))] {assert!(token("owned",&bad).is_err());}
        assert_eq!(command(Path::new(r"C:\Marea ñ 海\pleamar.exe")).unwrap(),r#""C:\Marea ñ 海\pleamar-notifications.exe" --activate-notification "%1""#);
    }
}
