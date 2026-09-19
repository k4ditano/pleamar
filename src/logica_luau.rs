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
    /// Hasta cuándo puede correr lo que está corriendo.
    limite: Option<Instant>,
}

fn en_claro(p: &crate::escena::Permisos) -> String {
    let lista = |l: &[String]| if l.is_empty() { "ninguno".to_owned() } else { l.join(", ") };
    format!("órdenes: {} · servicios: {}", lista(&p.ordenes), lista(&p.servicios))
}

/// ¿Puede esta lógica lanzar esa orden? Sin declarar, no; y el error dice qué escribir.
fn permiso_de_orden(c: &Mutex<Compartido>, orden: &str) -> mlua::Result<()> {
    if c.lock().unwrap().permisos.ordenes.iter().any(|o| o == orden) {
        return Ok(());
    }
    Err(mlua::Error::runtime(format!("la escena no da permiso para lanzar «{orden}». Si debe poder, decláralo en el .plm: permissions {{ run: \"{orden}\" }}")))
}

/// Lo mismo para un servicio: `audio.step` es del servicio `audio`.
fn permiso_de_servicio(c: &Mutex<Compartido>, nombre: &str) -> mlua::Result<()> {
    let servicio = nombre.split('.').next().unwrap_or(nombre);
    if c.lock().unwrap().permisos.servicios.iter().any(|s| s == servicio) {
        return Ok(());
    }
    Err(mlua::Error::runtime(format!("la escena no da permiso para usar el servicio «{servicio}». Si debe poder, decláralo en el .plm: permissions {{ services: \"{servicio}\" }}")))
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
    crate::lenguaje::parecido(k, conocidos).map_or(String::new(), |p| format!(". ¿Querías decir «{p}»?"))
}

pub struct GuionLuau {
    escena: String,
    logica: String,
    tx: Sender<ARender>,
    a_logica: Sender<Evento>,
    bloqueada: Arc<AtomicBool>,
    lua: Option<Lua>,
    c: Arc<Mutex<Compartido>>,
}

impl GuionLuau {
    pub fn nuevo(escena: &str, logica: &str, tx: Sender<ARender>, a_logica: Sender<Evento>, bloqueada: Arc<AtomicBool>) -> Self {
        GuionLuau { escena: escena.to_owned(), logica: logica.to_owned(), tx, a_logica, bloqueada, lua: None, c: Arc::default() }
    }

