//! An opt-in Windows-key layer. No injected input or persistent OS remapping.
use super::SysValue;
use std::{cell::RefCell, collections::HashMap, sync::{Arc, OnceLock, atomic::{AtomicBool, Ordering}, mpsc}, time::Duration};
use windows::{core::w, Win32::{Foundation::*, System::Threading::*, UI::{Accessibility::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*}}};

const REQUEST:u32=WM_APP+31;
const FIRE:u32=WM_APP+32;
const RESET:u32=WM_APP+33;
const CONTROL:u8=1;
const ALT:u8=2;
const SHIFT:u8=4;

#[derive(Clone,Copy,Debug,Eq,Hash,PartialEq)]
struct Chord { modifiers:u8, key:u16 }
#[derive(Clone)]
pub(super) struct Profile { chords:HashMap<Chord,usize>, events:Vec<String> }

pub(super) fn parse(value:&SysValue) -> Result<Option<Profile>,String> {
    if matches!(value,SysValue::Bool(false)) { return Ok(None); }
    let SysValue::Map(entries)=value else { return Err("hotkeys.windows takes false or a map from Windows-key chords to event names".into()); };
    if entries.is_empty() { return Ok(None); }
    if entries.len()>16 { return Err("at most 16 Windows-key actions can be registered".into()); }
    let mut profile=Profile {chords:HashMap::new(),events:Vec::new()};
    for (source,event) in entries {
        let SysValue::Text(event)=event else { return Err("shortcut events must be text".into()); };
        super::validate_event(event)?;
        let mut logo=false;let mut modifiers=0;let mut key=None;
        for part in source.split('+').map(|s|s.trim().to_ascii_lowercase()) {
            let flag=match part.as_str() {"ctrl"|"control"=>CONTROL,"alt"=>ALT,"shift"=>SHIFT,_=>0};
            if matches!(part.as_str(),"win"|"windows"|"super") {
                if logo { return Err("duplicate Windows-key modifier".into()); } logo=true;
            } else if flag!=0 {
                if modifiers&flag!=0 { return Err("duplicate shortcut modifier".into()); } modifiers|=flag;
            } else {
                let code=match part.as_str() {
                    "space"=>VK_SPACE.0,"tab"=>VK_TAB.0,"enter"|"return"=>VK_RETURN.0,"escape"|"esc"=>VK_ESCAPE.0,
                    "left"=>VK_LEFT.0,"right"=>VK_RIGHT.0,"up"=>VK_UP.0,"down"=>VK_DOWN.0,
                    _ if part.len()==1&&part.as_bytes()[0].is_ascii_alphanumeric()=>part.as_bytes()[0].to_ascii_uppercase() as u16,
                    _=>return Err("Windows-key actions use a letter, digit, arrow, Space, Tab, Enter or Escape".into()),
                };
                if key.replace(code).is_some() { return Err("a shortcut has exactly one non-modifier key".into()); }
            }
        }
        if !logo || (key.is_none()&&modifiers!=0) { return Err("use Win alone, or Win plus optional modifiers and a key".into()); }
        if profile.chords.insert(Chord {modifiers,key:key.unwrap_or(0)},profile.events.len()).is_some() {
            return Err("duplicate Windows-key chord".into());
        }
        profile.events.push(event.clone());
    }
    Ok(Some(profile))
}

