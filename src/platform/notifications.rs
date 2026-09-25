//! The desktop's notifications. Here nobody is listened to: **we are** the
//! server. Applications call `org.freedesktop.Notifications.Notify`, and
//! whoever holds that name on the bus is the one who shows them. There can be
//! only one: if someone else already has it (another bar, a daemon), the service
//! says it is not available.
//!
//! On Windows it will be `UserNotificationListener`; macOS does not let you read other apps' ones.
//!
//! `{ { id, app, title, body, icon, urgency, actions = { { key, label }, … } }, … }`,
//! newest first. `urgency`: 0 low, 1 normal, 2 critical.

use super::SysValue;
use std::collections::HashMap;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use zbus::zvariant::OwnedValue;

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
/// How long one lasts when it does not say how long it wants to last.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(6);

struct Notification {
    id: u32,
    app: String,
    title: String,
    body: String,
    icon: String,
    urgency: u8,
    actions: Vec<(String, String)>,
    expires: Option<Instant>,
}

/// What has to be told to the bus, or done, outside the call that caused it.
enum Event {
    Change,
    Closed(u32, u32),
    Action(u32, String),
}

#[derive(Default)]
struct State {
    notifications: Vec<Notification>,
    next: u32,
    /// Whether notifications stay until someone deals with them. Expiring after
    /// six seconds is what a bubble bar does; a notification CENTER —a history,
    /// a tray— keeps them, and the spec leaves it in the server's hands. And it
    /// is not only how long they are seen: once it is closed, the application
    /// stops listening for its action, so an expired one can no longer be opened.
    keep: bool,
}

struct Hub {
    state: Mutex<State>,
    events: Mutex<Sender<Event>>,
}

static HUB: OnceLock<Arc<Hub>> = OnceLock::new();

impl Hub {
    fn send(&self, c: Event) {
        let _ = self.events.lock().unwrap().send(c);
    }

    /// Why it is closed: 1 it expired, 2 the user closed it, 3 the application asked.
    fn close(&self, id: u32, reason: u32) -> bool {
        let mut e = self.state.lock().unwrap();
        let before = e.notifications.len();
        e.notifications.retain(|a| a.id != id);
        let was_there = e.notifications.len() != before;
        drop(e);
        if was_there {
            self.send(Event::Closed(id, reason));
        }
        was_there
    }

    fn report(&self) -> SysValue {
        let text = |s: &str| SysValue::Text(s.to_owned());
        SysValue::List(self.state.lock().unwrap().notifications.iter().rev().map(|a| SysValue::Map(vec![
            ("id".into(), SysValue::Num(a.id as f64)),
            ("app".into(), text(&a.app)),
            ("title".into(), text(&a.title)),
            ("body".into(), text(&a.body)),
            ("icon".into(), text(&a.icon)),
            ("urgency".into(), SysValue::Num(a.urgency as f64)),
            ("actions".into(), SysValue::List(a.actions.iter().map(|(k, l)| SysValue::Map(vec![("key".into(), text(k)), ("label".into(), text(l))])).collect())),
        ])).collect())
    }
}

/// We say we do not understand markup, but some send it anyway.
fn strip_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut inside = false;
    for c in s.chars() {
        match c {
            '<' => inside = true,
            '>' if inside => inside = false,
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'")
}

struct Server(Arc<Hub>);

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    #[allow(clippy::too_many_arguments)]
    fn notify(&self, app_name: &str, replaces_id: u32, app_icon: &str, summary: &str, body: &str, actions: Vec<String>, hints: HashMap<String, OwnedValue>, expire_timeout: i32) -> u32 {
        let hint = |k: &str| hints.get(k).and_then(|v| <&str>::try_from(v).ok()).map(str::to_owned);
        let icon = if app_icon.is_empty() { hint("image-path").or_else(|| hint("image_path")).unwrap_or_default() } else { app_icon.to_owned() };
        let urgency = hints.get("urgency").and_then(|v| u8::try_from(v).ok()).unwrap_or(1);
        let mut e = self.0.state.lock().unwrap();
        // With `replaces_id`, the same notification changing: the volume going up, a download progressing.
        let id = if replaces_id != 0 && e.notifications.iter().any(|a| a.id == replaces_id) {
            e.notifications.retain(|a| a.id != replaces_id);
            replaces_id
        } else {
            e.next += 1;
            e.next
        };
        let expires = match expire_timeout {
            _ if e.keep => None,
            0 => None,
            // Critical ones do not go away on their own, whatever they say.
            _ if urgency >= 2 => None,
            ms if ms > 0 => Some(Instant::now() + Duration::from_millis(ms as u64)),
            _ => Some(Instant::now() + DEFAULT_TIMEOUT),
        };
        e.notifications.push(Notification {
            id,
            app: app_name.to_owned(),
            title: strip_markup(summary),
            body: strip_markup(body),
            icon: icon.trim_start_matches("file://").to_owned(),
            urgency,
            // They come in pairs: key, label.
            actions: actions.chunks(2).filter(|p| p.len() == 2).map(|p| (p[0].clone(), p[1].clone())).collect(),
            expires,
        });
        drop(e);
        self.0.send(Event::Change);
        id
    }

    fn close_notification(&self, id: u32) {
        if self.0.close(id, 3) {
            self.0.send(Event::Change);
        }
    }

    fn get_capabilities(&self) -> Vec<String> {
        vec!["body".into(), "actions".into(), "icon-static".into(), "persistence".into()]
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        ("pleamar".into(), "k4ditano".into(), env!("CARGO_PKG_VERSION").into(), "1.2".into())
    }
}

