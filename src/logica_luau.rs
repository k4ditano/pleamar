//! La lógica de una escena, en Luau. Vive en su hilo, en una caja de arena: sin
//! ficheros ni sistema, con tope de memoria y con los segundos contados. Si se
//! atasca no se entera nadie —el render no la espera—, y si se queda en un
//! bucle para siempre, se la corta.
//!
//! Lo único que puede hacer es lo que cruza la frontera: decir qué es verdad,
//! qué pone un texto, que algo ha pasado; y oír lo que pasa en la escena.
//!
//! ```lua
//! fact.open = true                         -- un hecho
//! text["notice.title"] = "Reunión"        -- un texto vivo
//! emit("confirmed")   play("joy")          -- un suceso, un gesto
//! on("view_event", function(n) … end)      -- un suceso de la escena, con su carga
//! on("press:view", …)  on("enter:orb", …)  on("layer:card", function(claim) … end)
//! on("fact:open", function(v) … end)       -- una regla cambió un hecho
//! local t = every(1000, function() … end)  after(500, …)  cancel(t)
//! run("date", {"+%H:%M"}, function(out, code) … end)   -- una orden del sistema
//! ```

use crate::escena::{internar, ARender, Escena, Evento};
use crate::logica::{Contexto, Guion};
use crate::plataforma::Valor;
use mlua::{Function, Lua, MultiValue, Table, Value, VmState};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Todo lo que la lógica ha dejado corriendo. Al salir el programa hay que
/// pararlo: un hijo no muere con su padre, y un `pactl subscribe` huérfano se
/// quedaría ahí para siempre.
static HIJOS: Mutex<Vec<Arc<Mutex<Option<std::process::Child>>>>> = Mutex::new(Vec::new());

pub fn parar_hijos() {
    for h in HIJOS.lock().unwrap().drain(..) {
        if let Some(mut h) = h.lock().unwrap().take() {
            let _ = h.kill();
            let _ = h.wait();
        }
    }
}

/// Cuánto puede tardar un manejador antes de que se le corte.
const PACIENCIA: Duration = Duration::from_secs(2);
const MEMORIA: usize = 64 << 20;

struct Temporizador {
    id: u32,
    cuando: Instant,
    cada: Option<Duration>,
    f: Function,
}

#[derive(Default)]
struct Compartido {
    manejadores: HashMap<String, Vec<Function>>,
    temporizadores: Vec<Temporizador>,
    procesos: HashMap<u32, Function>,
    /// Las órdenes que siguen en marcha: a quién se le cuenta cada línea, y cómo pararlas.
    en_marcha: HashMap<u32, (Function, Arc<Mutex<Option<std::process::Child>>>)>,
    vigias: HashMap<String, Vec<Function>>,
    siguiente: u32,
    hechos: HashMap<String, f64>,
    textos: HashMap<String, String>,
    permisos: crate::escena::Permisos,
    modelos: Vec<crate::escena::Modelo>,
    /// De los hechos que no son números a secas, qué son: la lógica los ve como `true` o como `"critical"`.
    tipos: HashMap<String, crate::escena::TipoDeHecho>,
    sucesos: std::collections::HashSet<String>,
    /// Si es la lógica de un plugin, cómo se llama: para que los errores hablen de él y no de la escena.
    plugin: Option<String>,
    /// Pide permisos que nadie le ha aprobado: corre sin ninguno, y los errores dicen por qué.
    sin_aprobar: bool,
    /// Hasta cuándo puede correr lo que está corriendo.
    limite: Option<Instant>,
}

/// Reparte una lista de fichas entre los textos y los hechos que la escena tiene para
/// ella —`rows.3.label`—, y las listas de dentro, igual. Solo viaja lo que cambia.
///
/// Primero se lee entera y luego se aplica: si una ficha está mal, la lista que
/// había se queda como estaba, no a medias.
fn repartir(c: &mut Compartido, tx: &Sender<ARender>, prefijo: &str, m: &crate::escena::Modelo, lista: &Table) -> mlua::Result<()> {
    let mut cambios = Vec::new();
    leer_fichas(&mut cambios, prefijo, m, lista)?;
    for cambio in cambios {
        match cambio {
            Cambio::Texto(nombre, t) => if c.textos.get(&nombre) != Some(&t) {
                c.textos.insert(nombre.clone(), t.clone());
                let _ = tx.send(ARender::Texto(internar(&nombre), t));
            },
            Cambio::Hecho(nombre, n) => if c.hechos.get(&nombre) != Some(&n) {
                c.hechos.insert(nombre.clone(), n);
                let _ = tx.send(ARender::Hecho(internar(&nombre), n as f32));
            },
        }
    }
    Ok(())
}

enum Cambio {
    Texto(String, String),
    Hecho(String, f64),
}

