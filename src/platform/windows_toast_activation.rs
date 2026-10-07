//! COM activation only forwards opaque, process-scoped tokens to a live scene.
//! An activation after that process exits never starts Marea or a saved task.
use std::{path::Path, sync::{Arc, atomic::{AtomicU64, Ordering}}, time::{Duration, Instant}};
use windows::{core::{self as windows_core, implement, Interface, Ref, GUID, IUnknown, PCWSTR, BOOL},
    Win32::{Foundation::*, System::{Com::*, Registry::*}, UI::Notifications::*}};

pub(super) fn clsid(id: &str) -> GUID {
    let hash = id.bytes().fold(0xcbf29ce484222325u64, |h,b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    GUID::from_u128(0x61e6ee1c_8b1a_4175_0000_000000000000 | hash as u128)
}
pub(super) fn random() -> Result<String,String> {
    unsafe { CoCreateGuid() }.map(|id| format!("{:032x}", u128::from(id))).map_err(|e|e.to_string())
}
fn wide(text: &str) -> Vec<u16> { text.encode_utf16().chain([0]).collect() }
fn key(id:&str) -> String { format!("Software\\Classes\\CLSID\\{{{:?}}}\\LocalServer32", clsid(id)) }
fn broker(engine:&Path) -> Result<String,String> {
    let path=engine.with_file_name("pleamar-notifications.exe");
    let path=path.to_str().ok_or("notification broker path is not Unicode")?;
    if path.contains(['"','\0']) {return Err("invalid notification broker path".into());}
    Ok(format!("\"{path}\""))
}
fn registered(id:&str) -> Result<Option<String>,String> { unsafe {
    let key=wide(&key(id));let mut buffer=vec![0u16;32768];let mut bytes=(buffer.len()*2) as u32;
    let status=RegGetValueW(HKEY_CURRENT_USER,PCWSTR(key.as_ptr()),PCWSTR::null(),RRF_RT_REG_SZ,None,
        Some(buffer.as_mut_ptr().cast()),Some(&mut bytes));
    if status==ERROR_FILE_NOT_FOUND || status==ERROR_PATH_NOT_FOUND {return Ok(None);}
    status.ok().map_err(|e|e.to_string())?;
    let end=buffer.iter().position(|c|*c==0).ok_or("unterminated notification registration")?;
    String::from_utf16(&buffer[..end]).map(Some).map_err(|e|e.to_string())
} }
pub(super) fn ready(engine:&Path,id:&str)->bool {
    engine.with_file_name("pleamar-notifications.exe").is_file()
        && broker(engine).ok().is_some_and(|want|registered(id).ok().flatten().as_ref()==Some(&want))
        && super::windows_toast_protocol::ready(engine,id)
}
pub(super) fn register(engine:&Path,id:&str)->Result<(),String> {
    if !engine.with_file_name("pleamar-notifications.exe").is_file() {return Err("the package is missing pleamar-notifications.exe".into());}
    let value=broker(engine)?;
    if registered(id)?.is_some_and(|old|old!=value) {return Err("another server owns the notification registration".into());}
    let key=wide(&key(id));let value=wide(&value);
    unsafe {RegSetKeyValueW(HKEY_CURRENT_USER,PCWSTR(key.as_ptr()),PCWSTR::null(),REG_SZ.0,
        Some(value.as_ptr().cast()),(value.len()*2) as u32)}.ok().map_err(|e|e.to_string())?;
    super::windows_toast_protocol::register(engine,id)
}
pub(super) fn unregister(engine:&Path,id:&str)->Result<(),String> {
    super::windows_toast_protocol::unregister(engine,id)?;
    if registered(id)?.as_ref()!=Some(&broker(engine)?) {return Ok(());}
    // Delete only our exact server key, preserving other values/subkeys on CLSID.
    let path=wide(&key(id));
    unsafe {RegDeleteTreeW(HKEY_CURRENT_USER,PCWSTR(path.as_ptr()))}.ok().map_err(|e|e.to_string())
}
pub(super) fn route(token:&str)->Option<&str> {
    let rest=token.strip_prefix("v1.")?;
    let (route, action)=rest.split_once('.')?;
    (route.len()==32 && action.len()==32 && route.bytes().chain(action.bytes()).all(|c|c.is_ascii_hexdigit() && !c.is_ascii_uppercase())).then_some(route)
}
pub(super) fn pipe(id:&str,route:&str)->String {format!(r"\\.\pipe\pleamar-toast-{id}-{route}")}
unsafe fn bounded(value:PCWSTR,limit:usize)->windows_core::Result<String> {
    if value.is_null() {return Err(E_INVALIDARG.into());}
    for count in 0..=limit {
        if unsafe {*value.0.add(count)}==0 {
            return String::from_utf16(unsafe {std::slice::from_raw_parts(value.0,count)}).map_err(|_|E_INVALIDARG.into());
        }
    }
    Err(E_INVALIDARG.into())
}

#[implement(INotificationActivationCallback)]
struct Activation { id:String, delivered:Arc<AtomicU64> }
impl INotificationActivationCallback_Impl for Activation_Impl {
    fn Activate(&self,id:&PCWSTR,args:&PCWSTR,_:*const NOTIFICATION_USER_INPUT_DATA,count:u32)->windows_core::Result<()> {
        // No input boxes or commands are accepted. COM supplies valid terminated strings.
        if count!=0 || id.is_null() || args.is_null() {return Err(E_INVALIDARG.into());}
        let id=unsafe {bounded(*id,128)}?;let token=unsafe {bounded(*args,68)}?;
        if id!=self.id {return Err(E_ACCESSDENIED.into());}
        let route=route(&token).ok_or(E_INVALIDARG)?;
        // Several scenes can register this class. The random endpoint routes the
        // click to its originating process regardless of which COM server won.
        let _=super::windows_ipc::ask_path(&pipe(&id,route),&token,Duration::from_millis(750));
        self.delivered.fetch_add(1,Ordering::Release);
        Ok(())
    }
}
#[implement(IClassFactory)]
struct Factory { id:String, delivered:Arc<AtomicU64> }
impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self,outer:Ref<IUnknown>,iid:*const GUID,out:*mut *mut std::ffi::c_void)->windows_core::Result<()> {
        if out.is_null() || iid.is_null() {return Err(E_POINTER.into());}
        unsafe {*out=std::ptr::null_mut();}
        if outer.is_some() {return Err(CLASS_E_NOAGGREGATION.into());}
        let callback:INotificationActivationCallback=Activation{id:self.id.clone(),delivered:self.delivered.clone()}.into();
        unsafe {callback.query(iid,out).ok()}
    }
    fn LockServer(&self,_:BOOL)->windows_core::Result<()> {Ok(())}
}
pub(super) struct Registration(u32);
impl Registration {
    pub fn new(id:&str, class:GUID, delivered:Arc<AtomicU64>)->Result<Self,String> {
        let factory:IClassFactory=Factory{id:id.into(),delivered}.into();
        unsafe {CoRegisterClassObject(&class,&factory,CLSCTX_LOCAL_SERVER,REGCLS_MULTIPLEUSE)}.map(Self).map_err(|e|e.to_string())
    }
}
impl Drop for Registration {fn drop(&mut self){unsafe {let _=CoRevokeClassObject(self.0);}}}