struct Keys { down:[bool;256], hidden:[bool;256], used:bool }
impl Keys {
    fn new(mut down:[bool;256]) -> Self {
        // GetAsyncKeyState reports both aggregate and sided modifiers. The hook
        // reports sided releases, so aggregate snapshot bits would stay stuck.
        for key in [VK_CONTROL,VK_MENU,VK_SHIFT] { down[key.0 as usize]=false; }
        Self {down,hidden:[false;256],used:false}
    }
    fn modifiers(&self) -> u8 {
        let held=|keys:&[VIRTUAL_KEY]|keys.iter().any(|k|self.down[k.0 as usize]);
        (if held(&[VK_CONTROL,VK_LCONTROL,VK_RCONTROL]) {CONTROL} else {0}) |
        (if held(&[VK_MENU,VK_LMENU,VK_RMENU]) {ALT} else {0}) |
        (if held(&[VK_SHIFT,VK_LSHIFT,VK_RSHIFT]) {SHIFT} else {0})
    }
    fn route(&mut self,key:u32,down:bool,injected:bool,profile:&Profile) -> (bool,Option<usize>) {
        if injected||key>=256 { return (false,None); }
        let k=key as usize;let was=self.down[k];self.down[k]=down;
        let left=VK_LWIN.0 as usize;let right=VK_RWIN.0 as usize;
        if k==left||k==right {
            if down {
                if !was {
                    let other=if k==left {right} else {left};
                    // A Windows key already held when enabling belongs to the OS.
                    if self.down[other]&&!self.hidden[other] { self.used=true;return (false,None); }
                    self.hidden[k]=true;
                    self.used=self.down.iter().enumerate().any(|(i,v)|*v&&i!=k);
                }
                return (self.hidden[k],None);
            }
            let hidden=std::mem::take(&mut self.hidden[k]);
            let alone=hidden&&!self.used&&!self.down[left]&&!self.down[right];
            let event=alone.then(||profile.chords.get(&Chord {modifiers:0,key:0}).copied()).flatten();
            if !self.down[left]&&!self.down[right] { self.used=false; }
            return (hidden,event);
        }
        if !down { return (std::mem::take(&mut self.hidden[k]),None); }
        if was { return (self.hidden[k],None); }
        if self.hidden[left]||self.hidden[right] {
            self.used=true;self.hidden[k]=true;
            let event=profile.chords.get(&Chord {modifiers:self.modifiers(),key:key as u16}).copied();
            return (true,event);
        }
        (false,None)
    }
}

struct Routing { keys:Keys, profile:Profile, alive:Arc<AtomicBool>, generation:usize }
thread_local! { static ROUTING:RefCell<Option<Routing>>=const {RefCell::new(None)}; }
fn held_keys() -> [bool;256] {
    std::array::from_fn(|key|unsafe {GetAsyncKeyState(key as i32)<0})
}
unsafe extern "system" fn keyboard(code:i32,w:WPARAM,l:LPARAM) -> LRESULT {
    if code>=0&&matches!(w.0 as u32,WM_KEYDOWN|WM_KEYUP|WM_SYSKEYDOWN|WM_SYSKEYUP) {
        let event=unsafe {&*(l.0 as *const KBDLLHOOKSTRUCT)};
        let consumed=ROUTING.with(|state| {
            let mut state=state.borrow_mut();let Some(state)=state.as_mut() else {return false;};
            if !state.alive.load(Ordering::Acquire) { return false; }
            let (hidden,action)=state.keys.route(event.vkCode,matches!(w.0 as u32,WM_KEYDOWN|WM_SYSKEYDOWN),
                event.flags.contains(LLKHF_INJECTED),&state.profile);
            if let Some(action)=action {
                // Never run service subscribers or Luau in the input hook.
                let _=unsafe {PostThreadMessageW(GetCurrentThreadId(),FIRE,WPARAM(state.generation),LPARAM(action as isize))};
            }
            hidden
        });
        if consumed { return LRESULT(1); }
    }
    unsafe {CallNextHookEx(None,code,w,l)}
}
unsafe extern "system" fn desktop(_:HWINEVENTHOOK,_:u32,_:HWND,_:i32,_:i32,_:u32,_:u32) {
    let _=unsafe {PostThreadMessageW(GetCurrentThreadId(),RESET,WPARAM(0),LPARAM(0))};
}