fn leer_fichas(cambios: &mut Vec<Cambio>, prefijo: &str, m: &crate::escena::Modelo, lista: &Table) -> mlua::Result<()> {
    use crate::escena::{TipoDeCampo, TipoDeHecho, ValorDeCampo};
    let total = lista.raw_len();
    let texto = |c: &mut Vec<Cambio>, nombre: String, t: String| c.push(Cambio::Texto(nombre, t));
    let hecho = |c: &mut Vec<Cambio>, nombre: String, n: f64| c.push(Cambio::Hecho(nombre, n));
    let c = cambios;
    for i in 0..total.min(m.caben) {
        let ficha: Value = lista.raw_get(i + 1)?;
        let Value::Table(ficha) = ficha else {
            return Err(mlua::Error::runtime(format!("'{prefijo}' is a list of records, and number {} is not a table", i + 1)));
        };
        for campo in &m.campos {
            let nombre = format!("{prefijo}.{i}.{}", campo.nombre);
            let v: Value = ficha.get(campo.nombre.as_str())?;
            match (&campo.tipo, &campo.por_defecto) {
                (TipoDeCampo::Lista(dentro), _) => match v {
                    Value::Table(t) => leer_fichas(c, &nombre, dentro, &t)?,
                    // Sin lista, es una vacía: que no quede la de la ficha que hubo antes en este sitio.
                    _ => {
                        hecho(c, format!("{nombre}.count"), 0.0);
                        hecho(c, format!("{nombre}.total"), 0.0);
                    }
                },
                (TipoDeCampo::Texto | TipoDeCampo::Imagen(..), por_defecto) => {
                    let t = match &v {
                        Value::Nil => if let ValorDeCampo::Texto(t) = por_defecto { t.clone() } else { String::new() },
                        otro => otro.to_string()?,
                    };
                    texto(c, nombre, t);
                }
                (tipo, por_defecto) => {
                    let falta = if let ValorDeCampo::Numero(d) = por_defecto { *d as f64 } else { 0.0 };
                    let n = match (tipo, &v) {
                        (_, Value::Nil) => falta,
                        // Una lista donde se espera un número cuenta como cuántos tiene.
                        (TipoDeCampo::Numero, Value::Table(t)) => t.raw_len() as f64,
                        (TipoDeCampo::Bool, Value::Table(t)) => (t.raw_len() > 0) as u8 as f64,
                        (TipoDeCampo::Bool, _) => a_numero(&nombre, Some(&TipoDeHecho::Bool), &v)?,
                        (TipoDeCampo::Enum(nombres), _) => a_numero(&nombre, Some(&TipoDeHecho::Enum(nombres.clone())), &v)?,
                        _ => a_numero(&nombre, None, &v)?,
                    };
                    hecho(c, nombre, n);
                }
            }
        }
    }
    hecho(c, format!("{prefijo}.count"), total.min(m.caben) as f64);
    hecho(c, format!("{prefijo}.total"), total as f64);
    Ok(())
}