pub fn service(dispatch: Box<dyn Fn(SysValue) + Send>) -> bool {
    let (tx, events) = channel();
    let hub = Arc::new(Hub { state: Mutex::default(), events: Mutex::new(tx) });
    // Without queueing or taking it from anybody: if the name has an owner, there is no service here.
    let connection = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.serve_at(PATH, Server(hub.clone())))
        .and_then(|b| b.build())
        .and_then(|c| c.request_name_with_flags(NAME, zbus::fdo::RequestNameFlags::DoNotQueue.into()).map(|_| c));
    let connection = match connection {
        Ok(c) => c,
        Err(e) => {
            eprintln!("notifications · I cannot be the one receiving notifications ({e}): does another program already have them?");
            return false;
        }
    };
    let _ = HUB.set(hub.clone());
    std::thread::Builder::new().name("notifications".into()).spawn(move || {
        // Only reported if the list is different: discarding what is no longer there is not news.
        let last = std::cell::RefCell::new(String::new());
        let dispatch = |v: SysValue| {
            let fingerprint = format!("{v:?}");
            if *last.borrow() != fingerprint {
                *last.borrow_mut() = fingerprint;
                dispatch(v);
            }
        };
        dispatch(hub.report());
        loop {
            // Sleeps until something happens or it is the next one's turn to expire.
            let now = Instant::now();
            let wait = hub.state.lock().unwrap().notifications.iter().filter_map(|a| a.expires).min().map(|c| c.saturating_duration_since(now));
            let event = match wait {
                Some(d) => events.recv_timeout(d),
                None => events.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match event {
                Ok(Event::Change) => dispatch(hub.report()),
                Ok(Event::Closed(id, reason)) => {
                    let _ = connection.emit_signal(None::<&str>, PATH, NAME, "NotificationClosed", &(id, reason));
                }
                Ok(Event::Action(id, key)) => {
                    let _ = connection.emit_signal(None::<&str>, PATH, NAME, "ActionInvoked", &(id, key.as_str()));
                }
                Err(RecvTimeoutError::Timeout) => {
                    let now = Instant::now();
                    let expired: Vec<u32> = hub.state.lock().unwrap().notifications.iter().filter(|a| a.expires.is_some_and(|c| c <= now)).map(|a| a.id).collect();
                    for id in expired {
                        hub.close(id, 1);
                    }
                    dispatch(hub.report());
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }).is_ok()
}

/// `notifications.dismiss(id)`, `notifications.invoke(id, "default")`, `notifications.clear()`,
/// and `notifications.keep(true)`: so they do not expire on their own, which is what a notification center wants.
pub fn command(what: &str, args: &[SysValue]) -> Result<(), String> {
    let hub = HUB.get().ok_or("the notifications service is not running: sys.watch(\"notifications\", …) is missing")?;
    match (what, args) {
        ("notifications.dismiss", [SysValue::Num(id)]) => {
            hub.close(*id as u32, 2);
        }
        ("notifications.invoke", [SysValue::Num(id), SysValue::Text(key)]) => {
            // The application learns which button it was, and the notification has done its job.
            hub.send(Event::Action(*id as u32, key.clone()));
            hub.close(*id as u32, 2);
        }
        ("notifications.keep", [SysValue::Bool(yes)]) => {
            let mut e = hub.state.lock().unwrap();
            e.keep = *yes;
            if *yes {
                // And the ones that were already counting down stop counting.
                e.notifications.iter_mut().for_each(|a| a.expires = None);
            }
        }
        ("notifications.clear", []) => {
            let ids: Vec<u32> = hub.state.lock().unwrap().notifications.iter().map(|a| a.id).collect();
            ids.into_iter().for_each(|id| { hub.close(id, 2); });
        }
        _ => return Err(format!("'{what}' is not asked like that: notifications.dismiss(id), notifications.invoke(id, key), notifications.keep(true), notifications.clear()")),
    }
    hub.send(Event::Change);
    Ok(())
}
