//! A scene's logic, in Luau. It lives on its own thread, in a sandbox: no
//! files and no system, with a memory cap and with its seconds counted. If it
//! gets stuck nobody notices —the render does not wait for it—, and if it stays
//! in a loop forever, it gets cut off.
//!
//! The only thing it can do is what crosses the frontier: say what is true,
//! what a text says, that something has happened; and hear what happens in the scene.
//!
//! ```lua
//! fact.open = true                         -- a fact
//! text["notice.title"] = "Meeting"         -- a live text
//! emit("confirmed")   play("joy")          -- an event, a gesture
//! on("view_event", function(n) … end)      -- an event from the scene, with its payload
//! on("press:view", …)  on("enter:orb", …)  on("layer:card", function(claim) … end)
//! on("fact:open", function(v) … end)       -- a rule changed a fact
//! local t = every(1000, function() … end)  after(500, …)  cancel(t)
//! run("date", {"+%H:%M"}, function(out, code) … end)   -- a system command
//! ```

use crate::scene::{intern, Event, Scene, ToRender};
use crate::logic::{Context, Script};
use crate::platform::SysValue;
use mlua::{Function, Lua, MultiValue, Table, Value, VmState};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Everything the logic has left running. When the program exits it has to be
/// stopped: a child does not die with its parent, and an orphaned `pactl subscribe`
/// would stay there forever.
static CHILDREN: Mutex<Vec<Arc<Mutex<Option<std::process::Child>>>>> = Mutex::new(Vec::new());

pub fn stop_children() {
    for h in CHILDREN.lock().unwrap().drain(..) {
        if let Some(mut h) = h.lock().unwrap().take() {
            let _ = h.kill();
            let _ = h.wait();
        }
    }
}

/// How long a handler may take before it gets cut off.
const PATIENCE: Duration = Duration::from_secs(2);
const MEMORY: usize = 64 << 20;

struct Timer {
    id: u32,
    when: Instant,
    every: Option<Duration>,
    f: Function,
}

#[derive(Default)]
struct Shared {
    handlers: HashMap<String, Vec<Function>>,
    timers: Vec<Timer>,
    processes: HashMap<u32, Function>,
    /// The commands still running: who gets told each line, and how to stop them.
    running: HashMap<u32, (Function, Arc<Mutex<Option<std::process::Child>>>)>,
    watchers: HashMap<String, Vec<Function>>,
    next: u32,
    facts: HashMap<String, f64>,
    texts: HashMap<String, String>,
    permissions: crate::scene::Permissions,
    models: Vec<crate::scene::Model>,
    /// Of the facts that are not plain numbers, what they are: the logic sees them as `true` or as `"critical"`.
    types: HashMap<String, crate::scene::FactType>,
    signals: std::collections::HashSet<String>,
    /// The services the scene asked for by name, and which fields it wants from each one.
    services: Vec<crate::scene::Service>,
    /// Which ones already got a thread: reloading the scene does not start them twice.
    subscribed: std::collections::HashSet<String>,
    /// If it is a plugin's logic, what it is called: so that errors talk about it and not about the scene.
    plugin: Option<String>,
    /// It asks for permissions nobody has approved: it runs with none, and the errors say why.
    unapproved: bool,
    /// Until when whatever is running may run.
    deadline: Option<Instant>,
}

/// Spreads a list of records over the texts and facts the scene has for it
/// —`rows.3.label`—, and the lists inside, the same way. Only what changes travels.
///
/// First it is read whole and then applied: if a record is wrong, the list that
/// was there stays as it was, not half-done.
fn spread_list(c: &mut Shared, tx: &Sender<ToRender>, prefix: &str, m: &crate::scene::Model, list: &Table) -> mlua::Result<()> {
    let mut changes = Vec::new();
    read_rows(&mut changes, prefix, m, list)?;
    for change in changes {
        match change {
            Change::Text(name, t) => if c.texts.get(&name) != Some(&t) {
                c.texts.insert(name.clone(), t.clone());
                let _ = tx.send(ToRender::Text(intern(&name), t));
            },
            Change::Fact(name, n) => if c.facts.get(&name) != Some(&n) {
                c.facts.insert(name.clone(), n);
                let _ = tx.send(ToRender::Fact(intern(&name), n as f32));
            },
        }
    }
    Ok(())
}

enum Change {
    Text(String, String),
    Fact(String, f64),
}

fn read_rows(changes: &mut Vec<Change>, prefix: &str, m: &crate::scene::Model, list: &Table) -> mlua::Result<()> {
    use crate::scene::{FactType, FieldType, FieldValue};
    let total = list.raw_len();
    let text = |c: &mut Vec<Change>, name: String, t: String| c.push(Change::Text(name, t));
    let fact = |c: &mut Vec<Change>, name: String, n: f64| c.push(Change::Fact(name, n));
    let c = changes;
    for i in 0..total.min(m.capacity) {
        let row: Value = list.raw_get(i + 1)?;
        let Value::Table(row) = row else {
            return Err(mlua::Error::runtime(format!("'{prefix}' is a list of records, and number {} is not a table", i + 1)));
        };
        for field in &m.fields {
            let name = format!("{prefix}.{i}.{}", field.name);
            let v: Value = row.get(field.name.as_str())?;
            match (&field.kind, &field.fallback) {
                (FieldType::List(inner), _) => match v {
                    Value::Table(t) => read_rows(c, &name, inner, &t)?,
                    // No list means an empty one: don't leave behind the one from the record that was here before.
                    _ => {
                        fact(c, format!("{name}.count"), 0.0);
                        fact(c, format!("{name}.total"), 0.0);
                    }
                },
                (FieldType::Text | FieldType::Image(..), fallback) => {
                    let t = match &v {
                        Value::Nil => if let FieldValue::Text(t) = fallback { t.clone() } else { String::new() },
                        other => other.to_string()?,
                    };
                    text(c, name, t);
                }
                (kind, fallback) => {
                    let missing = if let FieldValue::Number(d) = fallback { *d as f64 } else { 0.0 };
                    let n = match (kind, &v) {
                        (_, Value::Nil) => missing,
                        // A list where a number is expected counts as how many it has.
                        (FieldType::Number, Value::Table(t)) => t.raw_len() as f64,
                        (FieldType::Bool, Value::Table(t)) => (t.raw_len() > 0) as u8 as f64,
                        (FieldType::Bool, _) => to_number(&name, Some(&FactType::Bool), &v)?,
                        (FieldType::Enum(names), _) => to_number(&name, Some(&FactType::Enum(names.clone())), &v)?,
                        _ => to_number(&name, None, &v)?,
                    };
                    fact(c, name, n);
                }
            }
        }
    }
    fact(c, format!("{prefix}.count"), total.min(m.capacity) as f64);
    fact(c, format!("{prefix}.total"), total as f64);
    Ok(())
}