/// Lo que la lógica escribe en un hecho, como el número que ve la escena. Un `bool`
/// quiere `true` o `false`; un enumerado, uno de sus nombres; los demás, un número.
fn a_numero(nombre: &str, tipo: Option<&crate::escena::TipoDeHecho>, v: &Value) -> mlua::Result<f64> {
    use crate::escena::TipoDeHecho;
    let numero = match v {
        Value::Boolean(b) => Some(*b as u8 as f64),
        Value::Integer(i) => Some(*i as f64),
        Value::Number(x) => Some(*x),
        _ => None,
    };
    match (tipo, v) {
        (Some(TipoDeHecho::Enum(nombres)), Value::String(t)) => {
            let t = t.to_string_lossy();
            match nombres.iter().position(|n| *n == t) {
                Some(k) => Ok(k as f64),
                None => Err(mlua::Error::runtime(format!("'{nombre}' cannot be '{t}'{}: it can be {}", pista(&t, nombres.iter()), nombres.join(", ")))),
            }
        }
        (Some(TipoDeHecho::Enum(nombres)), _) => match numero {
            Some(n) if n >= 0.0 && (n as usize) < nombres.len() && n.fract() == 0.0 => Ok(n),
            _ => Err(mlua::Error::runtime(format!("'{nombre}' is an enum: it can be {}", nombres.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ")))),
        },
        (Some(TipoDeHecho::Bool), _) => numero.map(|n| (n != 0.0) as u8 as f64).ok_or_else(|| mlua::Error::runtime(format!("'{nombre}' is a yes or no: true or false, not a {}", v.type_name()))),
        (None, _) => numero.ok_or_else(|| mlua::Error::runtime(format!("'{nombre}' is a number, not a {}", v.type_name()))),
    }
}

/// Y al revés: como lo ve la lógica.
fn de_numero(lua: &Lua, tipo: Option<&crate::escena::TipoDeHecho>, v: f64) -> mlua::Result<Value> {
    use crate::escena::TipoDeHecho;
    Ok(match tipo {
        Some(TipoDeHecho::Bool) => Value::Boolean(v > 0.5),
        Some(TipoDeHecho::Enum(nombres)) => match nombres.get(v.round().max(0.0) as usize) {
            Some(n) => Value::String(lua.create_string(n)?),
            None => Value::Number(v),
        },
        None => Value::Number(v),
    })
}

fn en_claro(p: &crate::escena::Permisos) -> String {
    crate::permisos::en_claro(p)
}

/// Por qué no, dicho a quien toca: la escena lo declara; un plugin lo declara y además se lo aprueban.
fn denegado(c: &Mutex<Compartido>, para_que: &str, como: &str) -> mlua::Error {
    let c = c.lock().unwrap();
    mlua::Error::runtime(match &c.plugin {
        None => format!("the scene gives no permission to {para_que}. If it should be able to, declare it in the .plm: permissions {{ {como} }}"),
        Some(p) if c.sin_aprobar => format!("plugin '{p}' wants to {para_que}, but nobody has approved its permissions: it runs with none. To see them and decide: pleamar --aprobar SCENE"),
        Some(p) => format!("plugin '{p}' has no permission to {para_que}. If it should be able to, declare it in its own .plm (the scene's do not count): permissions {{ {como} }}"),
    })
}

/// ¿Puede esta lógica lanzar esa orden? Sin declarar, no; y el error dice qué escribir.
fn permiso_de_orden(c: &Mutex<Compartido>, orden: &str) -> mlua::Result<()> {
    if c.lock().unwrap().permisos.ordenes.iter().any(|o| o == orden) {
        return Ok(());
    }
    Err(denegado(c, &format!("run '{orden}'"), &format!("run: \"{orden}\"")))
}

/// Lo mismo para un servicio. **Escuchar no es mandar**: `services: "audio"` deja saber el
/// volumen (`sys.watch`, `sys.ask`); para cambiarlo hace falta `"audio.volume"`, o `"audio.*"`.
fn permiso_de_servicio(c: &Mutex<Compartido>, nombre: &str, manda: bool) -> mlua::Result<()> {
    let servicio = nombre.split('.').next().unwrap_or(nombre);
    let tiene = |que: &str| c.lock().unwrap().permisos.servicios.iter().any(|s| s == que);
    if manda {
        if tiene(nombre) || tiene(&format!("{servicio}.*")) {
            return Ok(());
        }
        return Err(denegado(c, &format!("ask the system for '{nombre}'"), &format!("services: \"{nombre}\"  (or \"{servicio}.*\" for everything of {servicio})")));
    }
    if tiene(servicio) || tiene(&format!("{servicio}.*")) {
        return Ok(());
    }
    Err(denegado(c, &format!("use the '{servicio}' service"), &format!("services: \"{servicio}\"")))
}

/// Un dato del sistema, como lo ve Luau: tablas, números, textos.
fn a_lua(lua: &Lua, v: &Valor) -> mlua::Result<Value> {
    Ok(match v {
        Valor::Nulo => Value::Nil,
        Valor::Si(b) => Value::Boolean(*b),
        Valor::Num(n) => Value::Number(*n),
        Valor::Texto(s) => Value::String(lua.create_string(s)?),
        Valor::Lista(l) => {
            let t = lua.create_table()?;
            for (k, x) in l.iter().enumerate() {
                t.set(k + 1, a_lua(lua, x)?)?;
            }
            Value::Table(t)
        }
        Valor::Mapa(m) => {
            let t = lua.create_table()?;
            for (k, x) in m {
                t.set(k.as_str(), a_lua(lua, x)?)?;
            }
            Value::Table(t)
        }
    })
}

fn pista<'a>(k: &str, conocidos: impl Iterator<Item = &'a String>) -> String {
    crate::lenguaje::parecido(k, conocidos).map_or(String::new(), |p| format!(". Did you mean '{p}'?"))
}

pub struct GuionLuau {
    escena: String,
    logica: String,
    tx: Sender<ARender>,
    a_logica: Sender<Evento>,
    bloqueada: Arc<AtomicBool>,
    lua: Option<Lua>,
    c: Arc<Mutex<Compartido>>,
    /// Si es la lógica de un plugin: su nombre. Todo lo que nombre va bajo él (`Clock.now`),
    /// así que no puede tocar —ni oír— nada que no sea suyo.
    prefijo: Option<String>,
    /// La lógica de la escena lleva consigo la de sus plugins: cada una, su propio estado
    /// de Luau **y su propio hilo**. Uno que se atasque no frena a los demás, ni a la escena.
    plugins: Vec<PluginVivo>,
    /// Si es un plugin: lo que dice de él la escena (su lógica, lo que pide).
    definicion: Option<crate::escena::Plugin>,
}

/// Un plugin en marcha, visto desde la lógica de la escena: por dónde se le habla.
/// Soltar el buzón lo para: su hilo acaba, y con él lo que dejó corriendo.
struct PluginVivo {
    definicion: crate::escena::Plugin,
    buzon: Sender<Evento>,
}

/// El nombre de verdad de lo que una lógica nombra: en un plugin, bajo su nombre.
fn afuera(prefijo: &Option<String>, k: &str) -> String {
    match prefijo {
        Some(p) => format!("{p}.{k}"),
        None => k.to_owned(),
    }
}

/// «No existe», dicho a quien toca: a un plugin se le habla de lo suyo, con el nombre que él usa.
fn no_existe<'a>(prefijo: &Option<String>, que: &str, entero: &str, conocidos: impl Iterator<Item = &'a String>) -> mlua::Error {
    match prefijo {
        None => mlua::Error::runtime(format!("the scene has {que} called '{entero}'{}", pista(entero, conocidos))),
        Some(p) => {
            let corto = entero.strip_prefix(&format!("{p}.")).unwrap_or(entero);
            let suyos: Vec<String> = conocidos.filter_map(|k| k.strip_prefix(&format!("{p}.")).map(str::to_owned)).collect();
            mlua::Error::runtime(format!("plugin '{p}' has {que} called '{corto}'{}. A plugin only sees what its library declares", pista(corto, suyos.iter())))
        }
    }
}

/// Lo mismo para lo que se escucha: `fact:ticking` es `fact:Clock.ticking`, y `tapped`, `Clock.tapped`.
/// Así un plugin no oye el teclado, ni el ratón, ni los sucesos de la escena: solo lo suyo.
fn afuera_de_escucha(prefijo: &Option<String>, que: &str) -> String {
    match (prefijo, que.split_once(':')) {
        (None, _) => que.to_owned(),
        (Some(p), Some((clase, nombre))) => format!("{clase}:{p}.{nombre}"),
        (Some(p), None) => format!("{p}.{que}"),
    }
}

impl GuionLuau {
    pub fn nuevo(escena: &str, logica: &str, tx: Sender<ARender>, a_logica: Sender<Evento>, bloqueada: Arc<AtomicBool>) -> Self {
        GuionLuau { escena: escena.to_owned(), logica: logica.to_owned(), tx, a_logica, bloqueada, lua: None, c: Arc::default(), prefijo: None, plugins: Vec::new(), definicion: None }
    }

    /// La lógica de un plugin. El número es para que sus temporizadores y procesos no se
    /// llamen igual que los de otra lógica: todas comparten el buzón.
    fn de_plugin(&self, p: &crate::escena::Plugin, numero: usize) -> Self {
        let c = Compartido { siguiente: (numero as u32 + 1) * 1_000_000, plugin: Some(p.nombre.clone()), ..Default::default() };
        GuionLuau { escena: self.escena.clone(), logica: p.logica.to_string_lossy().into_owned(), tx: self.tx.clone(), a_logica: self.a_logica.clone(), bloqueada: self.bloqueada.clone(), lua: None, c: Arc::new(Mutex::new(c)), prefijo: Some(p.nombre.clone()), plugins: Vec::new(), definicion: Some(p.clone()) }
    }

    /// Pone a un plugin a correr en su hilo. Desde ahí atiende su buzón y sus temporizadores.
    fn arrancar(mut plugin: GuionLuau) -> PluginVivo {
        let (buzon, cartas) = std::sync::mpsc::channel::<Evento>();
        let definicion = plugin.definicion.clone().expect("only a plugin's logic is started");
        let nombre = format!("plugin {}", definicion.nombre);
        let _ = std::thread::Builder::new().name(nombre).spawn(move || {
            let mut ctx = Contexto::de_plugin(plugin.tx.clone(), plugin.bloqueada.clone());
            loop {
                let espera = plugin.proxima().map_or(Duration::from_secs(3600), |p| p.saturating_duration_since(Instant::now()));
                match cartas.recv_timeout(espera) {
                    Ok(e) => plugin.evento(e, &mut ctx),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
                if plugin.proxima().is_some_and(|p| p <= Instant::now()) {
                    plugin.tic(&mut ctx);
                }
            }
            plugin.soltar();
        });
        PluginVivo { definicion, buzon }
    }

    /// Para lo que esta lógica dejó corriendo: temporizadores, manejadores, procesos.
    fn soltar(&self) {
        let mut c = self.c.lock().unwrap();
        c.manejadores.clear();
        c.temporizadores.clear();
        c.procesos.clear();
        c.vigias.clear();
        for (_, (_, hijo)) in c.en_marcha.drain() {
            if let Some(mut h) = hijo.lock().unwrap().take() {
                let _ = h.kill();
            }
        }
    }

    /// Lo que la escena tiene, visto desde esta lógica. Un plugin lo ve todo por dentro
    /// —se comprueba contra ello—, pero solo puede nombrar lo que empieza por su nombre.
    fn conocer(&self, e: &Escena, permisos: crate::escena::Permisos) {
        let mut c = self.c.lock().unwrap();
        c.hechos = e.hechos.iter().map(|(n, v)| (n.to_string(), *v as f64)).collect();
        c.textos = e.textos.iter().map(|(n, v)| (n.to_string(), v.clone())).collect();
        c.permisos = permisos;
        c.modelos = e.modelos.clone();
        c.tipos = e.tipos.iter().cloned().collect();
        c.sucesos = e.sucesos.iter().map(|s| s.0.to_owned()).collect();
    }

    /// Un estado de Luau nuevo, con la frontera puesta, y el fichero ejecutado.
    fn cargar(&mut self) {
        // Lo que la lógica vieja dejó corriendo se para con ella.
        self.soltar();
        // Se dice antes de cargar: si el script tropieza con un permiso que no tiene, que se sepa por qué.
        if let (Some(p), true) = (&self.prefijo, self.c.lock().unwrap().sin_aprobar) {
            println!("logic  · plugin '{p}' · ⚠ NOT APPROVED: runs unable to touch the system. To see what it asks for and decide: pleamar --aprobar {}", self.escena);
        }
        // Una escena puede no tener lógica propia y sí plugins que la tengan.
        if self.prefijo.is_none() && !std::path::Path::new(&self.logica).is_file() {
            return;
        }
        let fuente = match std::fs::read_to_string(&self.logica) {
            Ok(f) => f,
            Err(e) => return eprintln!("logic  · {}: {e}", self.logica),
        };
        let t0 = Instant::now();
        match self.preparar().and_then(|lua| {
            self.c.lock().unwrap().limite = Some(Instant::now() + PACIENCIA);
            lua.load(&fuente).set_name(format!("@{}", self.logica)).exec()?;
            Ok(lua)
        }) {
            Ok(lua) => {
                self.lua = Some(lua);
                let c = self.c.lock().unwrap();
                println!("logic  · {} running in {:.1} ms · {} handlers, {} timers", self.logica, t0.elapsed().as_secs_f32() * 1000.0, c.manejadores.values().map(Vec::len).sum::<usize>(), c.temporizadores.len());
                match &self.prefijo {
                    Some(p) => println!("logic  · plugin '{p}' · permissions · {}", en_claro(&c.permisos)),
                    None => println!("logic  · permissions · {}", en_claro(&c.permisos)),
                }
            }
            Err(e) => eprintln!("logic  · the previous one stays as it was:\n{e}"),
        }
        self.c.lock().unwrap().limite = None;
    }

    fn preparar(&self) -> mlua::Result<Lua> {
        let lua = Lua::new();
        lua.set_memory_limit(MEMORIA)?;
        // En la caja de arena, Luau da por hecho que los globales no cambian y lee
        // `fact.open` UNA vez, al cargar el script. Estos sí cambian: hay que decírselo.
        lua.set_compiler(mlua::chunk::Compiler::new().set_mutable_globals(["fact", "text", "model", "sys"]));
        let g = lua.globals();

        // Un manejador que no acaba no puede quedarse con el hilo para siempre.
        let c = self.c.clone();
        lua.set_interrupt(move |_| match c.lock().unwrap().limite {
            Some(l) if Instant::now() > l => Err(mlua::Error::runtime(format!("a handler has been running for more than {} s: cut off", PACIENCIA.as_secs()))),
            _ => Ok(VmState::Continue),
        });

        // fact.open = true · fact.open
        let (tx, c) = (self.tx.clone(), self.c.clone());
        let pre = self.prefijo.clone();
        let poner = lua.create_function(move |_, (_, k, v): (Table, String, Value)| {
            let k = afuera(&pre, &k);
            // Un nombre mal escrito es un error aquí, con su línea, y no un aviso perdido en el render.
            if !c.lock().unwrap().hechos.contains_key(&k) {
                return Err(no_existe(&pre, "no fact", &k, c.lock().unwrap().hechos.keys()));
            }
            let tipo = c.lock().unwrap().tipos.get(&k).cloned();
            let n = a_numero(&k, tipo.as_ref(), &v)?;
            c.lock().unwrap().hechos.insert(k.clone(), n);
            let _ = tx.send(ARender::Hecho(internar(&k), n as f32));
            Ok(())
        })?;
        let c = self.c.clone();
        let pre = self.prefijo.clone();
        let leer = lua.create_function(move |lua, (_, k): (Table, String)| {
            let k = afuera(&pre, &k);
            let c = c.lock().unwrap();
            match c.hechos.get(&k) {
                Some(v) => de_numero(lua, c.tipos.get(&k), *v),
                None => Ok(Value::Nil),
            }
        })?;
        g.set("fact", Self::tabla_viva(&lua, leer, poner)?)?;

        // text["notice.title"] = "…"
        let (tx, c) = (self.tx.clone(), self.c.clone());
        let pre = self.prefijo.clone();
        let poner = lua.create_function(move |_, (_, k, v): (Table, String, String)| {
            let k = afuera(&pre, &k);
            if !c.lock().unwrap().textos.contains_key(&k) {
                return Err(no_existe(&pre, "no text", &k, c.lock().unwrap().textos.keys()));
            }
            c.lock().unwrap().textos.insert(k.clone(), v.clone());
            let _ = tx.send(ARender::Texto(internar(&k), v));
            Ok(())
        })?;
        let c = self.c.clone();
        let pre = self.prefijo.clone();
        let leer = lua.create_function(move |_, (_, k): (Table, String)| Ok(c.lock().unwrap().textos.get(&afuera(&pre, &k)).cloned()))?;
        g.set("text", Self::tabla_viva(&lua, leer, poner)?)?;

        // model.rows = { { label = "Abrir", enabled = true }, … }: una lista entera, de una vez.
        // Cada campo de cada ficha es por dentro un texto o un hecho; aquí se reparten,
        // y solo viaja lo que haya cambiado.
        let guardadas = lua.create_table()?;
        let (tx, c, almacen) = (self.tx.clone(), self.c.clone(), guardadas.clone());
        let pre = self.prefijo.clone();
        let poner = lua.create_function(move |_, (_, k, lista): (Table, String, Value)| {
            let k = afuera(&pre, &k);
            let Value::Table(lista) = lista else {
                return Err(mlua::Error::runtime(format!("'model.{k}' takes a list of records: model.{k} = {{ {{ … }}, {{ … }} }}")));
            };
            let mut c = c.lock().unwrap();
            let Some(m) = c.modelos.iter().find(|m| m.nombre == k).cloned() else {
                return Err(no_existe(&pre, "no model", &k, c.modelos.iter().map(|m| &m.nombre)));
            };
            repartir(&mut c, &tx, &k, &m, &lista)?;
            almacen.raw_set(k, lista)
        })?;
        let pre = self.prefijo.clone();
        let leer = lua.create_function(move |_, (_, k): (Table, String)| guardadas.raw_get::<Value>(afuera(&pre, &k)))?;
        g.set("model", Self::tabla_viva(&lua, leer, poner)?)?;

        let tx = self.tx.clone();
        let (pre, c) = (self.prefijo.clone(), self.c.clone());
        g.set("emit", lua.create_function(move |_, n: String| {
            let n = afuera(&pre, &n);
            // Un suceso que no existe era un aviso perdido en el render: aquí es un error, con su línea.
            if !c.lock().unwrap().sucesos.contains(&n) {
                let conocidos: Vec<String> = c.lock().unwrap().sucesos.iter().cloned().collect();
                return Err(no_existe(&pre, "no event", &n, conocidos.iter()));
            }
            Ok(tx.send(ARender::Suceso(internar(&n))).is_ok())
        })?)?;
        // focus("query") pone el cursor de texto en un campo; focus() lo quita.
        let tx = self.tx.clone();
        let pre = self.prefijo.clone();
        g.set("focus", lua.create_function(move |_, n: Option<String>| {
            if pre.is_some() {
                return Err(mlua::Error::runtime("a plugin does not move the text cursor: that belongs to the scene"));
            }
            Ok(tx.send(ARender::Enfocar(n.map(|n| internar(&n)))).is_ok())
        })?)?;
        let tx = self.tx.clone();
        let pre = self.prefijo.clone();
        g.set("play", lua.create_function(move |_, n: String| {
            if pre.is_some() {
                return Err(mlua::Error::runtime("a plugin does not ask for gestures: gestures belong to the scene. Let it emit an event of its own, and the scene will decide"));
            }
            Ok(tx.send(ARender::Gesto(internar(&n))).is_ok())
        })?)?;

        let c = self.c.clone();
        let pre = self.prefijo.clone();
        g.set("on", lua.create_function(move |_, (que, f): (String, Function)| {
            let que = afuera_de_escucha(&pre, &que);
            c.lock().unwrap().manejadores.entry(que).or_default().push(f);
            Ok(())
        })?)?;

        let plazo = |c: &Arc<Mutex<Compartido>>, ms: f64, f: Function, repite: bool| {
            let mut c = c.lock().unwrap();
            c.siguiente += 1;
            let d = Duration::from_secs_f64(ms.max(1.0) / 1000.0);
            let id = c.siguiente;
            c.temporizadores.push(Temporizador { id, cuando: Instant::now() + d, cada: repite.then_some(d), f });
            id
        };
        let c = self.c.clone();
        g.set("after", lua.create_function(move |_, (ms, f): (f64, Function)| Ok(plazo(&c, ms, f, false)))?)?;
        let c = self.c.clone();
        g.set("every", lua.create_function(move |_, (ms, f): (f64, Function)| Ok(plazo(&c, ms, f, true)))?)?;
        let c = self.c.clone();
        g.set("cancel", lua.create_function(move |_, id: u32| {
            c.lock().unwrap().temporizadores.retain(|t| t.id != id);
            Ok(())
        })?)?;

        // Trabajo de mentira, para ver que al render le da igual.
        let (c, bloqueada) = (self.c.clone(), self.bloqueada.clone());
        g.set("busy", lua.create_function(move |_, ms: f64| {
            let d = Duration::from_secs_f64(ms.max(0.0) / 1000.0);
            if let Some(l) = &mut c.lock().unwrap().limite {
                *l += d;
            }
            bloqueada.store(true, Ordering::Relaxed);
            let fin = Instant::now() + d;
            while Instant::now() < fin {
                std::hint::spin_loop();
            }
            bloqueada.store(false, Ordering::Relaxed);
            Ok(())
        })?)?;

        // Una orden del sistema: corre en otro hilo y contesta cuando acaba.
        let (c, a_logica) = (self.c.clone(), self.a_logica.clone());
        g.set("run", lua.create_function(move |_, (orden, args, f): (String, Option<Vec<String>>, Option<Function>)| {
            permiso_de_orden(&c, &orden)?;
            let id = {
                let mut c = c.lock().unwrap();
                c.siguiente += 1;
                let id = c.siguiente;
                if let Some(f) = f {
                    c.procesos.insert(id, f);
                }
                id
            };
            let a_logica = a_logica.clone();
            std::thread::spawn(move || {
                let (salida, codigo) = match std::process::Command::new(&orden).args(args.unwrap_or_default()).output() {
                    Ok(o) => (String::from_utf8_lossy(&o.stdout).trim_end().to_owned(), o.status.code().unwrap_or(-1)),
                    Err(e) => (e.to_string(), -1),
                };
                let _ = a_logica.send(Evento::Proceso(id, salida, codigo));
            });
            Ok(())
        })?)?;

        // Una orden que no acaba —`pactl subscribe`, `playerctl --follow`—: una
        // llamada por cada línea que escriba, y `kill(id)` para pararla.
        let (c, a_logica) = (self.c.clone(), self.a_logica.clone());
        g.set("spawn", lua.create_function(move |_, (orden, args, f): (String, Option<Vec<String>>, Function)| {
            use std::io::BufRead;
            permiso_de_orden(&c, &orden)?;
            let mut lanzar = std::process::Command::new(&orden);
            lanzar.args(args.unwrap_or_default()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
            // Si al programa lo matan, esto se va con él.
            crate::plataforma::morir_con_el_padre(&mut lanzar);
            let mut hijo = lanzar
                .spawn()
                .map_err(|e| mlua::Error::runtime(format!("cannot run '{orden}': {e}")))?;
            let salida = hijo.stdout.take();
            let hijo = Arc::new(Mutex::new(Some(hijo)));
            HIJOS.lock().unwrap().push(hijo.clone());
            let id = {
                let mut c = c.lock().unwrap();
                c.siguiente += 1;
                let id = c.siguiente;
                c.en_marcha.insert(id, (f, hijo.clone()));
                id
            };
            let a_logica = a_logica.clone();
            std::thread::spawn(move || {
                if let Some(s) = salida {
                    for linea in std::io::BufReader::new(s).lines().map_while(Result::ok) {
                        if a_logica.send(Evento::Linea(id, linea)).is_err() {
                            break;
                        }
                    }
                }
                let codigo = hijo.lock().unwrap().take().and_then(|mut h| h.wait().ok()).and_then(|s| s.code()).unwrap_or(-1);
                let _ = a_logica.send(Evento::Proceso(id, String::new(), codigo));
            });
            Ok(id)
        })?)?;
        let c = self.c.clone();
        g.set("kill", lua.create_function(move |_, id: u32| {
            if let Some((_, hijo)) = c.lock().unwrap().en_marcha.remove(&id) {
                if let Some(mut h) = hijo.lock().unwrap().take() {
                    let _ = h.kill();
                }
            }
            Ok(())
        })?)?;

        // Lo que pasa en el sistema, por un nombre que es el mismo en todas partes.
        let sys = lua.create_table()?;
        let (c, a_logica) = (self.c.clone(), self.a_logica.clone());
        sys.set("watch", lua.create_function(move |_, (nombre, f): (String, Function)| {
            permiso_de_servicio(&c, &nombre, false)?;
            let primero = {
                let mut c = c.lock().unwrap();
                let v = c.vigias.entry(nombre.clone()).or_default();
                v.push(f);
                v.len() == 1
            };
            if !primero {
                return Ok(true);
            }
            let (a_logica, n) = (Mutex::new(a_logica.clone()), nombre.clone());
            Ok(crate::plataforma::servicio(&nombre, Box::new(move |v| {
                let _ = a_logica.lock().unwrap().send(Evento::Dato(n.clone(), v));
            })))
        })?)?;
        let c = self.c.clone();
        sys.set("call", lua.create_function(move |_, (nombre, args): (String, mlua::Variadic<Value>)| {
            permiso_de_servicio(&c, &nombre, true)?;
            let args: Vec<Valor> = args.iter().map(|v| match v {
                Value::Boolean(b) => Valor::Si(*b),
                Value::Integer(i) => Valor::Num(*i as f64),
                Value::Number(n) => Valor::Num(*n),
                Value::String(s) => Valor::Texto(s.to_string_lossy()),
                _ => Valor::Nulo,
            }).collect();
            crate::plataforma::orden(&nombre, &args).map_err(mlua::Error::runtime)
        })?)?;
        // Lo mismo, pero contesta: `sys.ask("tray.menu", key)` devuelve el menú.
        let c = self.c.clone();
        sys.set("ask", lua.create_function(move |lua, (nombre, args): (String, mlua::Variadic<Value>)| {
            permiso_de_servicio(&c, &nombre, false)?;
            let args: Vec<Valor> = args.iter().map(|v| match v {
                Value::Boolean(b) => Valor::Si(*b),
                Value::Integer(i) => Valor::Num(*i as f64),
                Value::Number(n) => Valor::Num(*n),
                Value::String(s) => Valor::Texto(s.to_string_lossy()),
                _ => Valor::Nulo,
            }).collect();
            a_lua(lua, &crate::plataforma::consulta(&nombre, &args).map_err(mlua::Error::runtime)?)
        })?)?;
        g.set("sys", sys)?;

        // require("util"): otro fichero de la misma carpeta que esta lógica, y de ninguna otra.
        // Se carga una vez; lo que devuelva es el módulo.
        let carpeta = std::path::Path::new(&self.logica).parent().map(std::path::Path::to_owned).unwrap_or_default();
        let cargados = lua.create_table()?;
        g.set("require", lua.create_function(move |lua, nombre: String| {
            let limpio = !nombre.is_empty() && nombre.split('/').all(|t| !t.is_empty() && t != ".." && t != "." && t.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-'));
            if !limpio {
                return Err(mlua::Error::runtime(format!("require(\"{nombre}\"): a module is a file in this logic's folder, or in one inside it. No `..`, no full paths and no extension")));
            }
            if let Ok(Value::Table(t)) = cargados.raw_get::<Value>(nombre.as_str()) {
                return t.raw_get::<Value>("valor");
            }
            let ruta = carpeta.join(format!("{nombre}.luau"));
            let fuente = std::fs::read_to_string(&ruta).map_err(|e| mlua::Error::runtime(format!("require(\"{nombre}\"): {}: {e}", ruta.display())))?;
            let valor: Value = lua.load(&fuente).set_name(format!("@{}", ruta.display())).eval()?;
            let caja = lua.create_table()?;
            caja.raw_set("valor", valor.clone())?;
            cargados.raw_set(nombre, caja)?;
            Ok(valor)
        })?)?;
        let quien = self.prefijo.as_ref().map_or(String::new(), |p| format!("[{p}] "));
        g.set("log", lua.create_function(move |_, v: MultiValue| {
            let trozos: Vec<String> = v.iter().map(|x| x.to_string().unwrap_or_else(|_| format!("{x:?}"))).collect();
            println!("luau   · {quien}{}", trozos.join(" "));
            Ok(())
        })?)?;

        // La caja de arena, lo último: a partir de aquí los globales no se tocan.
        lua.sandbox(true)?;
        Ok(lua)
    }

    /// Una tabla que no guarda nada: leerla y escribirla son llamadas.
    fn tabla_viva(lua: &Lua, leer: Function, poner: Function) -> mlua::Result<Table> {
        let (t, meta) = (lua.create_table()?, lua.create_table()?);
        meta.set("__index", leer)?;
        meta.set("__newindex", poner)?;
        t.set_metatable(Some(meta))?;
        Ok(t)
    }

    fn llamar(&self, f: &Function, args: impl mlua::IntoLuaMulti) {
        self.c.lock().unwrap().limite = Some(Instant::now() + PACIENCIA);
        if let Err(e) = f.call::<()>(args) {
            // Que se sepa de quién es el fallo: con plugins, «la lógica» son varias.
            match &self.prefijo {
                Some(p) => eprintln!("logic  · plugin '{p}' · {e}"),
                None => eprintln!("logic  · {e}"),
            }
        }
        self.c.lock().unwrap().limite = None;
    }

    fn avisar_con(&self, que: &str, texto: String) {
        let quienes: Vec<Function> = self.c.lock().unwrap().manejadores.get(que).cloned().unwrap_or_default();
        for f in quienes {
            self.llamar(&f, texto.clone());
        }
    }

    fn avisar(&self, que: &str, arg: Value) {
        let quienes: Vec<Function> = self.c.lock().unwrap().manejadores.get(que).cloned().unwrap_or_default();
        for f in quienes {
            self.llamar(&f, arg.clone());
        }
    }
}

impl Guion for GuionLuau {
    fn escena(&mut self) -> Escena {
        let e = super::escenas::de_fichero::leer(&self.escena).unwrap_or_else(|m| {
            eprintln!("{m}");
            std::process::exit(1)
        });
        // Lo que la escena da por cierto al nacer es lo que la lógica cree hasta que alguien diga otra cosa.
        self.conocer(&e, e.permisos.clone());
        self.plugins = e.plugins.iter().enumerate().map(|(k, p)| {
            let plugin = self.de_plugin(p, k);
            plugin.conocer(&e, crate::permisos::los_que_valen(p));
            plugin.c.lock().unwrap().sin_aprobar = !crate::permisos::aprobado(p);
            Self::arrancar(plugin)
        }).collect();
        e
    }

    fn evento(&mut self, e: Evento, ctx: &mut Contexto) {
        // Cada plugin recibe lo mismo, pero solo tiene manejadores con su nombre delante:
        // de lo que no es suyo no se entera.
        if !matches!(e, Evento::EscenaNueva(..)) {
            for p in &self.plugins {
                let _ = p.buzon.send(e.clone());
            }
        }
        let _ = &ctx;
        let numero = |v: f32| Value::Number(v as f64);
        match e {
            // El fichero se ejecuta ya en el hilo de la lógica, con la escena entregada.
            Evento::Alarma("inicio") | Evento::RecargarLogica => self.cargar(),
            Evento::Suceso(n, carga) => self.avisar(n, carga.map_or(Value::Nil, numero)),
            Evento::Entra(z) => self.avisar(&format!("enter:{z}"), Value::Nil),
            Evento::Sale(z) => self.avisar(&format!("leave:{z}"), Value::Nil),
            Evento::Pulsa(z) => self.avisar(&format!("press:{z}"), Value::Nil),
            Evento::Suelta(z) => self.avisar(&format!("release:{z}"), Value::Nil),
            Evento::Rueda(z, d) => self.avisar(&format!("scroll:{z}"), numero(d)),
            // Lo que se escribe en un campo: la lógica lo sabe tecla a tecla, y no ha
            // tenido que hacer nada para que se vea.
            Evento::Texto(n, valor) => {
                self.c.lock().unwrap().textos.insert(n.to_owned(), valor.clone());
                self.avisar_con(&format!("text:{n}"), valor);
            }
            Evento::Envia(n, valor) => self.avisar_con(&format!("submit:{n}"), valor),
            Evento::Foco(si) => self.avisar(if si { "focus" } else { "blur" }, Value::Nil),
            Evento::Recibido(zona, tipo, datos) => {
                let quienes = self.c.lock().unwrap().manejadores.get(&format!("drop:{zona}")).cloned().unwrap_or_default();
                quienes.iter().for_each(|f| self.llamar(f, (datos.clone(), tipo.clone())));
            }
            Evento::Tecla(nombre, escribe) => {
                let Some(lua) = &self.lua else { return };
                let quienes = self.c.lock().unwrap().manejadores.get("key").cloned().unwrap_or_default();
                if let Ok(n) = lua.create_string(&nombre) {
                    quienes.iter().for_each(|f| self.llamar(f, (n.clone(), escribe.clone())));
                }
            }
            Evento::Demo => self.avisar("demo", Value::Nil),
            Evento::Hecho(n, v) => {
                self.c.lock().unwrap().hechos.insert(n.to_owned(), v as f64);
                // A quien escucha le llega como lo lee: `true`, `"critical"`, o un número.
                let tipo = self.c.lock().unwrap().tipos.get(n).cloned();
                let valor = match &self.lua {
                    Some(lua) => de_numero(lua, tipo.as_ref(), v as f64).unwrap_or(numero(v)),
                    None => numero(v),
                };
                self.avisar(&format!("fact:{n}"), valor);
            }
            Evento::Capa(capa, gana) => {
                let Some(lua) = &self.lua else { return };
                if let Ok(s) = lua.create_string(gana) {
                    self.avisar(&format!("layer:{capa}"), Value::String(s));
                }
            }
            // La escena se recargó: lo que tenga de nuevo ya se puede nombrar; lo que
            // ya se sabía, se sigue sabiendo.
            Evento::EscenaNueva(hechos, textos, permisos, modelos, tipos, plugins, sucesos) => {
                // Un plugin recibe la escena nueva con su propia definición: de ahí saca sus permisos.
                if let (Some(_), Some(def)) = (&self.prefijo, plugins.first()) {
                    let mut c = self.c.lock().unwrap();
                    c.sin_aprobar = !crate::permisos::aprobado(def);
                    self.definicion = Some(def.clone());
                }
                // La escena: los plugins que sigan, con lo nuevo; los que lleguen, arrancan; los que ya no estén, se van.
                if self.prefijo.is_none() {
                    let mut siguen: Vec<PluginVivo> = Vec::new();
                    for (k, p) in plugins.iter().enumerate() {
                        let nueva = Evento::EscenaNueva(hechos.clone(), textos.clone(), crate::permisos::los_que_valen(p), modelos.clone(), tipos.clone(), vec![p.clone()], sucesos.clone());
                        let vivo = match self.plugins.iter().position(|x| x.definicion.nombre == p.nombre && x.definicion.logica == p.logica) {
                            Some(i) => {
                                let mut v = self.plugins.remove(i);
                                v.definicion = p.clone();
                                v
                            }
                            None => {
                                let v = Self::arrancar(self.de_plugin(p, k + 100));
                                println!("logic  · plugin '{}' arrives", p.nombre);
                                let _ = v.buzon.send(nueva.clone());
                                let _ = v.buzon.send(Evento::RecargarLogica);
                                v
                            }
                        };
                        let _ = vivo.buzon.send(nueva);
                        siguen.push(vivo);
                    }
                    for fuera in std::mem::replace(&mut self.plugins, siguen) {
                        // Al soltar su buzón, su hilo acaba y para lo que dejó corriendo.
                        println!("logic  · plugin '{}' is gone", fuera.definicion.nombre);
                    }
                }
                let mut c = self.c.lock().unwrap();
                c.modelos = modelos;
                c.tipos = tipos.into_iter().collect();
                c.sucesos = sucesos.iter().map(|s| (*s).to_owned()).collect();
                // Los permisos sí se sustituyen: quitar uno de la escena lo quita ya.
                if c.permisos != permisos {
                    println!("logic  · permissions now: {}", en_claro(&permisos));
                    c.permisos = permisos;
                }
                for (n, v) in hechos {
                    c.hechos.entry(n.to_owned()).or_insert(v as f64);
                }
                for (n, v) in textos {
                    c.textos.entry(n.to_owned()).or_insert(v);
                }
            }
            Evento::Linea(id, linea) => {
                let f = self.c.lock().unwrap().en_marcha.get(&id).map(|x| x.0.clone());
                if let Some(f) = f {
                    self.llamar(&f, linea);
                }
            }
            Evento::Dato(nombre, valor) => {
                let Some(lua) = &self.lua else { return };
                let quienes = self.c.lock().unwrap().vigias.get(&nombre).cloned().unwrap_or_default();
                match a_lua(lua, &valor) {
                    Ok(v) => quienes.iter().for_each(|f| self.llamar(f, v.clone())),
                    Err(e) => eprintln!("logic  · {e}"),
                }
            }
            Evento::Proceso(id, salida, codigo) => {
                self.c.lock().unwrap().en_marcha.remove(&id);
                let f = self.c.lock().unwrap().procesos.remove(&id);
                if let Some(f) = f {
                    self.llamar(&f, (salida, codigo));
                }
            }
            _ => {}
        }
    }

    fn proxima(&self) -> Option<Instant> {
        self.c.lock().unwrap().temporizadores.iter().map(|t| t.cuando).min()
    }

    fn tic(&mut self, _: &mut Contexto) {
        let ahora = Instant::now();
        let vencidos: Vec<Function> = {
            let mut c = self.c.lock().unwrap();
            let f = c.temporizadores.iter().filter(|t| t.cuando <= ahora).map(|t| t.f.clone()).collect();
            for t in &mut c.temporizadores {
                if t.cuando <= ahora {
                    if let Some(d) = t.cada {
                        t.cuando = ahora + d;
                    }
                }
            }
            c.temporizadores.retain(|t| t.cuando > ahora);
            f
        };
        for f in vencidos {
            self.llamar(&f, ());
        }
    }
}