pub(crate) fn run_broker()->Result<(),String> {
    let engine=std::env::current_exe().map_err(|e|e.to_string())?.with_file_name("pleamar.exe");
    let id=super::windows_toasts::identity_for(&engine);
    let args=std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|arg|arg=="--activate-notification") {
        if args.len()!=2 {return Err("notification activation requires exactly one URI".into());}
        return super::windows_toast_protocol::activate(&id,&args[1]);
    }
    if !args.is_empty() && !(args.len()==1 && args[0].eq_ignore_ascii_case("-embedding")) {
        return Err("unrecognized notification broker arguments".into());
    }
    let _apartment=super::windows_system::Apartment::new()?;
    let calls=Arc::new(AtomicU64::new(0));
    let _class=Registration::new(&id,clsid(&id),calls.clone())?;
    let started=Instant::now();let mut last=(0,Instant::now());
    while started.elapsed()<Duration::from_secs(15) {
        let count=calls.load(Ordering::Acquire);
        if count!=last.0 {last=(count,Instant::now());}
        if count>0 && last.1.elapsed()>Duration::from_secs(2) {break;}
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activation_arguments_are_only_bounded_opaque_routes() {
        let a=random().unwrap();let b=random().unwrap();assert_ne!(a,b);
        assert_eq!(route(&format!("v1.{a}.{b}")),Some(a.as_str()));
        for bad in ["", "file:///tmp", "v1.a.b", &format!("v1.{a}.{b}.extra"), &format!("v1.{a}.{}","f".repeat(33))] {assert!(route(bad).is_none());}
        assert_ne!(clsid("one"),clsid("two"));
        assert_eq!(broker(Path::new(r"C:\Marea ñ\bin\pleamar.exe")).unwrap(),r#""C:\Marea ñ\bin\pleamar-notifications.exe""#);
    }
}
