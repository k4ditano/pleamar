//! Guarded foreground desktop operations for a native companion's dedicated worker.
//! A session owns its catalog and single-use capture permits on one thread.
pub use crate::platform::SysValue as Value;
use crate::platform::{windows_capture as lifetime, windows_desktop as native};
use std::{marker::PhantomData, rc::Rc, sync::{Arc, atomic::{AtomicBool, Ordering}}};

/// Companion entry points dispatch this before normal arguments. The windowless
/// child bounds applications that do not return from their GDI print handler.
pub fn capture_helper(args: &[String]) -> Option<i32> { native::capture_helper(args) }

/// Capture an already-validated companion window on an unused worker thread.
/// A disabled owner is readable; it is never redirected to its modal dialog.
/// No input catalog, permit, focus request or control endpoint is created.
pub fn read_only_picture(handle: usize, process: u32, thread: u32) -> Result<Value, String> {
    if lifetime::service_lifetime().is_some() || native::has_thread_state() {
        return Err("read-only capture requires an unused worker thread".into());
    }
    native::read_only_picture(handle, process, thread)
}

/// Stops this session, including an operation currently waiting for capture/input.
/// Cancellation is permanent; continuing requires a new session and fresh pictures.
#[derive(Clone)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) { self.0.store(false, Ordering::Release); }
}

/// Use only on a dedicated worker. This is deliberately neither Send nor Sync.
/// Input uses the user's foreground queue, with the same guards as Luau desktop.*.
pub struct Session {
    active: Cancellation,
    epoch: Option<Value>,
    _thread: PhantomData<Rc<()>>,
}
impl Session {
    /// Does not enumerate windows, create a surface or request focus.
    pub fn new() -> Result<Self, String> {
        if lifetime::service_lifetime().is_some() || native::has_thread_state() {
            return Err("desktop sessions require an unused dedicated worker thread".into());
        }
        let active=Cancellation(Arc::new(AtomicBool::new(true)));
        lifetime::set_service_lifetime(active.0.clone());
        native::exact_targets(true);
        Ok(Self { active, epoch:None, _thread:PhantomData })
    }
    pub fn cancellation(&self) -> Cancellation { self.active.clone() }
    pub fn windows(&mut self) -> Result<Value, String> {
        self.epoch=None;
        let result=native::query("desktop.windows",&[])?;
        let Value::Map(fields)=&result else { return Err("invalid native desktop catalog".into()); };
        self.epoch=fields.iter().find(|(name,_)|name=="epoch").map(|(_,value)|value.clone());
        Ok(result)
    }
    /// Revoke all capture permits, including after a cancelled or failed operation.
    pub fn forget(&mut self) { native::clear_catalog(); self.epoch=None; }
    /// Resolve a companion's already-validated identity in this session's catalog.
    /// Raw handles alone never grant input: a current catalog and picture are required.
    pub fn window_id(&self, handle:usize, process:u32, thread:u32) -> Result<String,String> {
        native::catalog_id(handle,process,thread)
    }
    /// Returns a PNG and its physical-pixel dimensions. Each input consumes its permit.
    pub fn look(&mut self, window:&str) -> Result<Value,String> {
        native::query("desktop.look",&[Value::Text(window.into())])
    }
    /// Focus is explicit; other input requires a fresh picture and foreground target.
    /// No arbitrary system service or saved credential is exposed through this API.
    pub fn action(&mut self, action:&str, window:&str, arguments:&[Value]) -> Result<(),String> {
        if !matches!(action,"focus"|"move"|"click"|"drag"|"scroll"|"type"|"key"|"hotkey") {
            return Err("unsupported native desktop action".into());
        }
        let epoch=self.epoch.clone().ok_or("list windows before native input")?;
        native::validate_exact_target(window)?;
        let mut args=vec![epoch,Value::Text(window.into())];args.extend_from_slice(arguments);
        native::command(&format!("desktop.{action}"),&args)
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.active.cancel();
        // Cleanup must still run after cancellation, when normal operations refuse.
        native::clear_catalog();
        native::exact_targets(false);
        lifetime::clear_service_lifetime();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_ownership_and_cancellation_never_create_or_focus_windows() {
        std::thread::spawn(|| {
            assert!(read_only_picture(0, 0, 0).unwrap_err().contains("current visible"));
            assert!(!native::has_thread_state());
            let mut first=Session::new().unwrap();
            let cancelled=first.cancellation();
            assert!(Session::new().is_err());
            assert!(read_only_picture(0, 0, 0).unwrap_err().contains("unused worker"));
            assert!(first.action("type","1",&[Value::Text("must not type".into())]).unwrap_err().contains("list windows"));
            assert!(first.action("type_secret","1",&[]).unwrap_err().contains("unsupported"));
            cancelled.cancel();
            assert!(first.windows().unwrap_err().contains("cancelled"));
            assert!(first.look("1").unwrap_err().contains("cancelled"));
            drop(first);
            let next=Session::new().unwrap();
            cancelled.cancel();
            assert!(next.active.0.load(Ordering::Acquire));
        }).join().unwrap();
    }
}
