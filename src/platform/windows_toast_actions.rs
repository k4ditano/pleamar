//! Bounded one-use actions, owned by the scene lifetime, not a worker thread.
use super::{SysValue, windows_toast_activation as activation, windows_toasts as toasts};
use std::{collections::{HashMap, HashSet, VecDeque}, io::Write, os::windows::io::AsRawHandle,
    sync::{Arc, Condvar, Mutex, OnceLock, atomic::{AtomicBool, AtomicU64, Ordering}}, time::{Duration,Instant}};
use windows::{core::HSTRING, Win32::{Foundation::HANDLE, System::Pipes::*}};

const LIMIT:usize=64;
const EXPIRY:Duration=Duration::from_secs(6*60*60);
#[derive(Clone)]
struct Owner { name:String, lifetime:Option<Arc<AtomicBool>> }
impl Owner {fn live(&self)->bool {self.lifetime.as_ref().is_none_or(|v|v.load(Ordering::Acquire))}}
struct Notice { owner:Owner, tag:String, physical:String, tokens:Vec<(String,String)>, until:Instant, delivered:Arc<AtomicBool> }
struct Event { owner:Owner, tag:String, physical:String, action:String, until:Instant }
#[derive(Default)]
struct Store { notices:HashMap<String,Notice>, events:VecDeque<Event>, removals:Vec<String> }
impl Store {
    fn prune(&mut self,now:Instant) {
        self.notices.retain(|_,n| {let keep=n.owner.live() && now<n.until;if !keep {self.removals.push(n.physical.clone());}keep});
        self.events.retain(|e|e.owner.live() && now<e.until);
    }
    fn cancel(&mut self,owner:&str,tag:Option<&str>) {
        self.notices.retain(|_,n| {let keep=n.owner.name!=owner || tag.is_some_and(|t|t!=n.tag);if !keep {self.removals.push(n.physical.clone());}keep});
        self.events.retain(|e|e.owner.name!=owner || tag.is_some_and(|t|t!=e.tag));
    }
    fn cancel_publication(&mut self,physical:&str) {
        self.notices.remove(physical);
        self.events.retain(|e|e.physical!=physical);
    }
    fn activate(&mut self,token:&str,now:Instant)->bool {
        self.prune(now);
        let found=self.notices.iter().find_map(|(id,n)|n.tokens.iter().find(|(t,_)|t==token).map(|(_,a)|(id.clone(),a.clone())));
        let Some((id,action))=found else {return false;};
        let n=self.notices.remove(&id).unwrap();
        n.delivered.store(true,Ordering::Release);
        self.removals.push(n.physical.clone());
        self.events.push_back(Event{owner:n.owner,tag:n.tag,physical:n.physical,action,until:n.until});
        true
    }
    fn drain(&mut self,owner:&str,now:Instant)->SysValue {
        self.prune(now);
        let mut answer=Vec::new();
        self.events.retain(|e| {
            if e.owner.name!=owner {return true;}
            answer.push(SysValue::Map(vec![("tag".into(),SysValue::Text(e.tag.clone())),("action".into(),SysValue::Text(e.action.clone()))]));false
        });
        SysValue::List(answer)
    }
    fn history_targets(&self)->Vec<String> {
        self.notices.iter().filter(|(_,n)|n.delivered.load(Ordering::Acquire)).map(|(id,_)|id.clone()).collect()
    }
    fn reconcile(&mut self,checked:&[String],present:&HashSet<String>) {
        // Only compare notices confirmed before this OS snapshot was requested.
        // A concurrent new publication must not be mistaken for a dismissal.
        for id in checked {
            if present.contains(id) {continue;}
            if let Some(n)=self.notices.remove(id) {
                self.events.push_back(Event{owner:n.owner,tag:n.tag,physical:n.physical,action:"dismissed".into(),until:n.until});
            }
        }
    }
}
struct Control { store:Mutex<Store>, wake:Condvar, route:String, running:AtomicBool, thread:Mutex<Option<std::thread::JoinHandle<()>>> }
impl Control {
    fn stop(&self) {
        // Pair the shutdown condition with the wait mutex so notification cannot
        // land between the predicate check and the worker entering its wait.
        let _state=self.store.lock().unwrap();
        self.running.store(false,Ordering::Release);self.wake.notify_one();
    }
}
static CONTROL:OnceLock<Arc<Control>>=OnceLock::new();
static START:Mutex<()>=Mutex::new(());
fn start(id:&str)->Result<Arc<Control>,String> {
    let _serial=START.lock().unwrap();
    if let Some(c)=CONTROL.get() {return Ok(c.clone());}
    let c=Arc::new(Control{store:Mutex::default(),wake:Condvar::new(),route:activation::random()?,running:AtomicBool::new(true),thread:Mutex::default()});
    let mut pipe=super::windows_ipc::bind_path(&activation::pipe(id,&c.route))?;
    let (ready,recv)=std::sync::mpsc::sync_channel(1);
    let shared=c.clone();let id=id.to_owned();
    let thread=std::thread::Builder::new().name("toast-actions".into()).spawn(move || {
        let setup=(|| {let a=super::windows_system::Apartment::new()?;
            let class=activation::Registration::new(&id,activation::clsid(&id),Arc::new(AtomicU64::new(0)))?;Ok::<_,String>((a,class))})();
        let Ok((_apartment,_class))=setup else {let _=ready.send(setup.err().unwrap());return;};
        let _=ready.send(String::new());
        let mut next_history=Instant::now()+Duration::from_secs(5);
        while shared.running.load(Ordering::Acquire) {
            let handle=HANDLE(pipe.as_raw_handle());
            if super::windows_ipc::accept_ready(&pipe) {
                if let Ok(token)=super::windows_ipc::read_line(&mut pipe,Instant::now()+Duration::from_millis(500)) {
                    let accepted=activation::route(&token)==Some(shared.route.as_str())
                        && shared.store.lock().unwrap().activate(&token,Instant::now());
                    let _=writeln!(pipe,"{}",if accepted {"\"accepted\""} else {"\"expired\""});
                    let _=super::windows_ipc::read_line(&mut pipe,Instant::now()+Duration::from_millis(100));
                }
                unsafe {let _=DisconnectNamedPipe(handle);}
            }
            let removed={let mut s=shared.store.lock().unwrap();s.prune(Instant::now());std::mem::take(&mut s.removals)};
            for tag in removed {toasts::remove(&id,&tag);}
            if Instant::now()>=next_history {
                let checked=shared.store.lock().unwrap().history_targets();
                if !checked.is_empty() {
                    if let Ok(present)=toasts::history_tags(&id) {shared.store.lock().unwrap().reconcile(&checked,&present);}
                }
                next_history=Instant::now()+Duration::from_secs(5);
            }
            let state=shared.store.lock().unwrap();
            if state.notices.is_empty() && state.removals.is_empty() {
                // Expired tokens need no reply; their windowless helper has a
                // bounded timeout. New notices and shutdown wake this worker.
                let _state=shared.wake.wait_while(state,|s|shared.running.load(Ordering::Acquire)
                    && s.notices.is_empty() && s.removals.is_empty()).unwrap();
            } else {
                drop(state);std::thread::sleep(Duration::from_millis(100));
            }
        }
        let removed={let mut s=shared.store.lock().unwrap();let tags=s.notices.drain().map(|(_,n)|n.physical).collect::<Vec<_>>();s.events.clear();s.removals.extend(tags);std::mem::take(&mut s.removals)};
        for tag in removed {toasts::remove(&id,&tag);}
    }).map_err(|e|e.to_string())?;
    let result=match recv.recv_timeout(Duration::from_secs(3)) {
        Ok(result)=>result,Err(_)=>{c.stop();return Err("notification activation startup timed out".into());}
    };
    if !result.is_empty() {let _=thread.join();return Err(result);}
    *c.thread.lock().unwrap()=Some(thread);let _=CONTROL.set(c.clone());Ok(c)
}
fn actions(value:&SysValue)->Result<Vec<(String,String)>,String> {
    let SysValue::List(list)=value else {return Err("notification actions need a list of [key, label] pairs".into());};
    if list.is_empty() || list.len()>4 {return Err("use 1–4 notification action buttons".into());}
    let mut result=Vec::new();
    for value in list {
        let SysValue::List(pair)=value else {return Err("notification action needs [key, label]".into());};
        let [SysValue::Text(key),SysValue::Text(label)]=pair.as_slice() else {return Err("notification action key and label must be text".into());};
        if matches!(key.as_str(),"default"|"dismissed") || !toasts::valid_tag(key) || label.trim().is_empty() || !toasts::valid_text(label,80)
            || result.iter().any(|(old,_)|old==key) {return Err("invalid or duplicate notification action key/label".into());}
        result.push((key.clone(),label.clone()));
    }
    Ok(result)
}
fn document(id:&str,title:&str,body:&str,buttons:&[(String,String)],tokens:&[(String,String)])->Result<windows::Data::Xml::Dom::XmlDocument,String> {
    let doc=toasts::document(title,body).map_err(|e|e.to_string())?;
    let root=doc.DocumentElement().map_err(|e|e.to_string())?;
    root.SetAttribute(&HSTRING::from("activationType"),&HSTRING::from("protocol")).map_err(|e|e.to_string())?;
    root.SetAttribute(&HSTRING::from("launch"),&HSTRING::from(super::windows_toast_protocol::uri(id,&tokens[0].0))).map_err(|e|e.to_string())?;
    let nodes=doc.CreateElement(&HSTRING::from("actions")).map_err(|e|e.to_string())?;
    for ((_,label),(token,_)) in buttons.iter().zip(tokens.iter().skip(1)) {
        let node=doc.CreateElement(&HSTRING::from("action")).map_err(|e|e.to_string())?;
        let uri=super::windows_toast_protocol::uri(id,token);
        for (name,value) in [("content",label.as_str()),("arguments",uri.as_str()),("activationType","protocol")] {
            node.SetAttribute(&HSTRING::from(name),&HSTRING::from(value)).map_err(|e|e.to_string())?;
        }
        nodes.AppendChild(&node).map_err(|e|e.to_string())?;
    }
    root.AppendChild(&nodes).map_err(|e|e.to_string())?;
    Ok(doc)
}
pub(super) fn publish(owner:&str,args:&[SysValue])->Result<(),String> {
    if args.len()==3 {return toasts::publish(args);}
    let [title,body,tag,buttons]=args else {return Err("notifications.publish takes title, body, tag, and optional actions".into());};
    let (title,body,tag)=toasts::validate(&[title.clone(),body.clone(),tag.clone()])
        .map(|(a,b,c)|(a.to_owned(),b.to_owned(),c.to_owned()))?;
    let buttons=actions(buttons)?;
    let _apartment=super::windows_system::Apartment::new()?;
    let owner=Owner{name:owner.into(),lifetime:super::windows_capture::service_lifetime()};
    if !owner.live() {return Err("notification request cancelled by reload".into());}
    let id=toasts::app_id()?;
    let engine=std::env::current_exe().map_err(|e|e.to_string())?;
    if !activation::ready(&engine,&id.to_string()) {return Err("actionable notifications require the installed shortcut and native notification broker".into());}
    let c=start(&id.to_string())?;
    let physical=activation::random()?[..16].to_owned();
    let mut tokens=Vec::new();
    for key in std::iter::once("default").chain(buttons.iter().map(|(key,_)|key.as_str())) {
        tokens.push((format!("v1.{}.{}",c.route,activation::random()?),key.to_owned()));
    }
    let doc=document(&id.to_string(),&title,&body,&buttons,&tokens)?;
    let delivered=Arc::new(AtomicBool::new(false));
    {let mut s=c.store.lock().unwrap();s.prune(Instant::now());s.cancel(&owner.name,Some(&tag));
        if s.notices.len()+s.events.len()+s.removals.len()>=LIMIT {return Err("too many outstanding notification actions".into());}
        if !owner.live() {return Err("notification request cancelled by reload".into());}
        s.notices.insert(physical.clone(),Notice{owner:owner.clone(),tag:tag.clone(),physical:physical.clone(),tokens,until:Instant::now()+EXPIRY,delivered:delivered.clone()});}
    c.wake.notify_one();
    let result=toasts::deliver(&id,&physical,doc,Some(&delivered));
    if result.is_ok() {delivered.store(true,Ordering::Release);}
    let remove={let mut s=c.store.lock().unwrap();
        if result.is_err() || !owner.live() {s.cancel_publication(&physical);}
        !s.notices.contains_key(&physical)};
    // A replaced request may finish after its queued removal already ran. Retire
    // that physical toast again without cancelling the newer logical tag.
    if remove {toasts::remove(&id.to_string(),&physical);}
    result
}
pub(super) fn query(owner:&str,args:&[SysValue])->Result<SysValue,String> {
    if !args.is_empty() {return Err("notifications.actions takes no arguments".into());}
    Ok(CONTROL.get().map_or(SysValue::List(vec![]),|c|c.store.lock().unwrap().drain(owner,Instant::now())))
}
pub(super) fn cancel(owner:&str,args:&[SysValue])->Result<(),String> {
    let [SysValue::Text(tag)]=args else {return Err("notifications.cancel takes an owned actionable tag".into());};
    if let Some(c)=CONTROL.get() {c.store.lock().unwrap().cancel(owner,Some(tag));c.wake.notify_one();}Ok(())
}
pub(crate) fn release(owner:&str) {if let Some(c)=CONTROL.get(){c.store.lock().unwrap().cancel(owner,None);c.wake.notify_one();}}
pub(super) fn shutdown()->bool {
    let Some(c)=CONTROL.get() else {return true;};c.stop();
    if let Some(thread)=c.thread.lock().unwrap().take() {
        let until=Instant::now()+Duration::from_secs(2);
        while !thread.is_finished() && Instant::now()<until {std::thread::sleep(Duration::from_millis(10));}
        if thread.is_finished() {return thread.join().is_ok();}
        eprintln!("windows · notification cleanup did not finish before shutdown");return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "subprocess entry point for the isolated COM activation test"]
    fn activation_child() {
        use windows::{core::HSTRING, Win32::{System::Com::*, UI::Notifications::INotificationActivationCallback}};
        let id=std::env::var("PLEAMAR_TEST_TOAST_APP").unwrap();
        let token=std::env::var("PLEAMAR_TEST_TOAST_TOKEN").unwrap();
        let _apartment=super::super::windows_system::Apartment::new().unwrap();
        unsafe {
            let callback:INotificationActivationCallback=CoCreateInstance(&activation::clsid(&id),None,CLSCTX_LOCAL_SERVER).unwrap();
            assert!(callback.Activate(&HSTRING::from("wrong app"),&HSTRING::from(&token),&[]).is_err());
            assert!(callback.Activate(&HSTRING::from(&id),&HSTRING::from("not an action"),&[]).is_err());
            callback.Activate(&HSTRING::from(&id),&HSTRING::from(&token),&[]).unwrap();
            callback.Activate(&HSTRING::from(&id),&HSTRING::from(&token),&[]).unwrap();
        }
    }
    #[test]
    fn native_com_activation_crosses_processes_without_a_toast_or_input() {
        use std::os::windows::process::CommandExt;
        let id=format!("org.pleamar.activation-test.{}",activation::random().unwrap());
        let c=start(&id).unwrap();
        std::thread::sleep(Duration::from_millis(250)); // Exercise a new action after the receiver becomes idle.
        let token=format!("v1.{}.{}",c.route,activation::random().unwrap());
        let physical=activation::random().unwrap()[..16].to_owned();
        let mut n=notice("owned COM test","logical",Arc::new(AtomicBool::new(true)),Instant::now()+EXPIRY);
        n.physical=physical.clone();n.tokens=vec![(token.clone(),"now".into())];
        c.store.lock().unwrap().notices.insert(physical,n);
        c.wake.notify_one();
        let mut child=std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact","platform::windows_toast_actions::tests::activation_child","--ignored","--nocapture"])
            .env("PLEAMAR_TEST_TOAST_APP",&id).env("PLEAMAR_TEST_TOAST_TOKEN",&token)
            .creation_flags(0x08000000 | 0x00004000).spawn().unwrap();
        let until=Instant::now()+Duration::from_secs(15);
        let status=loop {if let Some(status)=child.try_wait().unwrap(){break status;}
            if Instant::now()>=until {let _=child.kill();let _=child.wait();shutdown();panic!("COM activation child timed out");}
            std::thread::sleep(Duration::from_millis(20));};
        let result=query("owned COM test",&[]).unwrap();assert!(shutdown(),"idle notification worker did not exit");
        assert!(status.success());
        assert!(matches!(result,SysValue::List(v) if v==vec![SysValue::Map(vec![("tag".into(),SysValue::Text("logical".into())),("action".into(),SysValue::Text("now".into()))])]),"one native action must survive a duplicate COM activation");
    }
    #[test]
    #[ignore = "requires an owned installed protocol and paired native broker; sends no toast or input"]
    fn protocol_activation_child() {
        use windows::{core::PCWSTR, Win32::{Foundation::CloseHandle, System::Threading::{WaitForSingleObject,GetExitCodeProcess}, UI::Shell::*}};
        #[link(name="kernel32")]
        unsafe extern "system" {fn QueryThreadCycleTime(thread:HANDLE,cycles:*mut u64)->i32;}
        let id=std::env::var("PLEAMAR_TEST_TOAST_APP").unwrap();
        let engine=std::path::PathBuf::from(std::env::var_os("PLEAMAR_TEST_TOAST_ENGINE").unwrap());
        assert!(activation::ready(&engine,&id));
        let _apartment=super::super::windows_system::Apartment::new().unwrap();
        let c=start(&id).unwrap();
        for cycle in 0..2 {
            std::thread::sleep(Duration::from_millis(250));
            let cycles=|| {let thread=c.thread.lock().unwrap();let handle=HANDLE(thread.as_ref().unwrap().as_raw_handle());
                let mut count=0;assert_ne!(unsafe {QueryThreadCycleTime(handle,&mut count)},0,"QueryThreadCycleTime failed");count};
            let before=cycles();std::thread::sleep(Duration::from_millis(350));
            println!("idle receiver cycle {cycle}: {} CPU cycles during 350 ms",cycles()-before);
            let token=format!("v1.{}.{}",c.route,activation::random().unwrap());
            let physical=activation::random().unwrap()[..16].to_owned();
            let mut n=notice("owned protocol test","logical",Arc::new(AtomicBool::new(true)),Instant::now()+EXPIRY);
            n.physical=physical.clone();n.tokens=vec![(token.clone(),"now".into())];
            c.store.lock().unwrap().notices.insert(physical,n);
            c.wake.notify_one();
            let uri=super::super::windows_toast_protocol::uri(&id,&token).encode_utf16().chain([0]).collect::<Vec<_>>();
            // The replay reaches an idle receiver and its client times out. A
            // new notice must still work after that client has disconnected.
            for _ in 0..2 {unsafe {
                let mut info=SHELLEXECUTEINFOW{cbSize:std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
                    fMask:SEE_MASK_NOCLOSEPROCESS|SEE_MASK_NOASYNC|SEE_MASK_FLAG_NO_UI,
                    lpFile:PCWSTR(uri.as_ptr()),nShow:0,..Default::default()};
                ShellExecuteExW(&mut info).unwrap();assert!(!info.hProcess.is_invalid());
                let waited=WaitForSingleObject(info.hProcess,5000);
                let mut code=1;let result=GetExitCodeProcess(info.hProcess,&mut code);let _=CloseHandle(info.hProcess);
                assert_eq!(waited,windows::Win32::Foundation::WAIT_OBJECT_0);result.unwrap();assert_eq!(code,0);
                std::thread::sleep(Duration::from_millis(250));
            }}
            assert_eq!(query("owned protocol test",&[]).unwrap(),SysValue::List(vec![SysValue::Map(vec![("tag".into(),SysValue::Text("logical".into())),("action".into(),SysValue::Text("now".into()))])]));
        }
        assert!(shutdown(),"idle notification worker did not exit");
    }
    fn notice(owner:&str,tag:&str,active:Arc<AtomicBool>,until:Instant)->Notice {
        Notice{owner:Owner{name:owner.into(),lifetime:Some(active)},tag:tag.into(),physical:tag.into(),
            tokens:vec![(format!("{tag}-now"),"now".into()),(format!("{tag}-skip"),"skip".into())],until,delivered:Arc::new(AtomicBool::new(false))}
    }
    #[test]
    fn buttons_are_one_use_scene_scoped_and_revoked_on_reload() {
        let active=Arc::new(AtomicBool::new(true));let now=Instant::now();let mut s=Store::default();
        s.notices.insert("a".into(),notice("scene A","a",active.clone(),now+EXPIRY));
        assert!(!s.activate("unknown",now));assert!(s.activate("a-now",now));assert!(!s.activate("a-skip",now));
        assert_eq!(s.drain("scene B",now),SysValue::List(vec![]));
        assert!(matches!(s.drain("scene A",now),SysValue::List(v) if v.len()==1));
        assert_eq!(s.drain("scene A",now),SysValue::List(vec![]));
        s.notices.insert("b".into(),notice("scene A","b",active.clone(),now+EXPIRY));
        active.store(false,Ordering::Release);assert!(!s.activate("b-now",now));assert!(s.notices.is_empty());
        assert_eq!(s.removals,vec!["a","b"]);
    }
    #[test]
    fn cancellation_and_expiry_cannot_reach_replacement_callbacks() {
        let active=Arc::new(AtomicBool::new(true));let now=Instant::now();let mut s=Store::default();
        s.notices.insert("a".into(),notice("one","a",active.clone(),now));assert!(!s.activate("a-now",now));
        s.notices.insert("b".into(),notice("two","b",active.clone(),now+EXPIRY));s.cancel("one",None);
        assert!(s.activate("b-now",now));s.cancel("two",Some("b"));assert!(s.events.is_empty());
        s.notices.insert("c".into(),notice("two","c",active.clone(),now+EXPIRY));assert!(s.activate("c-now",now));
        active.store(false,Ordering::Release);assert_eq!(s.drain("two",now),SysValue::List(vec![]));
    }
    #[test]
    fn late_publication_failure_preserves_the_replacement_action() {
        let active=Arc::new(AtomicBool::new(true));let now=Instant::now();let mut s=Store::default();
        let mut old=notice("one","same",active.clone(),now+EXPIRY);old.physical="old".into();old.tokens=vec![("old-token".into(),"now".into())];
        s.notices.insert("old".into(),old);assert!(s.activate("old-token",now));
        let mut new=notice("one","same",active,now+EXPIRY);new.physical="new".into();new.tokens=vec![("new-token".into(),"now".into())];
        s.notices.insert("new".into(),new);
        s.cancel_publication("old");assert!(s.events.is_empty());assert!(s.notices.contains_key("new"));
        assert!(s.activate("new-token",now));s.cancel_publication("old");
        assert_eq!(s.events.len(),1);assert_eq!(s.events[0].physical,"new");
    }
    #[test]
    fn validates_button_contract_before_starting_native_services() {
        let pair=|key:&str,label:&str|SysValue::List(vec![SysValue::Text(key.into()),SysValue::Text(label.into())]);
        assert!(actions(&SysValue::List(vec![pair("now","Hacerlo ahora · 海 & < >")])).is_ok());
        for bad in [SysValue::Null,SysValue::List(vec![]),SysValue::List(vec![pair("default","Open")]),
            SysValue::List(vec![pair("a","one"),pair("a","two")]),SysValue::List(vec![pair("a","bad\0")]),
            SysValue::List(vec![pair("x","x");5])] {assert!(actions(&bad).is_err());}
    }
    #[test]
    fn action_xml_preserves_literal_unicode_labels_and_opaque_arguments() {
        let _apartment=super::super::windows_system::Apartment::new().unwrap();
        let label="Hacerlo · 海 & <action/> \" '";
        let tokens=vec![("v1.default".into(),"default".into()),("v1.button".into(),"now".into())];
        let doc=document("owned","<title>","España & 🚀",&[("now".into(),label.into())],&tokens).unwrap();
        let nodes=doc.GetElementsByTagName(&HSTRING::from("action")).unwrap();assert_eq!(nodes.Length().unwrap(),1);
        let attrs=nodes.Item(0).unwrap().Attributes().unwrap();
        assert_eq!(attrs.GetNamedItem(&HSTRING::from("content")).unwrap().InnerText().unwrap(),label);
        assert_eq!(attrs.GetNamedItem(&HSTRING::from("arguments")).unwrap().InnerText().unwrap(),super::super::windows_toast_protocol::uri("owned","v1.button"));
        assert_eq!(attrs.GetNamedItem(&HSTRING::from("activationType")).unwrap().InnerText().unwrap(),"protocol");
        let root=doc.DocumentElement().unwrap();
        assert_eq!(root.GetAttribute(&HSTRING::from("launch")).unwrap(),super::super::windows_toast_protocol::uri("owned","v1.default"));
        assert_eq!(root.GetAttribute(&HSTRING::from("activationType")).unwrap(),"protocol");
    }
    #[test]
    fn os_dismissal_retires_only_notices_in_the_confirmed_snapshot() {
        let active=Arc::new(AtomicBool::new(true));let now=Instant::now();let mut s=Store::default();
        let first=notice("one","a",active.clone(),now+EXPIRY);first.delivered.store(true,Ordering::Release);
        s.notices.insert("a".into(),first);
        s.notices.insert("b".into(),notice("one","b",active.clone(),now+EXPIRY));
        let checked=s.history_targets();assert_eq!(checked,vec!["a"]);
        s.notices.get("b").unwrap().delivered.store(true,Ordering::Release);
        s.reconcile(&checked,&HashSet::new());
        assert!(s.notices.contains_key("b") && !s.notices.contains_key("a"));
        assert_eq!(s.events.len(),1);assert_eq!(s.events[0].action,"dismissed");
        assert!(!s.activate("a-now",now));assert!(s.activate("b-now",now));
        assert_eq!(s.events.len(),2);
    }
}