    /// Un estado de Luau nuevo, con la frontera puesta, y el fichero ejecutado.
    fn cargar(&mut self) {
        {
            let mut c = self.c.lock().unwrap();
            c.manejadores.clear();
            c.temporizadores.clear();
            c.procesos.clear();
            c.vigias.clear();
            // Lo que la lógica vieja dejó corriendo se para con ella.
            for (_, (_, hijo)) in c.en_marcha.drain() {
                if let Some(mut h) = hijo.lock().unwrap().take() {
                    let _ = h.kill();
                }
            }
        }
        let fuente = match std::fs::read_to_string(&self.logica) {
            Ok(f) => f,
            Err(e) => return eprintln!("lógica · {}: {e}", self.logica),
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
                println!("lógica · {} en marcha en {:.1} ms · {} manejadores, {} temporizadores", self.logica, t0.elapsed().as_secs_f32() * 1000.0, c.manejadores.values().map(Vec::len).sum::<usize>(), c.temporizadores.len());
                println!("lógica · permisos · {}", en_claro(&c.permisos));
            }
            Err(e) => eprintln!("lógica · la anterior sigue como estaba:\n{e}"),
        }
        self.c.lock().unwrap().limite = None;
    }

    fn preparar(&self) -> mlua::Result<Lua> {
        let lua = Lua::new();
        lua.set_memory_limit(MEMORIA)?;
        // En la caja de arena, Luau da por hecho que los globales no cambian y lee
        // `fact.open` UNA vez, al cargar el script. Estos sí cambian: hay que decírselo.
        lua.set_compiler(mlua::chunk::Compiler::new().set_mutable_globals(["fact", "text", "sys"]));
        let g = lua.globals();

        // Un manejador que no acaba no puede quedarse con el hilo para siempre.
        let c = self.c.clone();
        lua.set_interrupt(move |_| match c.lock().unwrap().limite {
            Some(l) if Instant::now() > l => Err(mlua::Error::runtime(format!("un manejador lleva más de {} s sin acabar: cortado", PACIENCIA.as_secs()))),
            _ => Ok(VmState::Continue),
        });

        // fact.open = true · fact.open
        let (tx, c) = (self.tx.clone(), self.c.clone());
        let poner = lua.create_function(move |_, (_, k, v): (Table, String, Value)| {
            let n = match v {
                Value::Boolean(b) => b as u8 as f64,
                Value::Integer(i) => i as f64,
                Value::Number(x) => x,
                otro => return Err(mlua::Error::runtime(format!("un hecho es un número o un sí/no, no un {}", otro.type_name()))),
            };
            // Un nombre mal escrito es un error aquí, con su línea, y no un aviso perdido en el render.
            if !c.lock().unwrap().hechos.contains_key(&k) {
                return Err(mlua::Error::runtime(format!("la escena no tiene ningún hecho «{k}»{}", pista(&k, c.lock().unwrap().hechos.keys()))));
            }
            c.lock().unwrap().hechos.insert(k.clone(), n);
            let _ = tx.send(ARender::Hecho(internar(&k), n as f32));
            Ok(())
        })?;
        let c = self.c.clone();
        let leer = lua.create_function(move |_, (_, k): (Table, String)| Ok(c.lock().unwrap().hechos.get(&k).copied()))?;
        g.set("fact", Self::tabla_viva(&lua, leer, poner)?)?;

        // text["notice.title"] = "…"
        let (tx, c) = (self.tx.clone(), self.c.clone());
        let poner = lua.create_function(move |_, (_, k, v): (Table, String, String)| {
            if !c.lock().unwrap().textos.contains_key(&k) {
                return Err(mlua::Error::runtime(format!("la escena no tiene ningún texto «{k}»{}", pista(&k, c.lock().unwrap().textos.keys()))));
            }
            c.lock().unwrap().textos.insert(k.clone(), v.clone());
            let _ = tx.send(ARender::Texto(internar(&k), v));
            Ok(())
        })?;
        let c = self.c.clone();
        let leer = lua.create_function(move |_, (_, k): (Table, String)| Ok(c.lock().unwrap().textos.get(&k).cloned()))?;
        g.set("text", Self::tabla_viva(&lua, leer, poner)?)?;

        let tx = self.tx.clone();
        g.set("emit", lua.create_function(move |_, n: String| Ok(tx.send(ARender::Suceso(internar(&n))).is_ok()))?)?;
        // focus("query") pone el cursor de texto en un campo; focus() lo quita.
        let tx = self.tx.clone();
        g.set("focus", lua.create_function(move |_, n: Option<String>| Ok(tx.send(ARender::Enfocar(n.map(|n| internar(&n)))).is_ok()))?)?;
        let tx = self.tx.clone();
        g.set("play", lua.create_function(move |_, n: String| Ok(tx.send(ARender::Gesto(internar(&n))).is_ok()))?)?;

        let c = self.c.clone();
        g.set("on", lua.create_function(move |_, (que, f): (String, Function)| {
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
                .map_err(|e| mlua::Error::runtime(format!("no puedo lanzar «{orden}»: {e}")))?;
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
            permiso_de_servicio(&c, &nombre)?;
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
            permiso_de_servicio(&c, &nombre)?;
            let args: Vec<Valor> = args.iter().map(|v| match v {
                Value::Boolean(b) => Valor::Si(*b),
                Value::Integer(i) => Valor::Num(*i as f64),
                Value::Number(n) => Valor::Num(*n),
                Value::String(s) => Valor::Texto(s.to_string_lossy()),
                _ => Valor::Nulo,
            }).collect();
            crate::plataforma::orden(&nombre, &args).map_err(mlua::Error::runtime)
        })?)?;
        g.set("sys", sys)?;

        g.set("log", lua.create_function(|_, v: MultiValue| {
            let trozos: Vec<String> = v.iter().map(|x| x.to_string().unwrap_or_else(|_| format!("{x:?}"))).collect();
            println!("luau   · {}", trozos.join(" "));
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
            eprintln!("lógica · {e}");
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
        let mut c = self.c.lock().unwrap();
        c.hechos = e.hechos.iter().map(|(n, v)| (n.to_string(), *v as f64)).collect();
        c.textos = e.textos.iter().map(|(n, v)| (n.to_string(), v.clone())).collect();
        c.permisos = e.permisos.clone();
        e
    }

    fn evento(&mut self, e: Evento, _: &mut Contexto) {
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
                self.avisar(&format!("fact:{n}"), numero(v));
            }
            Evento::Capa(capa, gana) => {
                let Some(lua) = &self.lua else { return };
                if let Ok(s) = lua.create_string(gana) {
                    self.avisar(&format!("layer:{capa}"), Value::String(s));
                }
            }
            // La escena se recargó: lo que tenga de nuevo ya se puede nombrar; lo que
            // ya se sabía, se sigue sabiendo.
            Evento::EscenaNueva(hechos, textos, permisos) => {
                let mut c = self.c.lock().unwrap();
                // Los permisos sí se sustituyen: quitar uno de la escena lo quita ya.
                if c.permisos != permisos {
                    println!("lógica · permisos ahora: {}", en_claro(&permisos));
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
                    Err(e) => eprintln!("lógica · {e}"),
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