struct Hook { keyboard:HHOOK, desktop:HWINEVENTHOOK, mutex:HANDLE, timer:usize }
impl Hook {
    fn new() -> Result<Self,String> { unsafe {
        let mutex=CreateMutexW(None,false,w!("Local\\pleamar.windows-key")).map_err(|e|e.to_string())?;
        match WaitForSingleObject(mutex,0) {
            WAIT_OBJECT_0|WAIT_ABANDONED=>{},
            _=>{let _=CloseHandle(mutex);return Err("another application owns the Windows-key layer".into());}
        }
        let mut hook=Self {keyboard:HHOOK::default(),desktop:HWINEVENTHOOK::default(),mutex,timer:0};
        let module=windows::Win32::System::LibraryLoader::GetModuleHandleW(None).map_err(|e|e.to_string())?;
        hook.keyboard=SetWindowsHookExW(WH_KEYBOARD_LL,Some(keyboard),Some(module.into()),0).map_err(|e|e.to_string())?;
        hook.desktop=SetWinEventHook(EVENT_SYSTEM_DESKTOPSWITCH,EVENT_SYSTEM_DESKTOPSWITCH,None,Some(desktop),0,0,WINEVENT_OUTOFCONTEXT);
        if hook.desktop.is_invalid() { return Err("could not watch desktop changes for the Windows key".into()); }
        hook.timer=SetTimer(None,0,250,None);
        if hook.timer==0 { return Err("could not watch the Windows-key owner lifetime".into()); }
        Ok(hook)
    } }
}
impl Drop for Hook { fn drop(&mut self) { unsafe {
    ROUTING.with(|state|state.borrow_mut().take());
    if !self.keyboard.is_invalid() { let _=UnhookWindowsHookEx(self.keyboard); }
    if !self.desktop.is_invalid() { let _=UnhookWinEvent(self.desktop); }
    if self.timer!=0 { let _=KillTimer(None,self.timer); }
    let _=ReleaseMutex(self.mutex);let _=CloseHandle(self.mutex);
} } }

enum Request {
    Configure(Option<Profile>,Option<Arc<AtomicBool>>,mpsc::SyncSender<Result<(),String>>),
    State(mpsc::SyncSender<Result<SysValue,String>>),
}
struct Manager { sender:mpsc::SyncSender<Request>, thread:u32 }
static MANAGER:OnceLock<Result<Manager,String>>=OnceLock::new();
fn listen(requests:mpsc::Receiver<Request>,ready:mpsc::SyncSender<u32>) { unsafe {
    let mut message=MSG::default();let _=PeekMessageW(&mut message,None,0,0,PM_NOREMOVE);
    let _=ready.send(GetCurrentThreadId());let mut hook:Option<Hook>=None;let mut generation=0usize;
    while GetMessageW(&mut message,None,0,0).0>0 {
        let live=ROUTING.with(|state|state.borrow().as_ref().is_some_and(|s|s.alive.load(Ordering::Acquire)));
        if hook.is_some()&&!live { hook.take(); }
        match message.message {
            REQUEST=>for request in requests.try_iter() {
                match request {
                    Request::State(reply)=>{let _=reply.send(Ok(SysValue::Map(vec![
                        ("available".into(),SysValue::Bool(true)),("windows_key".into(),SysValue::Bool(hook.is_some()))])));},
                    Request::Configure(profile,alive,reply)=>{
                        let result=(|| {
                            let other=ROUTING.with(|state|state.borrow().as_ref().is_some_and(|s|s.alive.load(Ordering::Acquire)&&!alive.as_ref().is_some_and(|a|Arc::ptr_eq(&s.alive,a))));
                            if other { return Err("another live scene owns the Windows-key layer".into()); }
                            if let Some(profile)=profile {
                                let alive=alive.filter(|a|a.load(Ordering::Acquire)).ok_or("use call_async so the Windows-key layer has a live owner")?;
                                if hook.is_none() { hook=Some(Hook::new()?); }
                                generation=generation.checked_add(1).ok_or("Windows-key generation exhausted")?;
                                ROUTING.with(|state| {
                                    let mut state=state.borrow_mut();
                                    // Keep ownership of suppressed key releases across a profile update.
                                    let keys=state.take().map(|s|s.keys).unwrap_or_else(||Keys::new(held_keys()));
                                    *state=Some(Routing {keys,profile,alive,generation});
                                });
                            } else { hook.take(); }
                            Ok(())
                        })();let _=reply.send(result);
                    }
                }
            },
            FIRE=>{
                let event=ROUTING.with(|state|state.borrow().as_ref().filter(|s|s.generation==message.wParam.0&&s.alive.load(Ordering::Acquire))
                    .and_then(|s|s.profile.events.get(message.lParam.0 as usize)).cloned());
                if let Some(event)=event { let _=super::publish(event); }
            },
            RESET=>ROUTING.with(|state| {if let Some(s)=state.borrow_mut().as_mut() {s.keys=Keys::new(held_keys());}}),
            _=>{},
        }
    }
    drop(hook);
} }