/// What the logic writes into a fact, as the number the scene sees. A `bool`
/// wants `true` or `false`; an enumeration, one of its names; the rest, a number.
fn to_number(name: &str, kind: Option<&crate::scene::FactType>, v: &Value) -> mlua::Result<f64> {
    use crate::scene::FactType;
    let number = match v {
        Value::Boolean(b) => Some(*b as u8 as f64),
        Value::Integer(i) => Some(*i as f64),
        Value::Number(x) => Some(*x),
        _ => None,
    };
    match (kind, v) {
        (Some(FactType::Enum(names)), Value::String(t)) => {
            let t = t.to_string_lossy();
            match names.iter().position(|n| *n == t) {
                Some(k) => Ok(k as f64),
                None => Err(mlua::Error::runtime(format!("'{name}' cannot be '{t}'{}: it can be {}", did_you_mean(&t, names.iter()), names.join(", ")))),
            }
        }
        (Some(FactType::Enum(names)), _) => match number {
            Some(n) if n >= 0.0 && (n as usize) < names.len() && n.fract() == 0.0 => Ok(n),
            _ => Err(mlua::Error::runtime(format!("'{name}' is an enum: it can be {}", names.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ")))),
        },
        (Some(FactType::Bool), _) => number.map(|n| (n != 0.0) as u8 as f64).ok_or_else(|| mlua::Error::runtime(format!("'{name}' is a yes or no: true or false, not a {}", v.type_name()))),
        (None, _) => number.ok_or_else(|| mlua::Error::runtime(format!("'{name}' is a number, not a {}", v.type_name()))),
    }
}

/// And the other way round: as the logic sees it.
fn from_number(lua: &Lua, kind: Option<&crate::scene::FactType>, v: f64) -> mlua::Result<Value> {
    use crate::scene::FactType;
    Ok(match kind {
        Some(FactType::Bool) => Value::Boolean(v > 0.5),
        Some(FactType::Enum(names)) => match names.get(v.round().max(0.0) as usize) {
            Some(n) => Value::String(lua.create_string(n)?),
            None => Value::Number(v),
        },
        None => Value::Number(v),
    })
}

fn describe(p: &crate::scene::Permissions) -> String {
    crate::permissions::describe(p)
}

/// Why not, told to the right party: the scene declares it; a plugin declares it and also has it approved.
fn denied(c: &Mutex<Shared>, what_for: &str, how: &str) -> mlua::Error {
    let c = c.lock().unwrap();
    mlua::Error::runtime(match &c.plugin {
        None => format!("the scene gives no permission to {what_for}. If it should be able to, declare it in the .plm: permissions {{ {how} }}"),
        Some(p) if c.unapproved => format!("plugin '{p}' wants to {what_for}, but nobody has approved its permissions: it runs with none. To see them and decide: pleamar --approve SCENE"),
        Some(p) => format!("plugin '{p}' has no permission to {what_for}. If it should be able to, declare it in its own .plm (the scene's do not count): permissions {{ {how} }}"),
    })
}

/// Can this logic launch that command? Undeclared, no; and the error says what to write.
fn check_command_permission(c: &Mutex<Shared>, command: &str) -> mlua::Result<()> {
    if c.lock().unwrap().permissions.commands.iter().any(|o| o == command) {
        return Ok(());
    }
    Err(denied(c, &format!("run '{command}'"), &format!("run: \"{command}\"")))
}

/// The same for a service. **Listening is not commanding**: `services: "audio"` lets you know the
/// volume (`sys.watch`, `sys.ask`); changing it takes `"audio.volume"`, or `"audio.*"`.
fn check_service_permission(c: &Mutex<Shared>, name: &str, commands: bool) -> mlua::Result<()> {
    let service = name.split(['.', ':']).next().unwrap_or(name);
    let has = |what: &str| c.lock().unwrap().permissions.services.iter().any(|s| s == what);
    if commands {
        if has(name) || has(&format!("{service}.*")) {
            return Ok(());
        }
        return Err(denied(c, &format!("ask the system for '{name}'"), &format!("services: \"{name}\"  (or \"{service}.*\" for everything of {service})")));
    }
    if has(service) || has(&format!("{service}.*")) {
        return Ok(());
    }
    Err(denied(c, &format!("use the '{service}' service"), &format!("services: \"{service}\"")))
}

/// What the logic hands to the system: numbers, texts and tables. A table with
/// keys 1..n is a list; any other, a map. Up to six levels: deeper than
/// that, what is being passed is not data.
fn lua_to_value(v: &Value, depth: usize) -> SysValue {
    match v {
        Value::Boolean(b) => SysValue::Bool(*b),
        Value::Integer(i) => SysValue::Num(*i as f64),
        Value::Number(n) => SysValue::Num(*n),
        Value::String(s) => SysValue::Text(s.to_string_lossy()),
        Value::Table(t) if depth < 6 => {
            let length = t.raw_len();
            let list: Vec<SysValue> = (1..=length).map(|k| lua_to_value(&t.raw_get::<Value>(k).unwrap_or(Value::Nil), depth + 1)).collect();
            // With part list and part map, the map wins: nothing is lost.
            let mut map: Vec<(String, SysValue)> = Vec::new();
            for pair in t.clone().pairs::<Value, Value>().flatten() {
                if let Value::String(k) = pair.0 {
                    map.push((k.to_string_lossy(), lua_to_value(&pair.1, depth + 1)));
                }
            }
            if map.is_empty() { SysValue::List(list) } else { SysValue::Map(map) }
        }
        _ => SysValue::Null,
    }
}

/// A piece of system data, as Luau sees it: tables, numbers, texts.
fn value_to_lua(lua: &Lua, v: &SysValue) -> mlua::Result<Value> {
    Ok(match v {
        SysValue::Null => Value::Nil,
        SysValue::Bool(b) => Value::Boolean(*b),
        SysValue::Num(n) => Value::Number(*n),
        SysValue::Text(s) => Value::String(lua.create_string(s)?),
        SysValue::List(l) => {
            let t = lua.create_table()?;
            for (k, x) in l.iter().enumerate() {
                t.set(k + 1, value_to_lua(lua, x)?)?;
            }
            Value::Table(t)
        }
        SysValue::Map(m) => {
            let t = lua.create_table()?;
            for (k, x) in m {
                t.set(k.as_str(), value_to_lua(lua, x)?)?;
            }
            Value::Table(t)
        }
    })
}

fn did_you_mean<'a>(k: &str, known: impl Iterator<Item = &'a String>) -> String {
    crate::language::closest_match(k, known).map_or(String::new(), |p| format!(". Did you mean '{p}'?"))
}

pub struct LuauScript {
    scene: String,
    logic: String,
    tx: Sender<ToRender>,
    to_logic: Sender<Event>,
    blocked: Arc<AtomicBool>,
    lua: Option<Lua>,
    c: Arc<Mutex<Shared>>,
    /// If it is a plugin's logic: its name. Everything it names goes under it (`Clock.now`),
    /// so it cannot touch —or hear— anything that is not its own.
    prefix: Option<String>,
    /// The scene's logic carries its plugins' logic with it: each one, its own Luau
    /// state **and its own thread**. One that gets stuck does not hold up the others, or the scene.
    plugins: Vec<LivePlugin>,
    /// If it is a plugin: what the scene says about it (its logic, what it asks for).
    definition: Option<crate::scene::Plugin>,
}

/// A running plugin, seen from the scene's logic: how it is spoken to.
/// Dropping the mailbox stops it: its thread ends, and with it whatever it left running.
struct LivePlugin {
    definition: crate::scene::Plugin,
    mailbox: Sender<Event>,
}

/// The real name of what a logic names: in a plugin, under its name.
fn qualified(prefix: &Option<String>, k: &str) -> String {
    match prefix {
        Some(p) => format!("{p}.{k}"),
        None => k.to_owned(),
    }
}

/// "It does not exist", told to the right party: a plugin is told about its own things, with the name it uses.
fn not_found<'a>(prefix: &Option<String>, what: &str, full: &str, known: impl Iterator<Item = &'a String>) -> mlua::Error {
    match prefix {
        None => mlua::Error::runtime(format!("the scene has {what} called '{full}'{}", did_you_mean(full, known))),
        Some(p) => {
            let short = full.strip_prefix(&format!("{p}.")).unwrap_or(full);
            let theirs: Vec<String> = known.filter_map(|k| k.strip_prefix(&format!("{p}.")).map(str::to_owned)).collect();
            mlua::Error::runtime(format!("plugin '{p}' has {what} called '{short}'{}. A plugin only sees what its library declares", did_you_mean(short, theirs.iter())))
        }
    }
}

/// The same for what is listened to: `fact:ticking` is `fact:Clock.ticking`, and `tapped`, `Clock.tapped`.
/// That way a plugin does not hear the keyboard, or the mouse, or the scene's events: only its own.
fn qualified_listener(prefix: &Option<String>, what: &str) -> String {
    match (prefix, what.split_once(':')) {
        (None, _) => what.to_owned(),
        (Some(p), Some((class, name))) => format!("{class}:{p}.{name}"),
        (Some(p), None) => format!("{p}.{what}"),
    }
}

impl LuauScript {
    pub fn new(scene: &str, logic: &str, tx: Sender<ToRender>, to_logic: Sender<Event>, blocked: Arc<AtomicBool>) -> Self {
        LuauScript { scene: scene.to_owned(), logic: logic.to_owned(), tx, to_logic, blocked, lua: None, c: Arc::default(), prefix: None, plugins: Vec::new(), definition: None }
    }

    /// A plugin's logic. The number is so that its timers and processes are not
    /// named the same as another logic's: they all share the mailbox.
    fn for_plugin(&self, p: &crate::scene::Plugin, number: usize) -> Self {
        let c = Shared { next: (number as u32 + 1) * 1_000_000, plugin: Some(p.name.clone()), ..Default::default() };
        LuauScript { scene: self.scene.clone(), logic: p.logic.to_string_lossy().into_owned(), tx: self.tx.clone(), to_logic: self.to_logic.clone(), blocked: self.blocked.clone(), lua: None, c: Arc::new(Mutex::new(c)), prefix: Some(p.name.clone()), plugins: Vec::new(), definition: Some(p.clone()) }
    }

    /// Sets a plugin running on its thread. From there it tends its mailbox and its timers.
    fn start(mut plugin: LuauScript) -> LivePlugin {
        let (mailbox, letters) = std::sync::mpsc::channel::<Event>();
        let definition = plugin.definition.clone().expect("only a plugin's logic is started");
        let name = format!("plugin {}", definition.name);
        let _ = std::thread::Builder::new().name(name).spawn(move || {
            let mut ctx = Context::for_plugin(plugin.tx.clone(), plugin.blocked.clone());
            loop {
                let wait = plugin.next_deadline().map_or(Duration::from_secs(3600), |p| p.saturating_duration_since(Instant::now()));
                match letters.recv_timeout(wait) {
                    Ok(e) => plugin.on_event(e, &mut ctx),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
                if plugin.next_deadline().is_some_and(|p| p <= Instant::now()) {
                    plugin.tick(&mut ctx);
                }
            }
            plugin.release();
        });
        LivePlugin { definition, mailbox }
    }

    /// Stops what this logic left running: timers, handlers, processes.
    fn release(&self) {
        let mut c = self.c.lock().unwrap();
        c.handlers.clear();
        c.timers.clear();
        c.processes.clear();
        c.watchers.clear();
        for (_, (_, child)) in c.running.drain() {
            if let Some(mut h) = child.lock().unwrap().take() {
                let _ = h.kill();
            }
        }
    }

    /// What the scene has, seen from this logic. A plugin sees all of it on the inside
    /// —it is checked against it—, but can only name what starts with its name.
    /// Whose the files are: the scene's, by the name of its `.plm`; or the plugin's, by its own.
    fn owner(&self) -> String {
        match &self.prefix {
            Some(p) => p.clone(),
            None => std::path::Path::new(&self.scene)
                .file_stem()
                .map_or_else(|| "scene".to_owned(), |n| n.to_string_lossy().into_owned()),
        }
    }

    fn learn(&self, e: &Scene, permissions: crate::scene::Permissions) {
        let mut c = self.c.lock().unwrap();
        c.facts = e.facts.iter().map(|(n, v)| (n.to_string(), *v as f64)).collect();
        c.texts = e.texts.iter().map(|(n, v)| (n.to_string(), v.clone())).collect();
        c.permissions = permissions;
        c.models = e.models.clone();
        c.types = e.types.iter().cloned().collect();
        c.signals = e.signals.iter().map(|s| s.0.to_owned()).collect();
        c.services = e.services.clone();
    }

    /// The services the scene asked for with `service`: one per thread, and whatever they report
    /// is spread on its own over the facts and texts that carry their name in front.
    /// The permissions are the scene's: `services: "clock"` for `service clock`.
    fn subscribe_services(&mut self) {
        // A plugin does not set up the scene's services: whatever it declares goes with the rest,
        // and asking for them is up to whoever owns the frontier.
        if self.prefix.is_some() {
            return;
        }
        let who = self.owner();
        let pending: Vec<crate::scene::Service> = {
            let c = self.c.lock().unwrap();
            c.services.iter().filter(|s| !c.subscribed.contains(&s.alias)).cloned().collect()
        };
        for s in pending {
            let root = s.name.split(['.', ':']).next().unwrap_or(&s.name).to_owned();
            let has = {
                let c = self.c.lock().unwrap();
                c.permissions.services.iter().any(|x| *x == root || *x == format!("{root}.*"))
            };
            if !has {
                eprintln!("logic  · `service {}` needs permission: add `services: \"{root}\"` to this scene's `permissions`", s.name);
                continue;
            }
            self.c.lock().unwrap().subscribed.insert(s.alias.clone());
            let (to_logic, alias) = (Mutex::new(self.to_logic.clone()), s.alias.clone());
            if !crate::platform::service(&who, &s.name, Box::new(move |v| {
                let _ = to_logic.lock().unwrap().send(Event::Data(format!("service:{alias}"), v));
            })) {
                eprintln!("logic  · the '{}' service is not available here: '{}' stays as the scene left it", s.name, s.alias);
            }
        }
    }

    /// What a service reports, spread over the fields the scene asked for. Whatever
    /// does not come stays as it was: a service may report only what changed.
    fn spread_service(&self, alias: &str, value: &SysValue) {
        let SysValue::Map(fields) = value else { return };
        let theirs = {
            let c = self.c.lock().unwrap();
            let Some(s) = c.services.iter().find(|s| s.alias == alias) else { return };
            s.fields.clone()
        };
        for field in &theirs {
            let Some((_, v)) = fields.iter().find(|(k, _)| *k == field.name) else { continue };
            let full = format!("{alias}.{}", field.name);
            match &field.kind {
                crate::scene::FieldType::Text | crate::scene::FieldType::Image(..) => {
                    let t = match v {
                        SysValue::Text(t) => t.clone(),
                        SysValue::Num(n) => if n.fract() == 0.0 { format!("{n:.0}") } else { format!("{n}") },
                        SysValue::Bool(b) => b.to_string(),
                        _ => continue,
                    };
                    let mut c = self.c.lock().unwrap();
                    if c.texts.get(&full).is_some_and(|already| *already == t) {
                        continue;
                    }
                    c.texts.insert(full.clone(), t.clone());
                    drop(c);
                    let _ = self.tx.send(ToRender::Text(intern(&full), t));
                }
                kind => {
                    let n = match (v, kind) {
                        (SysValue::Num(n), _) => *n,
                        (SysValue::Bool(b), _) => *b as u8 as f64,
                        // An enumeration arrives by its name: `kind = "wifi"`.
                        (SysValue::Text(t), crate::scene::FieldType::Enum(names)) => match names.iter().position(|x| x == t) {
                            Some(k) => k as f64,
                            None => continue,
                        },
                        (SysValue::Text(t), _) => match t.parse() {
                            Ok(n) => n,
                            Err(_) => continue,
                        },
                        _ => continue,
                    };
                    let mut c = self.c.lock().unwrap();
                    if c.facts.get(&full).is_some_and(|already| *already == n) {
                        continue;
                    }
                    c.facts.insert(full.clone(), n);
                    drop(c);
                    let _ = self.tx.send(ToRender::Fact(intern(&full), n as f32));
                }
            }
        }
    }

    /// A fresh Luau state, with the frontier in place, and the file executed.
    fn load_script(&mut self) {
        // Whatever the old logic left running is stopped along with it.
        self.release();
        // Said before loading: if the script trips over a permission it does not have, let it be known why.
        if let (Some(p), true) = (&self.prefix, self.c.lock().unwrap().unapproved) {
            println!("logic  · plugin '{p}' · ⚠ NOT APPROVED: runs unable to touch the system. To see what it asks for and decide: pleamar --approve {}", self.scene);
        }
        // A scene may have no logic of its own and yet have plugins that do.
        if self.prefix.is_none() && !std::path::Path::new(&self.logic).is_file() {
            return;
        }
        let source = match std::fs::read_to_string(&self.logic) {
            Ok(f) => f,
            Err(e) => return eprintln!("logic  · {}: {e}", self.logic),
        };
        let t0 = Instant::now();
        match self.prepare().and_then(|lua| {
            self.c.lock().unwrap().deadline = Some(Instant::now() + PATIENCE);
            lua.load(&source).set_name(format!("@{}", self.logic)).exec()?;
            Ok(lua)
        }) {
            Ok(lua) => {
                self.lua = Some(lua);
                let c = self.c.lock().unwrap();
                println!("logic  · {} running in {:.1} ms · {} handlers, {} timers", self.logic, t0.elapsed().as_secs_f32() * 1000.0, c.handlers.values().map(Vec::len).sum::<usize>(), c.timers.len());
                match &self.prefix {
                    Some(p) => println!("logic  · plugin '{p}' · permissions · {}", describe(&c.permissions)),
                    None => println!("logic  · permissions · {}", describe(&c.permissions)),
                }
            }
            Err(e) => eprintln!("logic  · the previous one stays as it was:\n{e}"),
        }
        self.c.lock().unwrap().deadline = None;
    }

    fn prepare(&self) -> mlua::Result<Lua> {
        let lua = Lua::new();
        lua.set_memory_limit(MEMORY)?;
        // In the sandbox, Luau assumes globals do not change and reads
        // `fact.open` ONCE, when loading the script. These do change: it has to be told.
        lua.set_compiler(mlua::chunk::Compiler::new().set_mutable_globals(["fact", "text", "model", "sys"]));
        let g = lua.globals();

        // A handler that never finishes cannot keep the thread forever.
        let c = self.c.clone();
        lua.set_interrupt(move |_| match c.lock().unwrap().deadline {
            Some(l) if Instant::now() > l => Err(mlua::Error::runtime(format!("a handler has been running for more than {} s: cut off", PATIENCE.as_secs()))),
            _ => Ok(VmState::Continue),
        });

        // fact.open = true · fact.open
        let (tx, c) = (self.tx.clone(), self.c.clone());
        let pre = self.prefix.clone();
        let set = lua.create_function(move |_, (_, k, v): (Table, String, Value)| {
            let k = qualified(&pre, &k);
            // A misspelled name is an error here, with its line, and not a warning lost in the render.
            if !c.lock().unwrap().facts.contains_key(&k) {
                return Err(not_found(&pre, "no fact", &k, c.lock().unwrap().facts.keys()));
            }
            let kind = c.lock().unwrap().types.get(&k).cloned();
            let n = to_number(&k, kind.as_ref(), &v)?;
            c.lock().unwrap().facts.insert(k.clone(), n);
            let _ = tx.send(ToRender::Fact(intern(&k), n as f32));
            Ok(())
        })?;
        let c = self.c.clone();
        let pre = self.prefix.clone();
        let get = lua.create_function(move |lua, (_, k): (Table, String)| {
            let k = qualified(&pre, &k);
            let c = c.lock().unwrap();
            match c.facts.get(&k) {
                Some(v) => from_number(lua, c.types.get(&k), *v),
                None => Ok(Value::Nil),
            }
        })?;
        g.set("fact", Self::live_table(&lua, get, set)?)?;

        // text["notice.title"] = "…"
        let (tx, c) = (self.tx.clone(), self.c.clone());
        let pre = self.prefix.clone();
        let set = lua.create_function(move |_, (_, k, v): (Table, String, String)| {
            let k = qualified(&pre, &k);
            if !c.lock().unwrap().texts.contains_key(&k) {
                return Err(not_found(&pre, "no text", &k, c.lock().unwrap().texts.keys()));
            }
            c.lock().unwrap().texts.insert(k.clone(), v.clone());
            let _ = tx.send(ToRender::Text(intern(&k), v));
            Ok(())
        })?;
        let c = self.c.clone();
        let pre = self.prefix.clone();
        let get = lua.create_function(move |_, (_, k): (Table, String)| Ok(c.lock().unwrap().texts.get(&qualified(&pre, &k)).cloned()))?;
        g.set("text", Self::live_table(&lua, get, set)?)?;

        // model.rows = { { label = "Open", enabled = true }, … }: a whole list, in one go.
        // Each field of each record is on the inside a text or a fact; here they are spread out,
        // and only what has changed travels.
        let stored = lua.create_table()?;
        let (tx, c, store) = (self.tx.clone(), self.c.clone(), stored.clone());
        let pre = self.prefix.clone();
        let set = lua.create_function(move |_, (_, k, list): (Table, String, Value)| {
            let k = qualified(&pre, &k);
            let Value::Table(list) = list else {
                return Err(mlua::Error::runtime(format!("'model.{k}' takes a list of records: model.{k} = {{ {{ … }}, {{ … }} }}")));
            };
            let mut c = c.lock().unwrap();
            let Some(m) = c.models.iter().find(|m| m.name == k).cloned() else {
                return Err(not_found(&pre, "no model", &k, c.models.iter().map(|m| &m.name)));
            };
            spread_list(&mut c, &tx, &k, &m, &list)?;
            store.raw_set(k, list)
        })?;
        let pre = self.prefix.clone();
        let get = lua.create_function(move |_, (_, k): (Table, String)| stored.raw_get::<Value>(qualified(&pre, &k)))?;
        g.set("model", Self::live_table(&lua, get, set)?)?;

        let tx = self.tx.clone();
        let (pre, c) = (self.prefix.clone(), self.c.clone());
        g.set("emit", lua.create_function(move |_, n: String| {
            let n = qualified(&pre, &n);
            // An event that does not exist used to be a warning lost in the render: here it is an error, with its line.
            if !c.lock().unwrap().signals.contains(&n) {
                let known: Vec<String> = c.lock().unwrap().signals.iter().cloned().collect();
                return Err(not_found(&pre, "no event", &n, known.iter()));
            }
            Ok(tx.send(ToRender::Signal(intern(&n))).is_ok())
        })?)?;
        // focus("query") puts the text cursor in a field; focus() removes it.
        let tx = self.tx.clone();
        let pre = self.prefix.clone();
        g.set("focus", lua.create_function(move |_, n: Option<String>| {
            if pre.is_some() {
                return Err(mlua::Error::runtime("a plugin does not move the text cursor: that belongs to the scene"));
            }
            Ok(tx.send(ToRender::FocusField(n.map(|n| intern(&n)))).is_ok())
        })?)?;
        let tx = self.tx.clone();
        let pre = self.prefix.clone();
        g.set("play", lua.create_function(move |_, n: String| {
            if pre.is_some() {
                return Err(mlua::Error::runtime("a plugin does not ask for gestures: gestures belong to the scene. Let it emit an event of its own, and the scene will decide"));
            }
            Ok(tx.send(ToRender::Gesture(intern(&n))).is_ok())
        })?)?;

        let c = self.c.clone();
        let pre = self.prefix.clone();
        g.set("on", lua.create_function(move |_, (what, f): (String, Function)| {
            let what = qualified_listener(&pre, &what);
            c.lock().unwrap().handlers.entry(what).or_default().push(f);
            Ok(())
        })?)?;

        let schedule = |c: &Arc<Mutex<Shared>>, ms: f64, f: Function, repeats: bool| {
            let mut c = c.lock().unwrap();
            c.next += 1;
            let d = Duration::from_secs_f64(ms.max(1.0) / 1000.0);
            let id = c.next;
            c.timers.push(Timer { id, when: Instant::now() + d, every: repeats.then_some(d), f });
            id
        };
        let c = self.c.clone();
        g.set("after", lua.create_function(move |_, (ms, f): (f64, Function)| Ok(schedule(&c, ms, f, false)))?)?;
        let c = self.c.clone();
        g.set("every", lua.create_function(move |_, (ms, f): (f64, Function)| Ok(schedule(&c, ms, f, true)))?)?;
        let c = self.c.clone();
        g.set("cancel", lua.create_function(move |_, id: u32| {
            c.lock().unwrap().timers.retain(|t| t.id != id);
            Ok(())
        })?)?;

        // Fake work, to see that the render does not care.
        let (c, blocked) = (self.c.clone(), self.blocked.clone());
        g.set("busy", lua.create_function(move |_, ms: f64| {
            let d = Duration::from_secs_f64(ms.max(0.0) / 1000.0);
            if let Some(l) = &mut c.lock().unwrap().deadline {
                *l += d;
            }
            blocked.store(true, Ordering::Relaxed);
            let end = Instant::now() + d;
            while Instant::now() < end {
                std::hint::spin_loop();
            }
            blocked.store(false, Ordering::Relaxed);
            Ok(())
        })?)?;

        // A system command: it runs on another thread and answers when it finishes.
        let (c, to_logic) = (self.c.clone(), self.to_logic.clone());
        // With a fourth argument, how: `{ stdin = "/path" }` gives it that file as
        // its input —what in a terminal is `command < file`, which is how an image
        // is copied to the clipboard— and `{ output = false }` does not collect what
        // it writes. The latter is not a whim: a program that stays in the background
        // (`wl-copy` does, to keep serving what was copied) inherits the output
        // pipe, and waiting for it to close it is waiting forever.
        g.set("run", lua.create_function(move |_, (command, args, f, how): (String, Option<Vec<String>>, Option<Function>, Option<mlua::Table>)| {
            check_command_permission(&c, &command)?;
            let input: Option<String> = how.as_ref().and_then(|t| t.get("stdin").ok());
            let collect: bool = how.as_ref().and_then(|t| t.get::<Option<bool>>("output").ok().flatten()).unwrap_or(true);
            let id = {
                let mut c = c.lock().unwrap();
                c.next += 1;
                let id = c.next;
                if let Some(f) = f {
                    c.processes.insert(id, f);
                }
                id
            };
            let to_logic = to_logic.clone();
            std::thread::spawn(move || {
                let mut launch = std::process::Command::new(&command);
                launch.args(args.unwrap_or_default());
                if let Some(path) = &input {
                    match std::fs::File::open(path) {
                        Ok(f) => {
                            launch.stdin(f);
                        }
                        Err(e) => {
                            let _ = to_logic.send(Event::Process(id, format!("{path}: {e}"), -1));
                            return;
                        }
                    }
                }
                let (output, code) = if collect {
                    match launch.output() {
                        Ok(o) => (String::from_utf8_lossy(&o.stdout).trim_end().to_owned(), o.status.code().unwrap_or(-1)),
                        Err(e) => (e.to_string(), -1),
                    }
                } else {
                    match launch.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status() {
                        Ok(s) => (String::new(), s.code().unwrap_or(-1)),
                        Err(e) => (e.to_string(), -1),
                    }
                };
                let _ = to_logic.send(Event::Process(id, output, code));
            });
            Ok(())
        })?)?;

        // A command that never finishes —`pactl subscribe`, `playerctl --follow`—: one
        // call for every line it writes, and `kill(id)` to stop it. With a
        // fourth function, that one is called when the process HAS FINISHED, with its
        // code: asking something to stop and it having stopped are not the same, and
        // that difference is where a recording gets lost.
        let (c, to_logic) = (self.c.clone(), self.to_logic.clone());
        g.set("spawn", lua.create_function(move |_, (command, args, f, on_exit): (String, Option<Vec<String>>, Function, Option<Function>)| {
            use std::io::BufRead;
            check_command_permission(&c, &command)?;
            let mut launch = std::process::Command::new(&command);
            launch.args(args.unwrap_or_default()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
            // If the program gets killed, this goes with it.
            crate::platform::die_with_parent(&mut launch);
            let mut child = launch
                .spawn()
                .map_err(|e| mlua::Error::runtime(format!("cannot run '{command}': {e}")))?;
            let output = child.stdout.take();
            let child = Arc::new(Mutex::new(Some(child)));
            CHILDREN.lock().unwrap().push(child.clone());
            let id = {
                let mut c = c.lock().unwrap();
                c.next += 1;
                let id = c.next;
                c.running.insert(id, (f, child.clone()));
                if let Some(end) = on_exit {
                    c.processes.insert(id, end);
                }
                id
            };
            let to_logic = to_logic.clone();
            std::thread::spawn(move || {
                if let Some(s) = output {
                    for line in std::io::BufReader::new(s).lines().map_while(Result::ok) {
                        if to_logic.send(Event::Line(id, line)).is_err() {
                            break;
                        }
                    }
                }
                let code = child.lock().unwrap().take().and_then(|mut h| h.wait().ok()).and_then(|s| s.code()).unwrap_or(-1);
                let _ = to_logic.send(Event::Process(id, String::new(), code));
            });
            Ok(id)
        })?)?;
        let c = self.c.clone();
        // `kill(id)` kills it and forgets about it. `kill(id, "int")` —or `"term"`— ASKS
        // it: it sends it the signal and keeps track of it until it leaves
        // on its own. A recorder that gets killed leaves an MP4 with no index, which
        // nobody can open; with SIGINT it writes it and exits.
        g.set("kill", lua.create_function(move |_, (id, how): (u32, Option<String>)| {
            let signal = match how.as_deref() {
                None => None,
                Some("int") => Some(libc::SIGINT),
                Some("term") => Some(libc::SIGTERM),
                Some(other) => return Err(mlua::Error::runtime(format!("kill(id, \"{other}\"): it is \"int\" or \"term\" to ask, or nothing to kill"))),
            };
            let mut c = c.lock().unwrap();
            match signal {
                Some(signal) => {
                    if let Some((_, child)) = c.running.get(&id) {
                        if let Some(h) = child.lock().unwrap().as_ref() {
                            #[cfg(unix)]
                            unsafe {
                                libc::kill(h.id() as libc::pid_t, signal);
                            }
                            let _ = (h, signal);
                        }
                    }
                }
                None => {
                    c.processes.remove(&id);
                    if let Some((_, child)) = c.running.remove(&id) {
                        if let Some(mut h) = child.lock().unwrap().take() {
                            let _ = h.kill();
                        }
                    }
                }
            }
            Ok(())
        })?)?;

        // What happens in the system, by a name that is the same everywhere.
        let sys = lua.create_table()?;
        let who = self.owner();
        let (c, to_logic, mine) = (self.c.clone(), self.to_logic.clone(), who.clone());
        sys.set("watch", lua.create_function(move |_, (name, f): (String, Function)| {
            check_service_permission(&c, &name, false)?;
            let first = {
                let mut c = c.lock().unwrap();
                let v = c.watchers.entry(name.clone()).or_default();
                v.push(f);
                v.len() == 1
            };
            if !first {
                return Ok(true);
            }
            let (to_logic, n) = (Mutex::new(to_logic.clone()), name.clone());
            Ok(crate::platform::service(&mine, &name, Box::new(move |v| {
                let _ = to_logic.lock().unwrap().send(Event::Data(n.clone(), v));
            })))
        })?)?;
        let (c, mine) = (self.c.clone(), who.clone());
        sys.set("call", lua.create_function(move |_, (name, args): (String, mlua::Variadic<Value>)| {
            check_service_permission(&c, &name, true)?;
            let args: Vec<SysValue> = args.iter().map(|v| lua_to_value(v, 0)).collect();
            crate::platform::command(&mine, &name, &args).map_err(mlua::Error::runtime)
        })?)?;
        // The same, but it answers: `sys.ask("tray.menu", key)` returns the menu.
        let (c, mine) = (self.c.clone(), who.clone());
        sys.set("ask", lua.create_function(move |lua, (name, args): (String, mlua::Variadic<Value>)| {
            check_service_permission(&c, &name, false)?;
            let args: Vec<SysValue> = args.iter().map(|v| lua_to_value(v, 0)).collect();
            // However long asking the system takes does NOT count against the
            // handler's patience: patience is there to cut off a loop
            // that has run wild, and waiting is not running wild. PAM charges two
            // seconds for a bad password, which is exactly the limit: the
            // handler got cut off halfway through, without getting to say "that
            // wasn't it", and the lock screen stayed "checking"
            // forever. It is the same thing `busy` already did.
            let t0 = Instant::now();
            let answer = crate::platform::query(&mine, &name, &args);
            if let Some(l) = &mut c.lock().unwrap().deadline {
                *l += t0.elapsed();
            }
            value_to_lua(lua, &answer.map_err(mlua::Error::runtime)?)
        })?)?;
        g.set("sys", sys)?;

        // require("util"): another file from the same folder as this logic, and from no other.
        // It is loaded once; whatever it returns is the module.
        let folder = std::path::Path::new(&self.logic).parent().map(std::path::Path::to_owned).unwrap_or_default();
        let loaded = lua.create_table()?;
        g.set("require", lua.create_function(move |lua, name: String| {
            let clean = !name.is_empty() && name.split('/').all(|t| !t.is_empty() && t != ".." && t != "." && t.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-'));
            if !clean {
                return Err(mlua::Error::runtime(format!("require(\"{name}\"): a module is a file in this logic's folder, or in one inside it. No `..`, no full paths and no extension")));
            }
            if let Ok(Value::Table(t)) = loaded.raw_get::<Value>(name.as_str()) {
                return t.raw_get::<Value>("value");
            }
            let path = folder.join(format!("{name}.luau"));
            let source = std::fs::read_to_string(&path).map_err(|e| mlua::Error::runtime(format!("require(\"{name}\"): {}: {e}", path.display())))?;
            let value: Value = lua.load(&source).set_name(format!("@{}", path.display())).eval()?;
            let slot = lua.create_table()?;
            slot.raw_set("value", value.clone())?;
            loaded.raw_set(name, slot)?;
            Ok(value)
        })?)?;
        let who = self.prefix.as_ref().map_or(String::new(), |p| format!("[{p}] "));
        g.set("log", lua.create_function(move |_, v: MultiValue| {
            let pieces: Vec<String> = v.iter().map(|x| x.to_string().unwrap_or_else(|_| format!("{x:?}"))).collect();
            println!("luau   · {who}{}", pieces.join(" "));
            Ok(())
        })?)?;

        // The sandbox, last of all: from here on the globals are not touched.
        lua.sandbox(true)?;
        Ok(lua)
    }

    /// A table that stores nothing: reading and writing it are calls.
    fn live_table(lua: &Lua, get: Function, set: Function) -> mlua::Result<Table> {
        let (t, meta) = (lua.create_table()?, lua.create_table()?);
        meta.set("__index", get)?;
        meta.set("__newindex", set)?;
        t.set_metatable(Some(meta))?;
        Ok(t)
    }

    fn call_handler(&self, f: &Function, args: impl mlua::IntoLuaMulti) {
        self.c.lock().unwrap().deadline = Some(Instant::now() + PATIENCE);
        if let Err(e) = f.call::<()>(args) {
            // Let it be known whose error it is: with plugins, "the logic" is several.
            match &self.prefix {
                Some(p) => eprintln!("logic  · plugin '{p}' · {e}"),
                None => eprintln!("logic  · {e}"),
            }
        }
        self.c.lock().unwrap().deadline = None;
    }

    fn dispatch_text(&self, what: &str, text: String) {
        let listeners: Vec<Function> = self.c.lock().unwrap().handlers.get(what).cloned().unwrap_or_default();
        for f in listeners {
            self.call_handler(&f, text.clone());
        }
    }

    fn dispatch(&self, what: &str, arg: Value) {
        let listeners: Vec<Function> = self.c.lock().unwrap().handlers.get(what).cloned().unwrap_or_default();
        for f in listeners {
            self.call_handler(&f, arg.clone());
        }
    }
}

impl Script for LuauScript {
    fn scene(&mut self) -> Scene {
        let e = super::scenes::from_file::read(&self.scene).unwrap_or_else(|m| {
            eprintln!("{m}");
            std::process::exit(1)
        });
        // What the scene takes as true at birth is what the logic believes until someone says otherwise.
        self.learn(&e, e.permissions.clone());
        self.plugins = e.plugins.iter().enumerate().map(|(k, p)| {
            let plugin = self.for_plugin(p, k);
            plugin.learn(&e, crate::permissions::effective(p));
            plugin.c.lock().unwrap().unapproved = !crate::permissions::is_approved(p);
            Self::start(plugin)
        }).collect();
        e
    }

    fn on_event(&mut self, e: Event, ctx: &mut Context) {
        // Every plugin receives the same, but only has handlers with its name in front:
        // it does not find out about what is not its own.
        if !matches!(e, Event::NewScene(..)) {
            for p in &self.plugins {
                let _ = p.mailbox.send(e.clone());
            }
        }
        let _ = &ctx;
        let number = |v: f32| Value::Number(v as f64);
        match e {
            // The file is executed right on the logic's thread, with the scene delivered.
            Event::Alarm("start") | Event::ReloadLogic => {
                self.load_script();
                self.subscribe_services();
            }
            Event::Signal(n, payload) => self.dispatch(n, payload.map_or(Value::Nil, number)),
            Event::Enter(z) => self.dispatch(&format!("enter:{z}"), Value::Nil),
            Event::Leave(z) => self.dispatch(&format!("leave:{z}"), Value::Nil),
            Event::Press(z) => self.dispatch(&format!("press:{z}"), Value::Nil),
            Event::Release(z) => self.dispatch(&format!("release:{z}"), Value::Nil),
            Event::Wheel(z, d) => self.dispatch(&format!("scroll:{z}"), number(d)),
            // What is typed into a field: the logic knows it key by key, and has not
            // had to do anything for it to show.
            Event::Text(n, value) => {
                self.c.lock().unwrap().texts.insert(n.to_owned(), value.clone());
                self.dispatch_text(&format!("text:{n}"), value);
            }
            Event::Submit(n, value) => self.dispatch_text(&format!("submit:{n}"), value),
            Event::Focus(yes) => self.dispatch(if yes { "focus" } else { "blur" }, Value::Nil),
            Event::Received(zone, kind, data) => {
                let listeners = self.c.lock().unwrap().handlers.get(&format!("drop:{zone}")).cloned().unwrap_or_default();
                listeners.iter().for_each(|f| self.call_handler(f, (data.clone(), kind.clone())));
            }
            Event::Key(name, typed) => {
                let Some(lua) = &self.lua else { return };
                let listeners = self.c.lock().unwrap().handlers.get("key").cloned().unwrap_or_default();
                if let Ok(n) = lua.create_string(&name) {
                    listeners.iter().for_each(|f| self.call_handler(f, (n.clone(), typed.clone())));
                }
            }
            Event::Demo => self.dispatch("demo", Value::Nil),
            Event::Fact(n, v) => {
                self.c.lock().unwrap().facts.insert(n.to_owned(), v as f64);
                // Whoever listens gets it the way they read it: `true`, `"critical"`, or a number.
                let kind = self.c.lock().unwrap().types.get(n).cloned();
                let value = match &self.lua {
                    Some(lua) => from_number(lua, kind.as_ref(), v as f64).unwrap_or(number(v)),
                    None => number(v),
                };
                self.dispatch(&format!("fact:{n}"), value);
            }
            Event::Layer(layer, winner) => {
                let Some(lua) = &self.lua else { return };
                if let Ok(s) = lua.create_string(winner) {
                    self.dispatch(&format!("layer:{layer}"), Value::String(s));
                }
            }
            // The scene was reloaded: whatever it has that is new can now be named; whatever
            // was already known, is still known.
            Event::NewScene(facts, texts, permissions, models, types, plugins, signals, services) => {
                // A plugin receives the new scene with its own definition: that is where it gets its permissions.
                if let (Some(_), Some(def)) = (&self.prefix, plugins.first()) {
                    let mut c = self.c.lock().unwrap();
                    c.unapproved = !crate::permissions::is_approved(def);
                    self.definition = Some(def.clone());
                }
                // The scene: the plugins that stay, with what is new; the ones that arrive, start; the ones no longer there, go.
                if self.prefix.is_none() {
                    let mut staying: Vec<LivePlugin> = Vec::new();
                    for (k, p) in plugins.iter().enumerate() {
                        let fresh = Event::NewScene(facts.clone(), texts.clone(), crate::permissions::effective(p), models.clone(), types.clone(), vec![p.clone()], signals.clone(), services.clone());
                        let live = match self.plugins.iter().position(|x| x.definition.name == p.name && x.definition.logic == p.logic) {
                            Some(i) => {
                                let mut v = self.plugins.remove(i);
                                v.definition = p.clone();
                                v
                            }
                            None => {
                                let v = Self::start(self.for_plugin(p, k + 100));
                                println!("logic  · plugin '{}' arrives", p.name);
                                let _ = v.mailbox.send(fresh.clone());
                                let _ = v.mailbox.send(Event::ReloadLogic);
                                v
                            }
                        };
                        let _ = live.mailbox.send(fresh);
                        staying.push(live);
                    }
                    for gone in std::mem::replace(&mut self.plugins, staying) {
                        // Dropping its mailbox, its thread ends and stops what it left running.
                        println!("logic  · plugin '{}' is gone", gone.definition.name);
                    }
                }
                let mut c = self.c.lock().unwrap();
                c.services = services;
                c.models = models;
                c.types = types.into_iter().collect();
                c.signals = signals.iter().map(|s| (*s).to_owned()).collect();
                // Permissions are indeed replaced: removing one from the scene removes it right away.
                if c.permissions != permissions {
                    println!("logic  · permissions now: {}", describe(&permissions));
                    c.permissions = permissions;
                }
                for (n, v) in facts {
                    c.facts.entry(n.to_owned()).or_insert(v as f64);
                }
                for (n, v) in texts {
                    c.texts.entry(n.to_owned()).or_insert(v);
                }
                // If the scene now asks for a service it did not ask for before, it is set up here.
                drop(c);
                self.subscribe_services();
            }
            Event::Line(id, line) => {
                let f = self.c.lock().unwrap().running.get(&id).map(|x| x.0.clone());
                if let Some(f) = f {
                    self.call_handler(&f, line);
                }
            }
            Event::Data(name, value) => {
                // `service clock as now`: it arrives without anyone asking for it from Luau,
                // and it is spread even if the scene has no logic at all.
                if let Some(alias) = name.strip_prefix("service:") {
                    return self.spread_service(alias, &value);
                }
                let Some(lua) = &self.lua else { return };
                let listeners = self.c.lock().unwrap().watchers.get(&name).cloned().unwrap_or_default();
                match value_to_lua(lua, &value) {
                    Ok(v) => listeners.iter().for_each(|f| self.call_handler(f, v.clone())),
                    Err(e) => eprintln!("logic  · {e}"),
                }
            }
            Event::Process(id, output, code) => {
                self.c.lock().unwrap().running.remove(&id);
                let f = self.c.lock().unwrap().processes.remove(&id);
                if let Some(f) = f {
                    self.call_handler(&f, (output, code));
                }
            }
            _ => {}
        }
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.c.lock().unwrap().timers.iter().map(|t| t.when).min()
    }

    fn tick(&mut self, _: &mut Context) {
        let now = Instant::now();
        let due: Vec<Function> = {
            let mut c = self.c.lock().unwrap();
            let f = c.timers.iter().filter(|t| t.when <= now).map(|t| t.f.clone()).collect();
            for t in &mut c.timers {
                if t.when <= now {
                    if let Some(d) = t.every {
                        t.when = now + d;
                    }
                }
            }
            c.timers.retain(|t| t.when > now);
            f
        };
        for f in due {
            self.call_handler(&f, ());
        }
    }
}
