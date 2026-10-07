//! On-demand WGC pictures. Listening lists windows; only want/live captures them.
use super::{SysValue, ThumbnailFrame, windows_windows::{self, ThumbnailWindow}};
use std::{collections::{HashMap, HashSet}, path::PathBuf, rc::Rc,
    sync::{Arc, Condvar, Mutex, OnceLock, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use windows::Win32::{Foundation::*, UI::{Accessibility::*, WindowsAndMessaging::*}};

#[path = "windows_thumbnail_capture.rs"]
mod capture;
const PIXELS: u64 = 16_777_216;
const RETAINED_BYTES: usize = 16*1024*1024;
const LIVE_EVERY: Duration = Duration::from_millis(34);
const STILL_EVERY: Duration = Duration::from_millis(300);
type Notify = Box<dyn Fn(SysValue)+Send>;

#[derive(Default, Clone, PartialEq, Debug)]
enum Wanted { #[default] None, All, These(HashSet<u32>) }
impl Wanted {
    fn parse(args:&[SysValue]) -> Result<Self,String> {
        fn id(text:&str) -> Result<u32,String> {
            text.parse().ok().filter(|n| *n>0).ok_or("use window ids from thumbnails.list".into())
        }
        Ok(match args {
            [] | [SysValue::Null] => Self::None,
            [SysValue::Text(v)] if v=="all" => Self::All,
            [SysValue::Text(v)] => Self::These(HashSet::from([id(v)?])),
            [SysValue::List(values)] if values.len()<=1024 => Self::These(values.iter().map(|v| match v {
                SysValue::Text(v) => id(v), _ => Err("thumbnail ids must be strings".into()),
            }).collect::<Result<_,_>>()?),
            _ => return Err("thumbnails.want/live takes ids, all, or no argument".into()),
        })
    }
    fn has(&self,id:u32) -> bool { match self { Self::All=>true, Self::These(ids)=>ids.contains(&id), Self::None=>false } }
}
#[derive(Default)]
struct Request { wanted:Wanted, live:Wanted, lifetime:Option<Arc<AtomicBool>> }
impl Request { fn active(&self)->bool { self.lifetime.as_ref().is_none_or(|v|v.load(Ordering::Acquire)) } }
#[derive(Default)]
struct Listeners { callbacks:Vec<Notify>, latest:Option<SysValue> }
#[derive(Default)]
struct Wake { changed:Mutex<bool>, condition:Condvar }
impl Wake {
    fn signal(&self) { *self.changed.lock().unwrap()=true; self.condition.notify_one(); }
    fn wait(&self,duration:Duration) {
        let flag=self.changed.lock().unwrap();
        let (mut flag,_)=self.condition.wait_timeout_while(flag,duration,|flag|!*flag).unwrap();
        *flag=false;
    }
}
struct Control { requests:Mutex<HashMap<String,Request>>, listeners:Mutex<Listeners>, wake:Arc<Wake>,
    running:AtomicBool, thread:Mutex<Option<std::thread::JoinHandle<()>>> }
static CONTROL: OnceLock<Arc<Control>>=OnceLock::new();
static START: Mutex<()>=Mutex::new(());
static FRAMES: Mutex<Option<HashMap<u32,Arc<ThumbnailFrame>>>>=Mutex::new(None);
pub fn frame(name:&str)->Option<Arc<ThumbnailFrame>> {
    let id=name.strip_prefix("thumbnails:")?.parse::<u32>().ok()?;
    FRAMES.lock().unwrap().as_ref()?.get(&id).cloned()
}
fn forget(id:u32) { if let Some(frames)=FRAMES.lock().unwrap().as_mut() { frames.remove(&id); } }
fn start()->Result<Arc<Control>,String> {
    let _serial=START.lock().unwrap();
    if let Some(control)=CONTROL.get() { return Ok(control.clone()); }
    let control=Arc::new(Control {requests:Mutex::default(),listeners:Mutex::default(),wake:Arc::default(),
        running:AtomicBool::new(true),thread:Mutex::default()});
    let shared=control.clone();
    let thread=std::thread::Builder::new().name("thumbnails".into()).spawn(move || {
        let _apartment=match super::windows_system::Apartment::new() {
            Ok(a)=>a,Err(error)=>{ report_error(&shared,error);return; }
        };
        let mut worker=Worker::new(shared.clone());
        worker.run();
    }).map_err(|e|e.to_string())?;
    *control.thread.lock().unwrap()=Some(thread);
    let _=CONTROL.set(control.clone());
    Ok(control)
}
pub fn service(notify:Notify)->bool {
    let Ok(control)=start() else { return false; };
    let mut listeners=control.listeners.lock().unwrap();
    if let Some(value)=&listeners.latest { notify(value.clone()); }
    listeners.callbacks.push(notify);
    true
}
pub fn command(from:&str,name:&str,args:&[SysValue])->Result<(),String> {
    if !matches!(name,"thumbnails.want"|"thumbnails.live") { return Err(format!("unknown thumbnail command: {name}")); }
    let wanted=Wanted::parse(args)?;
    let lifetime=super::windows_capture::service_lifetime();
    if lifetime.as_ref().is_some_and(|v|!v.load(Ordering::Acquire)) { return Err("thumbnail request cancelled by reload".into()); }
    let control=start()?;
    let mut requests=control.requests.lock().unwrap();
    let request=requests.entry(from.into()).or_default();
    request.lifetime=lifetime;
    if name=="thumbnails.live" { request.live=wanted; } else { request.wanted=wanted; }
    if request.live==Wanted::None && request.wanted==Wanted::None { requests.remove(from); }
    drop(requests);
    control.wake.signal();
    Ok(())
}
pub(crate) fn release(from:&str) {
    if let Some(control)=CONTROL.get() { control.requests.lock().unwrap().remove(from);control.wake.signal(); }
}
pub(super) fn shutdown() {
    let Some(control)=CONTROL.get() else {return;};
    control.running.store(false,Ordering::Release);control.wake.signal();
    if let Some(thread)=control.thread.lock().unwrap().take() {
        let deadline=Instant::now()+Duration::from_secs(2);
        while !thread.is_finished() && Instant::now()<deadline { std::thread::sleep(Duration::from_millis(5)); }
        if thread.is_finished() {let _=thread.join();}
        else {eprintln!("windows · thumbnail capture did not finish before shutdown");}
    }
}
fn report(control:&Control,value:SysValue) {
    let mut listeners=control.listeners.lock().unwrap();
    if listeners.latest.as_ref()==Some(&value) { return; }
    listeners.latest=Some(value.clone());
    for notify in &listeners.callbacks { notify(value.clone()); }
}
fn report_error(control:&Control,error:String) { report(control,SysValue::Map(vec![
    ("list".into(),SysValue::List(vec![])),("capturing".into(),SysValue::Bool(false)),("error".into(),SysValue::Text(error))])); }

struct Slot { window:ThumbnailWindow, capture:Option<capture::Capture>, awaiting:Option<Instant>, picture:String,
    image:Option<Arc<ThumbnailFrame>>, version:u64, stale:bool, error:String, live:bool, next:Instant, used:Instant }
impl Slot {
    fn bytes(&self)->usize { self.image.as_ref().map_or(0,|image|image.pixels.len()) }
    fn pixels(&self)->u64 { self.capture.as_ref().and_then(|c|c.size().ok()).map_or(0,|(w,h)|w as u64*h as u64) }
    fn store(&mut self,root:&std::path::Path)->Result<(),String> {
        let Some(frame)=&self.image else { return Ok(()); };
        let mut rgba=frame.pixels.clone();
        // WGC is premultiplied BGRA; PNG/image decoding expects straight RGBA.
        for p in rgba.chunks_exact_mut(4) {
            p.swap(0,2);
            for c in 0..3 { p[c]=if p[3]==0 {0} else {((p[c] as u32*255+p[3] as u32/2)/p[3] as u32).min(255) as u8}; }
        }
        std::fs::create_dir_all(root).map_err(|e|e.to_string())?;
        let path=root.join(format!("{}.png",self.window.id));
        let temporary=path.with_extension("tmp");
        image::save_buffer_with_format(&temporary,&rgba,frame.width,frame.height,image::ColorType::Rgba8,image::ImageFormat::Png)
            .map_err(|e|e.to_string())?;
        std::fs::rename(&temporary,&path).map_err(|e|e.to_string())?;
        self.picture=format!("{}?{}",path.to_string_lossy(),self.version);
        forget(self.window.id);
        Ok(())
    }
}
struct Hook(HWINEVENTHOOK);
impl Drop for Hook { fn drop(&mut self) { let _=unsafe {UnhookWinEvent(self.0)}; } }
unsafe extern "system" fn destroyed(_:HWINEVENTHOOK,_:u32,hwnd:HWND,object:i32,child:i32,_:u32,_:u32) {
    if object==0 && child==0 { windows_windows::forget_handle(hwnd.0 as isize); }
}
struct Worker { control:Arc<Control>, slots:HashMap<u32,Slot>, device:Option<Rc<capture::Device>>,
    directory:PathBuf, supported:bool, error:String, _hook:Hook }
impl Worker {
    fn new(control:Arc<Control>)->Self {
        let supported=super::windows_capture_winrt::supported().unwrap_or(false);
        let hook=unsafe { SetWinEventHook(EVENT_OBJECT_DESTROY,EVENT_OBJECT_DESTROY,None,Some(destroyed),0,0,WINEVENT_OUTOFCONTEXT) };
        Self {control,slots:HashMap::new(),device:None,
            directory:std::env::temp_dir().join("pleamar-thumbnails").join(format!("{}-{}",std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos())),
            supported:supported&&!hook.is_invalid(),error:if hook.is_invalid(){"window lifetime subscription unavailable".into()}
                else if !supported {"Windows Graphics Capture is unavailable".into()} else {String::new()},_hook:Hook(hook)}
    }
    fn remove(&mut self,id:u32) {
        self.slots.remove(&id); forget(id);
        let _=std::fs::remove_file(self.directory.join(format!("{id}.png")));
    }
    fn refresh(&mut self) {
        match windows_windows::thumbnail_catalog() {
            Ok(windows)=>{
                let present:HashSet<_>=windows.iter().map(|w|w.id).collect();
                for id in self.slots.keys().copied().filter(|id|!present.contains(id)).collect::<Vec<_>>() { self.remove(id); }
                for window in windows {
                    if let Some(slot)=self.slots.get_mut(&window.id) { slot.window=window; }
                    else { self.slots.insert(window.id,Slot {window,capture:None,awaiting:None,picture:String::new(),image:None,
                        version:0,stale:false,error:String::new(),live:false,next:Instant::now(),used:Instant::now()}); }
                }
                if self.supported { self.error.clear(); }
            },
            Err(error)=>{
                self.error=error;
                for slot in self.slots.values_mut() { slot.capture=None;slot.stale=true; }
            },
        }
    }
    fn tick(&mut self) {
        let mut requests=self.control.requests.lock().unwrap();
        requests.retain(|_,r|r.active());
        let wanted:Vec<_>=self.slots.keys().map(|id|(*id,
            requests.values().any(|r|r.wanted.has(*id)||r.live.has(*id)),
            requests.values().any(|r|r.live.has(*id)))).collect();
        drop(requests);
        let now=Instant::now();
        for &(id,want,live) in &wanted {
            let slot=self.slots.get_mut(&id).unwrap();
            if slot.live!=live {
                slot.live=live;
                if live {
                    if let Some(image)=&slot.image {
                        FRAMES.lock().unwrap().get_or_insert_with(HashMap::new).insert(id,image.clone());
                        slot.picture=format!("thumbnails:{id}?{}",slot.version);
                    }
                } else if let Err(error)=slot.store(&self.directory) { slot.error=error;slot.stale=true; }
            }
            if !want || slot.window.minimized || !slot.window.identity.current() {
                slot.capture=None;
                if want { slot.stale=true; }
            }
        }
        for (id,want,_) in wanted {
            if !want || !self.supported || !self.error.is_empty() { continue; }
            let total:u64=self.slots.values().map(Slot::pixels).sum();
            let count=self.slots.values().filter(|s|s.capture.is_some()).count();
            let slot=self.slots.get_mut(&id).unwrap();
            if slot.window.minimized || !slot.window.identity.current() || now<slot.next { continue; }
            slot.used=now;
            let budget=PIXELS.saturating_sub(total-slot.pixels());
            let result=(|| -> windows::core::Result<_> {
                if slot.capture.is_none() {
                    if count>=8 { return Err(windows::core::Error::new(E_OUTOFMEMORY,"eight thumbnail capture sessions are already active")); }
                    if self.device.is_none() { self.device=Some(capture::Device::new(Some(self.control.wake.clone()))?); }
                    slot.capture=Some(capture::Capture::new(self.device.as_ref().unwrap().clone(),slot.window.identity.hwnd(),budget)?);
                    slot.awaiting=Some(now);
                }
                slot.capture.as_mut().unwrap().next(budget)
            })();
            match result {
                Ok(Some(picture))=>{
                    slot.awaiting=None;
                    slot.stale=false;slot.error.clear();
                    let changed=slot.image.as_ref().is_none_or(|f| f.width!=picture.size.0 || f.height!=picture.size.1 || f.pixels!=picture.pixels);
                    if changed {
                        slot.version+=1;
                        let frame=Arc::new(ThumbnailFrame {version:slot.version,width:picture.size.0,height:picture.size.1,pixels:picture.pixels,opaque:false});
                        slot.image=Some(frame.clone());
                        if slot.live {
                            FRAMES.lock().unwrap().get_or_insert_with(HashMap::new).insert(id,frame);
                            slot.picture=format!("thumbnails:{id}?{}",slot.version);
                        } else if let Err(error)=slot.store(&self.directory) {slot.error=error;slot.stale=true;}
                    }
                    slot.next=now+if slot.live {LIVE_EVERY}else{STILL_EVERY};
                },
                Ok(None)=>{
                    if slot.awaiting.is_some_and(|since|now.duration_since(since)>Duration::from_secs(3)) {
                        slot.capture=None;slot.stale=true;
                        slot.error="the application did not provide a capture frame".into();
                        slot.next=now+Duration::from_secs(3);
                    } else {slot.next=now+Duration::from_millis(5);}
                },
                Err(error)=>{slot.capture=None;slot.stale=true;slot.error=error.to_string();slot.next=now+Duration::from_secs(3);},
            }
        }
        if self.slots.values().all(|s|s.capture.is_none()) {self.device=None;}
        let mut retained:usize=self.slots.values().map(Slot::bytes).sum();
        let mut old:Vec<_>=self.slots.iter().filter(|(_,s)|s.capture.is_none()&&s.image.is_some()).map(|(id,s)|(*id,s.used)).collect();
        old.sort_by_key(|(_,used)|*used);
        for (id,_) in old {
            if retained<=RETAINED_BYTES {break;}
            let slot=self.slots.get_mut(&id).unwrap();retained-=slot.bytes();
            slot.image=None;slot.picture.clear();forget(id);
            let _=std::fs::remove_file(self.directory.join(format!("{id}.png")));
        }
    }
    fn report(&self) {
        let mut slots:Vec<_>=self.slots.iter().collect();slots.sort_by_key(|(id,_)|**id);
        report(&self.control,SysValue::Map(vec![("capturing".into(),SysValue::Bool(self.supported)),
            ("error".into(),SysValue::Text(self.error.clone())),("list".into(),SysValue::List(slots.into_iter().map(|(id,s)|SysValue::Map(vec![
                ("id".into(),SysValue::Text(id.to_string())),("title".into(),SysValue::Text(s.window.title.clone())),
                ("app".into(),SysValue::Text(s.window.app.clone())),("picture".into(),SysValue::Text(s.picture.clone())),
                ("stale".into(),SysValue::Bool(s.stale)),("error".into(),SysValue::Text(s.error.clone()))])).collect()))]));
    }
    fn run(&mut self) {
        let mut next_catalog=Instant::now();
        while self.control.running.load(Ordering::Acquire) {
            unsafe { let mut message=MSG::default();for _ in 0..512 {
                if !PeekMessageW(&mut message,None,0,0,PM_REMOVE).as_bool(){break;}
                let _=TranslateMessage(&message);DispatchMessageW(&message);
            } }
            if Instant::now()>=next_catalog {self.refresh();next_catalog=Instant::now()+Duration::from_secs(1);}
            self.tick();self.report();
            let now=Instant::now();let mut wait=next_catalog.saturating_duration_since(now);
            for slot in self.slots.values() {
                if slot.capture.as_ref().is_some_and(|c|c.ready()) {wait=wait.min(slot.next.saturating_duration_since(now));}
            }
            self.control.wake.wait(wait.max(Duration::from_millis(1)));
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        for id in self.slots.keys().copied().collect::<Vec<_>>() {
            self.remove(id);
            let _=std::fs::remove_file(self.directory.join(format!("{id}.tmp")));
        }
        self.device=None;
        let _=std::fs::remove_dir(&self.directory);
    }
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn selectors_are_explicit_and_bounded() {
        assert_eq!(Wanted::parse(&[]).unwrap(),Wanted::None);
        assert!(Wanted::parse(&[SysValue::Text("all".into())]).unwrap().has(12));
        assert!(!Wanted::parse(&[SysValue::Text("12".into())]).unwrap().has(13));
        assert!(Wanted::parse(&[SysValue::Text("0".into())]).is_err());
        assert!(Wanted::parse(&[SysValue::List(vec![SysValue::Num(12.0)])]).is_err());
        assert!(Wanted::parse(&[SysValue::List(vec![SysValue::Text("1".into());1025])]).is_err());
    }
    #[test] fn reload_revokes_old_asynchronous_demand() {
        let lifetime=Arc::new(AtomicBool::new(true));
        let request=Request {live:Wanted::All,lifetime:Some(lifetime.clone()),..Default::default()};
        assert!(request.active());lifetime.store(false,Ordering::Release);assert!(!request.active());
    }
}