fn manager() -> Result<&'static Manager,String> {
    MANAGER.get_or_init(|| {
        let (sender,requests)=mpsc::sync_channel(16);let (ready,started)=mpsc::sync_channel(1);
        std::thread::Builder::new().name("windows-key".into()).spawn(move || listen(requests,ready)).map_err(|e|e.to_string())?;
        Ok(Manager {sender,thread:started.recv().map_err(|e|e.to_string())?})
    }).as_ref().map_err(Clone::clone)
}
fn send(request:Request) -> Result<(),String> {
    let manager=manager()?;manager.sender.try_send(request).map_err(|e|e.to_string())?;
    unsafe {PostThreadMessageW(manager.thread,REQUEST,WPARAM(0),LPARAM(0)).map_err(|e|e.to_string())}
}
pub(super) fn configure(value:&SysValue) -> Result<(),String> {
    let profile=parse(value)?;let (reply,response)=mpsc::sync_channel(1);
    send(Request::Configure(profile,super::super::windows_capture::service_lifetime(),reply))?;
    response.recv_timeout(Duration::from_secs(5)).map_err(|e|e.to_string())?
}
pub(super) fn state() -> Result<SysValue,String> {
    let (reply,response)=mpsc::sync_channel(1);send(Request::State(reply))?;
    response.recv_timeout(Duration::from_secs(5)).map_err(|e|e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> Profile {parse(&SysValue::Map(vec![
        ("Win".into(),SysValue::Text("search".into())),("Win+Space".into(),SysValue::Text("search".into())),
        ("Win+Shift+A".into(),SysValue::Text("chat".into()))])).unwrap().unwrap()}
    #[test]
    fn profile_rejects_invalid_or_duplicate_chords() {
        for chord in ["Ctrl+A","Win+Shift","Win+Win+A","Win+Ctrl+Ctrl+A","Win+A+B","Win+","Win+F12"] {
            assert!(parse(&SysValue::Map(vec![(chord.into(),SysValue::Text("search".into()))])).is_err(),"{chord}");
        }
        assert!(parse(&SysValue::Map(vec![("Win+A".into(),SysValue::Text("a".into())),("SUPER+a".into(),SysValue::Text("b".into()))])).is_err());
        assert!(parse(&SysValue::Bool(false)).unwrap().is_none());
    }
    #[test]
    fn windows_tap_and_chords_fire_once_without_leaking_keys() {
        let p=profile();let mut keys=Keys::new([false;256]);
        let mut step=|key:VIRTUAL_KEY,down|keys.route(key.0 as u32,down,false,&p);
        assert_eq!(step(VK_LWIN,true),(true,None));assert_eq!(step(VK_LWIN,true),(true,None));
        assert_eq!(step(VK_LWIN,false),(true,Some(0)));
        assert_eq!(step(VK_RWIN,true),(true,None));assert_eq!(step(VK_SPACE,true),(true,Some(1)));
        assert_eq!(step(VK_SPACE,true),(true,None));assert_eq!(step(VK_RWIN,false),(true,None));
        assert_eq!(step(VK_SPACE,true),(true,None));assert_eq!(step(VK_SPACE,false),(true,None));
        assert_eq!(step(VK_SPACE,true),(false,None));assert_eq!(step(VK_SPACE,false),(false,None));
        step(VK_LWIN,true);step(VK_LSHIFT,true);assert_eq!(step(VK_A,true),(true,Some(2)));
        step(VK_A,false);step(VK_LSHIFT,false);assert_eq!(step(VK_LWIN,false),(true,None));
    }
    #[test]
    fn arrow_navigation_is_registered_without_repeats_or_leaking_releases() {
        let arrows=[("Left",VK_LEFT),("Right",VK_RIGHT),("Up",VK_UP),("Down",VK_DOWN)];
        let p=parse(&SysValue::Map(arrows.iter().map(|(name,_)|
            (format!("Win+{name}"),SysValue::Text("navigate".into()))).collect())).unwrap().unwrap();
        let mut keys=Keys::new([false;256]);
        for (i,(_,key)) in arrows.into_iter().enumerate() {
            assert_eq!(keys.route(VK_LWIN.0 as u32,true,false,&p),(true,None));
            assert_eq!(keys.route(key.0 as u32,true,false,&p),(true,Some(i)));
            assert_eq!(keys.route(key.0 as u32,true,false,&p),(true,None));
            assert_eq!(keys.route(VK_LWIN.0 as u32,false,false,&p),(true,None));
            assert_eq!(keys.route(key.0 as u32,false,false,&p),(true,None));
            assert_eq!(keys.route(key.0 as u32,true,false,&p),(false,None));
            assert_eq!(keys.route(key.0 as u32,false,false,&p),(false,None));
        }
    }
    #[test]
    fn reserved_unknown_chords_and_preexisting_keys_do_not_type_or_stick() {
        let p=profile();let mut keys=Keys::new([false;256]);
        assert_eq!(keys.route(VK_LCONTROL.0 as u32,true,false,&p),(false,None));
        keys.route(VK_LWIN.0 as u32,true,false,&p);
        assert_eq!(keys.route(VK_LCONTROL.0 as u32,false,false,&p),(false,None));
        assert_eq!(keys.route(VK_E.0 as u32,true,false,&p),(true,None));
        assert_eq!(keys.route(VK_E.0 as u32,false,false,&p),(true,None));
        assert_eq!(keys.route(VK_LWIN.0 as u32,false,false,&p),(true,None));
        let mut held=[false;256];held[VK_LWIN.0 as usize]=true;
        let mut keys=Keys::new(held);
        assert_eq!(keys.route(VK_LWIN.0 as u32,true,false,&p),(false,None));
        assert_eq!(keys.route(VK_RWIN.0 as u32,true,false,&p),(false,None));
        assert_eq!(keys.route(VK_RWIN.0 as u32,false,false,&p),(false,None));
        assert_eq!(keys.route(VK_LWIN.0 as u32,false,false,&p),(false,None));
    }
    #[test]
    fn injected_keys_and_two_windows_keys_do_not_trigger_a_tap() {
        let p=profile();let mut keys=Keys::new([false;256]);
        assert_eq!(keys.route(VK_LWIN.0 as u32,true,true,&p),(false,None));
        assert_eq!(keys.route(VK_LWIN.0 as u32,false,true,&p),(false,None));
        keys.route(VK_LWIN.0 as u32,true,false,&p);keys.route(VK_RWIN.0 as u32,true,false,&p);
        assert_eq!(keys.route(VK_LWIN.0 as u32,false,false,&p),(true,None));
        assert_eq!(keys.route(VK_RWIN.0 as u32,false,false,&p),(true,None));
    }

    #[test]
    fn held_modifier_snapshot_does_not_leave_aggregate_keys_stuck() {
        let p=profile();let mut held=[false;256];
        for key in [VK_CONTROL,VK_LCONTROL,VK_MENU,VK_RMENU,VK_SHIFT,VK_LSHIFT] {held[key.0 as usize]=true;}
        let mut keys=Keys::new(held);assert_eq!(keys.modifiers(),CONTROL|ALT|SHIFT);
        for key in [VK_LCONTROL,VK_RMENU,VK_LSHIFT] {assert_eq!(keys.route(key.0 as u32,false,false,&p),(false,None));}
        assert_eq!(keys.modifiers(),0);
        keys.route(VK_LWIN.0 as u32,true,false,&p);
        assert_eq!(keys.route(VK_SPACE.0 as u32,true,false,&p),(true,Some(1)));
        keys.route(VK_SPACE.0 as u32,false,false,&p);
        keys.route(VK_LWIN.0 as u32,false,false,&p);
        keys.route(VK_LWIN.0 as u32,true,false,&p);
        assert_eq!(keys.route(VK_LWIN.0 as u32,false,false,&p),(true,Some(0)));
    }

    #[test]
    #[ignore = "real hook lifecycle on a new private desktop; never switches desktops or injects input"]
    fn native_layer_private_desktop() {
        use windows::{core::PCWSTR,Win32::System::StationsAndDesktops::*};
        struct Desktop {old:HDESK,new:HDESK}
        impl Drop for Desktop {fn drop(&mut self) {unsafe {let _=SetThreadDesktop(self.old);let _=CloseDesktop(self.new);}}}
        let (sender,requests)=mpsc::sync_channel(16);let (ready,started)=mpsc::sync_channel(1);
        let thread=std::thread::spawn(move || unsafe {
            let old=GetThreadDesktop(GetCurrentThreadId()).unwrap();
            let name:Vec<_>=format!("pleamar-hotkeys-test-{}-{}",std::process::id(),GetCurrentThreadId()).encode_utf16().chain([0]).collect();
            let new=CreateDesktopW(PCWSTR(name.as_ptr()),PCWSTR::null(),None,DESKTOP_CONTROL_FLAGS(0),
                DESKTOP_CREATEWINDOW.0|DESKTOP_READOBJECTS.0|DESKTOP_WRITEOBJECTS.0|DESKTOP_HOOKCONTROL.0,None).unwrap();
            let _desktop=Desktop {old,new};assert_ne!(old,new);
            SetThreadDesktop(new).unwrap();assert_eq!(GetThreadDesktop(GetCurrentThreadId()).unwrap(),new);
            listen(requests,ready);
        });
        let manager=Manager {sender,thread:started.recv_timeout(Duration::from_secs(10)).unwrap()};
        struct Listener {id:u32,thread:Option<std::thread::JoinHandle<()>>}
        impl Drop for Listener {fn drop(&mut self) {
            let _=unsafe {PostThreadMessageW(self.id,WM_QUIT,WPARAM(0),LPARAM(0))};
            if let Some(thread)=self.thread.take() {thread.join().unwrap();}
        }}
        let listener=Listener {id:manager.thread,thread:Some(thread)};
        let wake=||unsafe {PostThreadMessageW(manager.thread,REQUEST,WPARAM(0),LPARAM(0)).unwrap()};
        let configure=|profile,alive| {
            let (reply,receive)=mpsc::sync_channel(1);
            manager.sender.send(Request::Configure(profile,alive,reply)).unwrap();wake();
            receive.recv_timeout(Duration::from_secs(5)).unwrap()
        };
        let enabled=|| {
            let (reply,receive)=mpsc::sync_channel(1);manager.sender.send(Request::State(reply)).unwrap();wake();
            let SysValue::Map(state)=receive.recv_timeout(Duration::from_secs(5)).unwrap().unwrap() else {panic!("state is not a map")};
            state.iter().any(|(k,v)|k=="windows_key"&&matches!(v,SysValue::Bool(true)))
        };
        let released=||unsafe {
            let mutex=CreateMutexW(None,false,w!("Local\\pleamar.windows-key")).unwrap();
            let free=WaitForSingleObject(mutex,0)==WAIT_OBJECT_0;
            if free {ReleaseMutex(mutex).unwrap();}CloseHandle(mutex).unwrap();assert!(free,"hook owner mutex leaked");
        };
        assert!(!enabled());assert!(configure(Some(profile()),None).is_err());
        let alive=Arc::new(AtomicBool::new(true));
        configure(Some(profile()),Some(alive.clone())).unwrap();assert!(enabled());
        let next=Arc::new(AtomicBool::new(true));
        assert!(configure(Some(profile()),Some(next.clone())).is_err());
        assert!(configure(None,Some(next.clone())).is_err());assert!(enabled());
        assert!(configure(None,None).is_err());assert!(enabled());
        alive.store(false,Ordering::Release);
        std::thread::sleep(Duration::from_millis(700));
        released();assert!(!enabled());
        for _ in 0..3 {
            configure(Some(profile()),Some(next.clone())).unwrap();assert!(enabled());
            configure(None,Some(next.clone())).unwrap();assert!(!enabled());released();
        }
        configure(Some(profile()),Some(next)).unwrap();drop(listener);released();
        println!("PASS: private desktop hook ownership, lease expiry, repeated opt-out and thread shutdown; no input or desktop switch");
    }
}
