//! Del árbol a la escena: qué significa cada nodo, y si los nombres existen.
//!
//! Se lee en cuatro vueltas, para que el orden en que se escribe sea el que le
//! convenga a quien lee y no al programa: primero lo que se declara; luego los
//! nombres (`let`) y las capas; luego el dibujo, y al final las reglas, que ya
//! pueden nombrar cualquier forma. Solo un `let` tiene que ir antes de quien lo usa.

use super::arbol::{Entrada, Nodo};
use super::fichas::{Ficha, F};
use super::vocabulario as voz;
use super::{parecido, Fallo};
use crate::escena::*;
use std::collections::HashMap;
use std::time::Duration;

type R<T> = Result<T, Fallo>;

fn fijo(s: &str) -> &'static str {
    internar(s)
}

/// Un cursor sobre las fichas de una cabecera o de un valor.
struct Cur<'a> {
    f: &'a [Ficha],
    i: usize,
    fin: (usize, usize),
}

impl<'a> Cur<'a> {
    fn de(f: &'a [Ficha], linea: usize, col: usize) -> Self {
        let fin = f.last().map_or((linea, col), |u| (u.linea, u.col + 1));
        Cur { f, i: 0, fin }
    }
    fn mira(&self) -> Option<&'a F> {
        // Lo último que se ha mirado es casi siempre el nombre que se va a resolver: así
        // quien resuelve nombres sabe dónde está sin que cada llamada tenga que decírselo.
        if let Some(f) = self.f.get(self.i) {
            MIRANDO.with(|m| m.set((f.linea, f.col)));
        }
        self.f.get(self.i).map(|x| &x.f)
    }
    /// Se acaba de leer un nombre de los que se resuelven: es lo que el editor
    /// subraya al preguntar dónde se usa algo. Se apunta antes de avanzar.
    fn nombre_aqui(&self) {
        NOMBRADO.with(|n| n.set(MIRANDO.with(std::cell::Cell::get)));
    }
    fn pos(&self) -> (usize, usize) {
        self.f.get(self.i).map_or(self.fin, |x| (x.linea, x.col))
    }
    fn fallo<T>(&self, m: impl Into<String>) -> R<T> {
        let (l, c) = self.pos();
        Err(Fallo::en(l, c, m))
    }
    fn acabo(&self) -> bool {
        self.i >= self.f.len()
    }
    fn sim(&mut self, s: &str) -> bool {
        let si = matches!(self.mira(), Some(F::Sim(x)) if *x == s);
        self.i += si as usize;
        si
    }
    fn palabra(&mut self, p: &str) -> bool {
        let si = matches!(self.mira(), Some(F::Id(x)) if x == p);
        self.i += si as usize;
        si
    }
    fn exige_sim(&mut self, s: &str) -> R<()> {
        if self.sim(s) { Ok(()) } else { self.fallo(format!("expected '{s}' here")) }
    }
    fn exige_palabra(&mut self, p: &str) -> R<()> {
        if self.palabra(p) { Ok(()) } else { self.fallo(format!("expected '{p}' here")) }
    }
    fn id(&mut self, que: &str) -> R<String> {
        match self.mira() {
            Some(F::Id(x)) => {
                // `mira` acaba de dejar ahí este nombre: se guarda antes de avanzar.
                NOMBRADO.with(|n| n.set(MIRANDO.with(std::cell::Cell::get)));
                self.i += 1;
                Ok(x.clone())
            }
            _ => self.fallo(format!("expected {que} here")),
        }
    }
    fn num(&mut self) -> R<f32> {
        let menos = self.sim("-");
        match self.mira() {
            Some(F::Num(n)) => {
                self.i += 1;
                Ok(if menos { -n } else { *n })
            }
            _ => self.fallo("expected a number here"),
        }
    }
    fn dur(&mut self) -> R<Duration> {
        match self.mira() {
            Some(F::Dur(s)) => {
                self.i += 1;
                Ok(Duration::from_secs_f32(*s))
            }
            _ => self.fallo("expected a duration here, like 320ms or 14s"),
        }
    }
    fn cadena(&mut self) -> R<String> {
        match self.mira() {
            Some(F::Cadena(s)) => {
                self.i += 1;
                Ok(s.clone())
            }
            _ => self.fallo("expected a quoted string here"),
        }
    }
    /// Una palabra de una lista del vocabulario. Si no es ninguna, dice cuáles valen.
    fn una_de(&mut self, lista: &[&str], que: &str) -> R<String> {
        let palabra = self.id(que)?;
        if lista.contains(&palabra.as_str()) {
            return Ok(palabra);
        }
        self.i -= 1;
        let todas: Vec<String> = lista.iter().map(|s| s.to_string()).collect();
        let pista = parecido(&palabra, todas.iter()).map_or(String::new(), |p| format!(" Did you mean '{p}'?"));
        self.fallo(format!("'{palabra}' is not valid here: {que} is {}.{pista}", enumerar(lista)))
    }
    fn nada_mas(&self) -> R<()> {
        if self.acabo() { Ok(()) } else { self.fallo("this is left over here") }
    }
}

thread_local! {
    // Dónde está la última ficha que se ha mirado: (línea, columna).
    static MIRANDO: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
    // Y dónde estaba el último NOMBRE que se leyó, que no es lo mismo: para cuando se
    // resuelve ya se ha mirado la ficha siguiente. Es lo que el editor subraya.
    static NOMBRADO: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// Una forma con nombre: será zona si alguna regla la nombra, si se declaró
/// con `zone` o si lleva `active`.
struct Candidata {
    nombre: String,
    forma: Forma,
    activa: Option<Expr>,
    /// Si lo que la contiene no está (`show:`, una ficha que no existe), ella tampoco.
    visible: Option<Expr>,
    bajo: Vec<Transformacion>,
    forzada: bool,
    cursor: Cursor,
}

/// Lo que vale dentro de un componente o de una vuelta de `repeat`: sus
/// parámetros, y los nombres que declara, que fuera se llaman de otra manera
/// para que dos copias no se pisen.
#[derive(Clone, Default)]
struct Entorno {
    exprs: HashMap<String, Expr>,
    colores: HashMap<String, Color>,
    cadenas: HashMap<String, String>,
    /// Las cadenas con huecos, ya resueltas donde se escribieron: `Chip("{n.app}")`.
    contenidos: HashMap<String, Contenido>,
    alias: HashMap<String, String>,
    /// De esos nombres, los que tienen partes: `label.width` es de la medida `label`.
    con_partes: std::collections::HashSet<String>,
    sufijo: String,
    /// Dentro de un `for`: esta vuelta solo existe si su ficha existe.
    visible: Option<Expr>,
    /// Si es la copia de un componente: cuál, y en qué línea se puso.
    copia: Option<(String, usize)>,
    /// Los muelles que llegaron como parámetro: `~bounce`.
    muelles: HashMap<String, Muelle>,
    /// Marca de un hijo que viene de fuera del componente (`children`): se lee con
    /// los nombres de quien lo escribió, no con los de dentro.
    ambito_de_fuera: Option<Vec<Entorno>>,
    /// La copia de un componente de biblioteca: de qué fichero.
    biblioteca: Option<usize>,
    /// De una biblioteca `strict`: aquí dentro solo vale lo que se pide, lo que se
    /// declara y lo de la propia biblioteca.
    estricta: bool,
}

struct Componente<'a> {
    /// De una biblioteca `strict`.
    estricta: bool,
    /// Los huecos para hijos que tiene: `children` («») y `children header`.
    huecos: Vec<String>,
    parametros: Vec<Parametro>,
    nodo: &'a Nodo,
}

/// Una biblioteca importada: de qué fichero viene, cómo se llama, y su lógica si la tiene.
pub struct Biblioteca {
    pub fichero: usize,
    pub nombre: String,
    pub logica: Option<std::path::PathBuf>,
}

/// Lo que una copia trae para los huecos de su componente, y el ámbito de quien lo escribió.
struct HijosDeCopia<'a> {
    fuera: Vec<Entorno>,
    /// Por hueco («» es el que no tiene nombre): lo que va dentro, y si ya se ha puesto.
    huecos: Vec<(String, Vec<&'a Entrada>, bool)>,
}

/// Lo que un componente pide a quien lo usa. Con tipo, quien lo usa se entera al
/// momento de qué falta o de qué sobra; sin él, se adivina por lo que se le pase.
#[derive(Clone)]
struct Parametro {
    nombre: String,
    tipo: Option<String>,
    /// Lo que vale si no se le pasa: tal como se escribió, para leerlo donde se use.
    por_defecto: Option<Vec<Ficha>>,
}

struct Obra<'a> {
    e: Escena,
    /// Repartos que se desplazan: su zona, su propiedad, hasta dónde, cuánto por muesca y con qué muelle.
    scrolls: Vec<(String, PropId, Expr, Expr, Muelle, HechoId)>,
    /// De los repartos que se desplazan, cuáles son filas: se arrastran de lado.
    fila_de_scroll: std::collections::HashSet<String>,
    /// Superficies cuyo `open:` se resuelve al final: (cuál, el hecho, dónde está escrito).
    superficies_pendientes: Vec<(usize, &'a [Ficha], (usize, usize))>,
    /// Los nombres de los ficheros de los que está hecha, para decir dónde está algo.
    ficheros: &'a [String],
    /// Qué ficheros, por su número, son bibliotecas `strict`.
    estrictos: &'a [usize],
    bibliotecas: &'a [Biblioteca],
    /// La carpeta de cada fichero: una ruta relativa lo es al fichero que la escribe.
    carpetas: &'a [std::path::PathBuf],
    /// La frontera que declara cada biblioteca, por el número de su fichero: cómo se
    /// llama dentro (`now`) y cómo se llama de verdad (`Clock.now`).
    frontera_de: HashMap<usize, HashMap<String, String>>,
    /// Y los permisos que pide para su lógica.
    permisos_de: HashMap<usize, Permisos>,
    /// En qué vuelta va la lectura: `surface` lee sus propiedades en la 0 y su dibujo en la 2.
    vuelta: u8,
    /// Cada superficie y cada emergente miran a un trozo distinto del mismo plano.
    siguiente_origen: f32,
    /// Los nombres de los valores de los enumerados, como números: `critical` es 2.
    valores: HashMap<String, f32>,
    /// Los que están en dos enumerados con números distintos: sueltos no dicen nada.
    ambiguos: std::collections::HashSet<String>,
    /// Lo que cada copia en curso trae para el `children` de su componente: los
    /// nodos, el ámbito de quien los escribió, y si ya se han puesto.
    hijos_de_copia: Vec<HijosDeCopia<'a>>,
    /// Lo que declaran las bibliotecas: un componente `strict` puede leerlo.
    de_biblioteca: std::collections::HashSet<String>,
    /// Lo que un componente `strict` ha leído de la escena sin pedirlo: (componente, nombre).
    sin_pedir: std::cell::RefCell<Vec<(String, String, (usize, usize))>>,
    sin_vigilar: std::cell::Cell<bool>,
    props: HashMap<String, PropId>,
    hechos: HashMap<String, HechoId>,
    sucesos: HashMap<String, SucesoId>,
    textos: HashMap<String, TextoId>,
    imagenes: HashMap<String, ImagenId>,
    modelos: HashMap<String, usize>,
    medidas: HashMap<String, (PropId, PropId)>,
    gestos: HashMap<String, GestoId>,
    zonas: HashMap<String, ZonaId>,
    lets: HashMap<String, Expr>,
    colores: HashMap<String, Color>,
    muelles: HashMap<String, Muelle>,
    candidatas: Vec<Candidata>,
    /// Las reglas se dejan para el final: así pueden nombrar formas que se
    /// pintan más abajo.
    reglas: Vec<(&'a Nodo, Vec<Entorno>)>,
    fallos: Vec<Fallo>,
    /// Cada nombre que se declara, con su sitio: para el editor.
    declarados: Vec<Simbolo>,
    /// Y cada vez que se nombra algo, para «dónde se usa» y para cambiarlo de nombre.
    /// Como los nombres se resuelven sin poder escribir (`&self`), va en una celda.
    usados: std::cell::RefCell<Vec<Simbolo>>,
    /// La sentencia que se está leyendo (`fact`, `prop`…), para saber de qué clase
    /// es lo que se declare dentro de ella.
    clase_actual: String,
    entornos: Vec<Entorno>,
    componentes: HashMap<String, Componente<'a>>,
    copias: usize,
    /// Dentro de un `row` o un `column`, un hijo no dice dónde va: va a su hueco.
    en_hueco: bool,
    /// Cuánto ocupó lo último que se pintó, para quien esté repartiendo huecos.
    ultimo_tam: Option<(Expr, Expr)>,
    /// La medida que un layout le impone al texto que va a pintar.
    medida_impuesta: Option<(PropId, PropId)>,
    teclado_pendiente: Option<(&'a [Ficha], (usize, usize))>,
    /// Las transformaciones bajo las que se está pintando: una zona las hereda.
    bajo: Vec<Transformacion>,
}

/// Un nombre que la escena declara, con su sitio: lo que el editor necesita para
/// completar, para ir a donde nació y para enseñar el esquema del fichero.
#[derive(Clone, Debug)]
pub struct Simbolo {
    /// Como se escribió: `hit`, sin el sufijo del ámbito.
    pub local: String,
    /// Con qué sentencia se declaró: `fact`, `prop`, `component`…
    pub clase: String,
    pub linea: usize,
    pub col: usize,
}

pub fn levantar<'a>(arbol: &'a [Entrada], ficheros: &'a [String], carpetas: &'a [std::path::PathBuf], estrictos: &'a [usize], bibliotecas: &'a [Biblioteca]) -> (Result<Escena, Vec<Fallo>>, Vec<Simbolo>) {
    let escena = match arbol {
        [Entrada::Nodo(n)] if matches!(n.cabeza.first().map(|f| &f.f), Some(F::Id(p)) if p == "scene") => n,
        [Entrada::Nodo(n)] if matches!(n.cabeza.first().map(|f| &f.f), Some(F::Id(p)) if p == "library") => {
            return (Err(vec![Fallo::en(n.linea, n.col, "this is a library: it is not opened, it is imported from a scene (`import \"…\"`)")]), Vec::new());
        }
        _ => return (Err(vec![Fallo::en(1, 1, "a file is a scene: as many `import`s as you want, then `scene Name { … }`")]), Vec::new()),
    };
    let mut o = Obra {
        e: Escena::default(),
        props: HashMap::new(), hechos: HashMap::new(), sucesos: HashMap::new(), textos: HashMap::new(), imagenes: HashMap::new(), modelos: HashMap::new(),
        medidas: HashMap::new(), gestos: HashMap::new(), zonas: HashMap::new(), lets: HashMap::new(), colores: HashMap::new(),
        muelles: voz::MUELLES.iter().map(|n| ((*n).to_owned(), match *n {
            "lively" => Muelle::VIVO,
            "calm" => Muelle::SERENO,
            "quick" => Muelle::RAPIDO,
            "slow" => Muelle::LENTO,
            "gentle" => Muelle::SUAVE,
            "pose" => Muelle::POSE,
            otro => unreachable!("'{otro}' is in the vocabulary, but it has no stiffness or damping"),
        })).collect(),
        bajo: Vec::new(), candidatas: Vec::new(), reglas: Vec::new(), fallos: Vec::new(), declarados: Vec::new(), usados: Default::default(), clase_actual: String::new(),
        scrolls: Vec::new(), fila_de_scroll: Default::default(), superficies_pendientes: Vec::new(), ficheros, carpetas, estrictos, bibliotecas, frontera_de: HashMap::new(), permisos_de: HashMap::new(), vuelta: 0, siguiente_origen: 0.0, valores: HashMap::new(), ambiguos: Default::default(), hijos_de_copia: Vec::new(), de_biblioteca: Default::default(), sin_pedir: Default::default(), sin_vigilar: Default::default(), entornos: Vec::new(), componentes: HashMap::new(), copias: 0, en_hueco: false, ultimo_tam: None, medida_impuesta: None, teclado_pendiente: None,
    };
    // Dos hechos que siempre existen: lo que mide la superficie de verdad. El
    // render los pone cuando el compositor la configura.
    // …y lo que una regla puede leer del ratón mientras se dispara.
    for n in ["screen.width", "screen.height", "pointer.x", "pointer.y", "local.x", "local.y", "drag.dx", "drag.dy", "wheel"] {
        let h = o.e.hecho(n, 0.0);
        o.hechos.insert(n.into(), h);
    }
    // Un suceso que siempre existe: lo dispara `--demo`, para escenas sin ratón.
    let demo = o.e.suceso("demo");
    o.sucesos.insert("demo".into(), demo);
    let Some(cuerpo) = escena.cuerpo.as_ref() else {
        return (Err(vec![Fallo::en(escena.linea, escena.col, "this scene is missing its `{ … }` block")]), Vec::new());
    };
    o.adelantar_medidas(cuerpo);
    // Cuatro vueltas: declaraciones; nombres y capas; dibujo; reglas.
    for vuelta in 0..3 {
        o.vuelta = vuelta;
        // Las superficies ya se conocen: si alguna se repite por monitor, sus repartos
        // con nombre tienen una medida por copia (`desks#screen1.width`).
        if vuelta == 1 {
            let instancias: Vec<usize> = o.e.superficies.iter().filter(|s| matches!(s.pantallas, Pantallas::Numero(_))).map(|s| s.instancia).collect();
            for k in instancias {
                o.entornos.push(o.ambito_de_pantalla(k));
                o.adelantar_medidas(cuerpo);
                o.entornos.pop();
            }
        }
        // Una `surface` se lee dos veces: en la 0 sus propiedades, y en la 2 lo que dibuja.
        let de_esta: Vec<&Entrada> = cuerpo.iter().filter(|e| vuelta_de(e) == vuelta || (vuelta == 2 && es_superficie(e))).collect();
        // El dibujo suelto es de la superficie de la escena. Si esa se repite por monitor
        // (`screens: each`), se repite con ella: una copia por pantalla, con lo suyo.
        let copias: Vec<(f32, f32, usize)> = if vuelta == 2 {
            o.e.superficies.iter().filter(|s| s.nombre.is_empty() && matches!(s.pantallas, Pantallas::Numero(_))).map(|s| (s.origen.0, s.origen.1, s.instancia)).collect()
        } else {
            Vec::new()
        };
        if copias.is_empty() {
            o.grupo(de_esta.into_iter());
            continue;
        }
        for (ox, oy, k) in copias {
            let marca = o.reglas.len();
            o.entornos.push(o.ambito_de_pantalla(k));
            let t = Transformacion { mueve: (ox.into(), oy.into()), ..Transformacion::en((0.0.into(), 0.0.into())) };
            o.e.pintar(Instr::Transformar(Some(t.clone())));
            o.bajo.push(t);
            o.grupo(de_esta.clone().into_iter());
            o.bajo.pop();
            o.e.pintar(Instr::Transformar(None));
            o.cerrar_ambito(marca);
        }
    }
    o.zonas_de_verdad();
    for (n, entornos) in std::mem::take(&mut o.reglas) {
        o.entornos = entornos;
        let mut c = Cur::de(&n.cabeza, n.linea, n.col);
        let palabra = c.id("on o every").unwrap_or_default();
        if let Err(f) = o.regla(n, &palabra, &mut c) {
            o.anotar(f);
        }
    }
    if let Some((fichas, (l, col))) = o.teclado_pendiente.take() {
        o.entornos.clear();
        let mut c = Cur::de(fichas, l, col);
        match o.expr(&mut c) {
            Ok(e) => o.e.teclado_mientras = Some(e),
            Err(f) => o.fallos.push(f),
        }
    }
    // Hasta que llegue la de verdad, la que pide el fichero.
    let (w, h) = (o.e.superficie().ancho as f32, o.e.superficie().alto as f32);
    o.e.hechos[0].1 = if w > 0.0 { w } else { 1920.0 };
    o.e.hechos[1].1 = h;
    for (cual, fichas, (l, col)) in std::mem::take(&mut o.superficies_pendientes) {
        o.entornos.clear();
        let mut c = Cur::de(fichas, l, col);
        match o.expr(&mut c) {
            Ok(e) => o.e.superficies[cual].abierta = Some(e),
            Err(f) => o.fallos.push(f),
        }
    }
    if o.e.superficies.is_empty() {
        o.e.superficies.push(Superficie::default());
    }
    // Una biblioteca con un `.luau` al lado es un plugin: su lógica, con sus permisos.
    for b in bibliotecas {
        if let Some(logica) = &b.logica {
            o.e.plugins.push(Plugin { nombre: b.nombre.clone(), logica: logica.clone(), permisos: o.permisos_de.get(&b.fichero).cloned().unwrap_or_default() });
        } else if o.permisos_de.contains_key(&b.fichero) {
            o.fallos.push(Fallo::en(b.fichero * super::POR_FICHERO + 1, 1, format!("library '{}' asks for permissions, but has no logic to use them: its `.luau` is missing next to it", b.nombre)));
        }
    }
    // Lo que un componente de biblioteca `strict` leyó de la escena sin pedirlo.
    for (componente, nombre, donde) in o.sin_pedir.take() {
        o.entornos.clear();
        o.anotar(Fallo::en(donde.0, donde.1, format!("'{componente}' belongs to a `strict` library and reads '{nombre}', which is the scene\'s, without asking for it. Take it as a parameter, or declare it in the library")));
    }
    // Los nombres que se declararon, uno por nombre: las cuatro vueltas y las copias
    // de un `repeat` declaran el mismo muchas veces, y al editor le vale el primer sitio.
    let mut vistos = std::collections::HashSet::new();
    let mut simbolos: Vec<Simbolo> = std::mem::take(&mut o.declarados).into_iter().filter(|s| vistos.insert(s.local.clone())).collect();
    // Y cada vez que se nombró algo, una vez por sitio: las vueltas repiten.
    let mut donde = std::collections::HashSet::new();
    simbolos.extend(o.usados.take().into_iter().filter(|s| donde.insert((s.local.clone(), s.linea, s.col))));
    (if o.fallos.is_empty() { Ok(o.e) } else { Err(o.fallos) }, simbolos)
}

/// `surface … { … }`, que se lee en dos vueltas.
fn es_superficie(e: &Entrada) -> bool {
    matches!(e, Entrada::Nodo(n) if matches!(n.cabeza.first().map(|f| &f.f), Some(F::Id(p)) if p == "surface"))
}

/// En qué vuelta se lee cada sentencia del nivel de la escena.
fn vuelta_de(e: &Entrada) -> u8 {
    let Entrada::Nodo(n) = e else { return 2 };
    let es_asignacion = matches!(n.cabeza.get(2).map(|x| &x.f), Some(F::Sim("=")));
    match n.cabeza.first().map(|f| &f.f) {
        Some(F::Id(p)) => match p.as_str() {
            "surface" | "permissions" | "model" | "service" | "spring" | "prop" | "pose" | "fact" | "event" | "measure" | "component" => 0,
            "text" | "image" if es_asignacion => 0,
            "let" | "layer" => 1,
            _ => 2,
        },
        _ => 2,
    }
}

impl<'a> Obra<'a> {
    // ── nombres ─────────────────────────────────────────────────

    /// `item.$i.title`, con `i` valiendo 3, es `item.3.title`.
    fn interpolar(&self, n: &str) -> String {
        if !n.contains('$') {
            return n.to_owned();
        }
        n.split('.')
            .map(|trozo| match trozo.strip_prefix('$') {
                Some(var) => match self.entornos.iter().rev().find_map(|e| e.exprs.get(var)) {
                    Some(Expr::K(v)) => format!("{}", *v as i64),
                    _ => trozo.to_owned(),
                },
                None => trozo.to_owned(),
            })
            .collect::<Vec<_>>()
            .join(".")
    }

    fn interpolar_en(&self, n: &str) -> String {
        self.interpolar(n)
    }

    /// Cómo se llama de verdad un nombre visto desde aquí dentro: lo que declaró
    /// esta copia de un componente lleva su sufijo. Vale para el nombre entero o
    /// para su principio: `label.width` es de la medida `label`.
    fn global(&self, n: &str) -> String {
        // Dónde se ha nombrado esto: es lo que el editor enseña en «dónde se usa».
        if !self.sin_vigilar.get() {
            let (linea, col) = NOMBRADO.with(|m| m.get());
            if linea > 0 {
                self.usados.borrow_mut().push(Simbolo { local: n.to_owned(), clase: "use".to_owned(), linea, col });
            }
        }
        let n = self.interpolar(n);
        for e in self.entornos.iter().rev() {
            let mut hasta = n.len();
            loop {
                // El nombre entero vale siempre; su principio, solo si es de algo con
                // partes. Si no, una zona `hit` se comería al texto `hit.3`.
                if let Some(g) = e.alias.get(&n[..hasta]).filter(|_| hasta == n.len() || e.con_partes.contains(&n[..hasta])) {
                    return format!("{g}{}", &n[hasta..]);
                }
                match n[..hasta].rfind('.') {
                    Some(p) => hasta = p,
                    None => break,
                }
            }
        }
        // Dentro de un componente de una biblioteca, `now` es el `Clock.now` de su frontera.
        // (O en el propio nivel de la biblioteca: un gesto suyo que mueve una pose suya.)
        let de_biblioteca = self.entornos.iter().rev().find_map(|e| e.biblioteca).or_else(|| {
            let k = MIRANDO.with(|m| m.get().0) / super::POR_FICHERO;
            (self.entornos.is_empty() && k > 0).then_some(k)
        });
        if let Some(mia) = de_biblioteca.and_then(|k| self.frontera_de.get(&k)) {
            let mut hasta = n.len();
            loop {
                if let Some(g) = mia.get(&n[..hasta]) {
                    return format!("{g}{}", &n[hasta..]);
                }
                match n[..hasta].rfind('.') {
                    Some(p) => hasta = p,
                    None => break,
                }
            }
        }
        self.vigilar(&n);
        n
    }

    /// Dentro de un componente de biblioteca `strict`, un nombre que no es suyo, ni de
    /// su biblioteca, ni de los que siempre existen, es algo que lee de la escena sin
    /// haberlo pedido. Se apunta; se dice al acabar.
    fn vigilar(&self, nombre: &str) {
        if self.sin_vigilar.get() {
            return;
        }
        let Some(e) = self.entornos.iter().rev().find(|e| e.copia.is_some()) else { return };
        if !e.estricta || self.de_biblioteca.contains(nombre) || self.valores.contains_key(nombre) {
            return;
        }
        // Un parámetro, o un `let` del componente: es suyo aunque no tenga sufijo.
        if self.entornos.iter().any(|e| e.exprs.contains_key(nombre) || e.colores.contains_key(nombre) || e.cadenas.contains_key(nombre) || e.muelles.contains_key(nombre)) {
            return;
        }
        let de_siempre = ["screen.", "pointer.", "local.", "drag."].iter().any(|p| nombre.starts_with(p)) || nombre == "wheel";
        // Lo que declaró la propia copia lleva su sufijo; lo que no, es de fuera.
        let suyo = self.entornos.iter().any(|e| !e.sufijo.is_empty() && nombre.ends_with(&e.sufijo));
        if !de_siempre && !suyo {
            let componente = e.copia.as_ref().unwrap().0.clone();
            let mut v = self.sin_pedir.borrow_mut();
            if !v.iter().any(|(c, n, _)| *c == componente && n == nombre) {
                v.push((componente, nombre.to_owned(), MIRANDO.with(|m| m.get())));
            }
        }
    }

    /// El nombre con el que se declara algo desde aquí dentro.
    fn declarar(&mut self, local: &str) -> String {
        let interpolado = self.interpolar(local);
        // El sitio del nombre que se acaba de leer, si es de esta misma línea.
        let (linea, col) = match (MIRANDO.with(std::cell::Cell::get), NOMBRADO.with(std::cell::Cell::get)) {
            (m, n) if n.0 == m.0 => n,
            (m, _) => m,
        };
        if linea > 0 {
            let clase = if self.clase_actual.is_empty() { "name".to_owned() } else { self.clase_actual.clone() };
            self.declarados.push(Simbolo { local: interpolado.clone(), clase, linea, col });
        }
        match self.entornos.last_mut() {
            Some(e) if !e.sufijo.is_empty() && !local.contains('$') => {
                let g = format!("{interpolado}{}", e.sufijo);
                e.alias.insert(interpolado, g.clone());
                g
            }
            _ => {
                // En el nivel de una biblioteca: su frontera vive bajo su nombre, para que dos
                // plugins puedan tener cada uno su `count` sin pisarse, y sin pisar a la escena.
                let fichero = MIRANDO.with(|m| m.get().0) / super::POR_FICHERO;
                match self.bibliotecas.iter().find(|b| b.fichero == fichero && fichero > 0) {
                    Some(b) => {
                        let g = format!("{}.{interpolado}", b.nombre);
                        self.frontera_de.entry(fichero).or_default().insert(interpolado, g.clone());
                        g
                    }
                    None => interpolado,
                }
            }
        }
    }

    /// Un fallo más, hasta ocho. El mismo fallo en el mismo sitio se dice una vez:
    /// un componente mal escrito falla en cada copia, y son la misma errata.
    fn anotar(&mut self, mut f: Fallo) {
        // Un fallo dentro de un componente —que puede estar en otro fichero— dice también
        // desde dónde se usó: a menudo lo que está mal es lo que se le pasó.
        if let Some((componente, linea)) = self.entornos.iter().rev().find_map(|e| e.copia.as_ref()) {
            if *linea != f.linea {
                f.mensaje = format!("{} (inside '{componente}', used at {})", f.mensaje, super::sitio(self.ficheros, *linea));
            }
        }
        let repetido = self.fallos.iter().any(|g| g.linea == f.linea && g.col == f.col && g.mensaje == f.mensaje);
        if !repetido && self.fallos.len() < 8 {
            self.fallos.push(f);
        }
    }

    fn desconocido<T>(&self, c: &Cur, que: &str, nombre: &str, conocidos: Vec<&String>) -> R<T> {
        let (l, col) = c.f.get(c.i.saturating_sub(1)).map_or(c.fin, |x| (x.linea, x.col));
        // ¿Es el campo de una ficha? Entonces lo que importa son los campos de su modelo,
        // no que por dentro se llame `rows.3.lable`.
        let entero = self.global(nombre);
        let de_ficha = entero.rsplit_once('.').and_then(|(ficha, campo)| {
            let (modelo, k) = ficha.rsplit_once('.')?;
            k.parse::<usize>().ok()?;
            Some((self.e.modelos.iter().find(|m| m.nombre == modelo)?, campo))
        });
        if let Some((m, campo)) = de_ficha {
            // Existe, pero es de otra clase: un número donde hace falta un texto, o al revés.
            if let Some(c) = m.campos.iter().find(|c| c.nombre == campo) {
                let (es, hace_falta) = match c.tipo {
                    TipoDeCampo::Texto => ("text", "a number (number, bool or an enum)"),
                    TipoDeCampo::Imagen(..) => ("image", "a number (number, bool or an enum)"),
                    TipoDeCampo::Numero => ("number", "a text"),
                    TipoDeCampo::Bool => ("bool", "a text"),
                    TipoDeCampo::Enum(_) => ("an enum", "a text"),
                    TipoDeCampo::Lista(_) => ("a list", "a plain field: walk it with `for`"),
                };
                return Err(Fallo::en(l, col, format!("field '{campo}' of '{}' is {es}, and here {hace_falta} is needed.", m.nombre)));
            }
            let mut campos: Vec<String> = m.campos.iter().map(|c| c.nombre.clone()).collect();
            campos.push("index".into());
            let pista = parecido(campo, campos.iter()).map_or(String::new(), |p| format!(" Did you mean '{p}'?"));
            let de_que = match que { "no text" => " of type text", _ => "" };
            return Err(Fallo::en(l, col, format!("records of '{}' have no field{de_que} called '{campo}'.{pista} Its fields are: {}.", m.nombre, campos.join(", "))));
        }
        let pista = parecido(nombre, conocidos.into_iter()).map_or(String::new(), |p| format!(" Did you mean '{p}'?"));
        Err(Fallo::en(l, col, format!("there is {que} called '{nombre}'.{pista} A `let` has to come before whatever uses it; everything else can go anywhere.")))
    }

    fn prop(&self, c: &mut Cur) -> R<PropId> {
        let n = self.global(&c.id("a property name")?);
        match self.props.get(&n) {
            Some(p) => Ok(*p),
            None => self.desconocido(c, "no property", &n, self.props.keys().collect()),
        }
    }
    fn hecho(&self, c: &mut Cur) -> R<HechoId> {
        let n = self.global(&c.id("a fact name")?);
        match self.hechos.get(&n) {
            Some(h) => Ok(*h),
            None => self.desconocido(c, "no fact", &n, self.hechos.keys().collect()),
        }
    }
    fn suceso(&self, c: &mut Cur) -> R<SucesoId> {
        let n = self.global(&c.id("an event name")?);
        match self.sucesos.get(&n) {
            Some(s) => Ok(*s),
            None => self.desconocido(c, "no event", &n, self.sucesos.keys().collect()),
        }
    }
    fn sucesos(&self, c: &mut Cur) -> R<Vec<SucesoId>> {
        let mut v = vec![self.suceso(c)?];
        while c.sim(",") {
            v.push(self.suceso(c)?);
        }
        Ok(v)
    }
    fn zona(&self, c: &mut Cur) -> R<ZonaId> {
        let n = self.global(&c.id("the name of a shape or a zone")?);
        match self.zonas.get(&n) {
            Some(z) => Ok(*z),
            None => self.desconocido(c, "no named shape or zone", &n, self.zonas.keys().collect()),
        }
    }
    fn muelle(&self, c: &mut Cur) -> R<Muelle> {
        // `~620ms`: un muelle dicho en tiempo. Llega ahí en ese rato y no rebota,
        // que es como están escritos los contratos de animación de la casa.
        if let Some(F::Dur(_)) = c.mira() {
            let t = c.dur()?.as_secs_f32().max(0.016);
            return Ok(Muelle::en(t));
        }
        let n = c.id("a spring name")?;
        if n == "spring" {
            c.exige_sim("(")?;
            let rigidez = c.num()?;
            c.exige_sim(",")?;
            let freno = c.num()?;
            c.exige_sim(")")?;
            return Ok(Muelle { rigidez, freno });
        }
        if let Some(m) = self.entornos.iter().rev().find_map(|e| e.muelles.get(&n)) {
            return Ok(*m);
        }
        if !voz::MUELLES.contains(&n.as_str()) {
            self.vigilar(&n);
        }
        match self.muelles.get(&n) {
            Some(m) => Ok(*m),
            None => self.desconocido(c, "no spring", &n, self.muelles.keys().collect()),
        }
    }

    // ── expresiones ─────────────────────────────────────────────

    fn expr(&self, c: &mut Cur) -> R<Expr> {
        let mut a = self.expr_y(c)?;
        while c.palabra("or") {
            a = a.o(self.expr_y(c)?);
        }
        Ok(a)
    }
    fn expr_y(&self, c: &mut Cur) -> R<Expr> {
        let mut a = self.expr_no(c)?;
        while c.palabra("and") {
            a = a.y(self.expr_no(c)?);
        }
        Ok(a)
    }
    fn expr_no(&self, c: &mut Cur) -> R<Expr> {
        if c.palabra("not") {
            return Ok(self.expr_no(c)?.no());
        }
        let desde = c.i;
        let a = self.expr_suma(c)?;
        let enumerado = self.enumerado_suelto(c, desde);
        for (s, f) in [(">=", 0), ("<=", 1), ("==", 4), ("!=", 5), (">", 2), ("<", 3)] {
            if c.sim(s) {
                // Un enumerado se compara con SUS valores: el nombre se busca en su lista, no en
                // la de todos (así dos enumerados pueden tener cada uno su `normal`).
                let b = match (&enumerado, c.mira(), c.f.get(c.i + 1).map(|x| &x.f)) {
                    (Some((hecho, nombres)), Some(F::Id(v)), sigue) if !matches!(sigue, Some(F::Sim("("))) && self.es_valor(v) => {
                        match nombres.iter().position(|n| n == v) {
                            Some(k) => {
                                c.i += 1;
                                Expr::K(k as f32)
                            }
                            None => return c.fallo(format!("'{v}' is not a value of '{hecho}': it can be {}", enumerar(&nombres.iter().map(String::as_str).collect::<Vec<_>>()))),
                        }
                    }
                    _ => self.expr_suma(c)?,
                };
                // Iguales es «a menos de una milésima»: son números con coma, y un muelle nunca llega del todo.
                let distintos = |a: Expr, b: Expr| (a - b).abs().mayor(Expr::K(0.001));
                return Ok(match f {
                    0 => b.mayor(a).no(),
                    1 => a.mayor(b).no(),
                    2 => a.mayor(b),
                    3 => b.mayor(a),
                    4 => distintos(a, b).no(),
                    _ => distintos(a, b),
                });
            }
        }
        Ok(a)
    }
    fn expr_suma(&self, c: &mut Cur) -> R<Expr> {
        let desde = c.i;
        let mut a = self.expr_prod(c)?;
        loop {
            let hasta = c.i;
            let op = if c.sim("+") { '+' } else if c.sim("-") { '-' } else { return Ok(a) };
            self.sin_cuentas(c, desde, hasta)?;
            let otro = c.i;
            let b = self.expr_prod(c)?;
            self.sin_cuentas(c, otro, c.i)?;
            a = if op == '+' { a + b } else { a - b };
        }
    }
    fn expr_prod(&self, c: &mut Cur) -> R<Expr> {
        let desde = c.i;
        let mut a = self.expr_uno(c)?;
        loop {
            let hasta = c.i;
            let op = if c.sim("*") { '*' } else if c.sim("/") { '/' } else { return Ok(a) };
            self.sin_cuentas(c, desde, hasta)?;
            let otro = c.i;
            let b = self.expr_uno(c)?;
            self.sin_cuentas(c, otro, c.i)?;
            a = if op == '*' { a * b } else { a / b };
        }
    }

    /// Si lo leído desde `desde` es un hecho enumerado a secas —`mode`, `n.urgency`—: cuál, y sus valores.
    fn enumerado_suelto(&self, c: &Cur, desde: usize) -> Option<(String, Vec<String>)> {
        self.enumerado_entre(c, desde, c.i)
    }

    fn enumerado_entre(&self, c: &Cur, desde: usize, hasta: usize) -> Option<(String, Vec<String>)> {
        let [Ficha { f: F::Id(n), .. }] = c.f.get(desde..hasta)? else { return None };
        let g = self.global_callado(n);
        match self.e.tipos.iter().find(|(x, _)| *x == g) {
            Some((_, TipoDeHecho::Enum(nombres))) => Some((n.clone(), nombres.clone())),
            _ => None,
        }
    }

    /// Con un enumerado no se hacen cuentas: `mode + 1` no significa nada. (Con un sí o no, sí:
    /// `r.separator * 21` es como se escribe «21 si es un separador».)
    fn sin_cuentas(&self, c: &Cur, desde: usize, hasta: usize) -> R<()> {
        match self.enumerado_entre(c, desde, hasta) {
            Some((hecho, nombres)) => {
                let f = &c.f[desde];
                Err(Fallo::en(f.linea, f.col, format!("'{hecho}' is an enum ({}): you do not do arithmetic with it. You compare it: `{hecho} == {}`", nombres.join(", "), nombres.last().cloned().unwrap_or_default())))
            }
            None => Ok(()),
        }
    }

    /// Si ese nombre es el valor de algún enumerado (y no otra cosa de la escena).
    fn es_valor(&self, n: &str) -> bool {
        self.valores.contains_key(n) || self.ambiguos.contains(n)
    }

    /// `global`, sin que cuente como haber leído nada (para mirar qué es un nombre).
    fn global_callado(&self, n: &str) -> String {
        let antes = self.sin_vigilar.replace(true);
        let g = self.global(n);
        self.sin_vigilar.set(antes);
        g
    }
    fn expr_uno(&self, c: &mut Cur) -> R<Expr> {
        if c.sim("-") {
            return Ok(Expr::K(0.0) - self.expr_uno(c)?);
        }
        if c.sim("(") {
            let e = self.expr(c)?;
            c.exige_sim(")")?;
            return Ok(e);
        }
        match c.mira() {
            Some(F::Num(n)) => {
                c.i += 1;
                Ok(Expr::K(*n))
            }
            Some(F::Dur(s)) => {
                c.i += 1;
                Ok(Expr::K(*s))
            }
            Some(F::Id(n)) => {
                c.nombre_aqui();
                c.i += 1;
                if c.sim("(") {
                    return self.funcion(n, c);
                }
                match n.as_str() {
                    "true" => return Ok(Expr::K(1.0)),
                    "false" => return Ok(Expr::K(0.0)),
                    _ => {}
                }
                // Primero lo de dentro —parámetros y `let` del componente—, luego lo de fuera.
                if let Some(e) = self.entornos.iter().rev().find_map(|e| e.exprs.get(n)) {
                    return Ok(e.clone());
                }
                let n = &self.global(n);
                if let Some(e) = self.lets.get(n) {
                    Ok(e.clone())
                } else if let Some(p) = self.props.get(n) {
                    Ok(p.e())
                } else if let Some(h) = self.hechos.get(n) {
                    Ok(h.e())
                } else if let Some(v) = self.valores.get(n) {
                    // El valor de un enumerado: `mode == critical`.
                    Ok(Expr::K(*v))
                } else if let Some(k) = n.rsplit_once('.').and_then(|(hecho, valor)| match self.e.tipos.iter().find(|(x, _)| x == hecho) {
                    Some((_, TipoDeHecho::Enum(nombres))) => nombres.iter().position(|x| x == valor),
                    _ => None,
                }) {
                    // La forma larga, que no se confunde con nada: `mode.critical`.
                    Ok(Expr::K(k as f32))
                } else if self.ambiguos.contains(n.as_str()) {
                    c.fallo(format!("'{n}' is a value of several enums, with different numbers: here there is no telling which. Compare it with its fact (`mode == {n}`) or write it in full (`mode.{n}`)"))
                } else {
                    let conocidos: Vec<&String> = self.lets.keys().chain(self.props.keys()).chain(self.hechos.keys()).chain(self.valores.keys()).collect();
                    self.desconocido(c, "nothing", n, conocidos)
                }
            }
            _ => c.fallo("expected a number, a name or a parenthesis here"),
        }
    }
    fn funcion(&self, nombre: &str, c: &mut Cur) -> R<Expr> {
        if !voz::FUNCIONES.contains(&nombre) {
            let todas: Vec<String> = voz::FUNCIONES.iter().map(|s| s.to_string()).collect();
            let pista = parecido(nombre, todas.iter()).map_or(String::new(), |p| format!(" Did you mean '{p}'?"));
            return c.fallo(format!("I don\'t know the function '{nombre}': there are {}.{pista}", voz::FUNCIONES.join(", ")));
        }
        let (l, col) = c.f.get(c.i.saturating_sub(2)).map_or(c.fin, |x| (x.linea, x.col));
        if nombre == "vel" {
            let p = self.prop(c)?;
            c.exige_sim(")")?;
            return Ok(p.vel());
        }
        let mut a = Vec::new();
        if !c.sim(")") {
            loop {
                a.push(self.expr(c)?);
                if c.sim(")") {
                    break;
                }
                c.exige_sim(",")?;
            }
        }
        let cte = |e: &Expr| match e {
            Expr::K(v) => Ok(*v),
            _ => Err(Fallo::en(l, col, format!("in '{nombre}', the first two have to be numbers"))),
        };
        let mut a = a.into_iter();
        let mut toma = || a.next().ok_or_else(|| Fallo::en(l, col, format!("'{nombre}' is missing arguments")));
        Ok(match nombre {
            "min" => toma()?.min(toma()?),
            "max" => toma()?.max(toma()?),
            "abs" => toma()?.abs(),
            "floor" => toma()?.suelo(),
            "sin" => toma()?.seno(),
            "cos" => toma()?.coseno(),
            "ceil" => toma()?.techo(),
            "clamp" => {
                let (x, lo, hi) = (toma()?, toma()?, toma()?);
                x.max(lo).min(hi)
            }
            "smooth" => {
                let (lo, hi, x) = (toma()?, toma()?, toma()?);
                x.suave(cte(&lo)?, cte(&hi)?)
            }
            "mix" => {
                let (x, y, t) = (toma()?, toma()?, toma()?);
                x.clone() + (y - x) * t
            }
            // La condición vale 1 o 0: elegir es mezclar.
            "if" => {
                let (si, x, y) = (toma()?, toma()?, toma()?);
                y.clone() + (x - y) * si
            }
            otra => unreachable!("'{otra}' is in the vocabulary, but `funcion` cannot compute it"),
        })
    }

    fn punto(&self, c: &mut Cur) -> R<Punto> {
        let x = self.expr(c)?;
        c.exige_sim(",")?;
        Ok((x, self.expr(c)?))
    }

    /// Un color: `#9ed6bd`, o `mix(#9ed6bd, #bdeed6, realce)`.
    fn color(&self, c: &mut Cur) -> R<Color> {
        match c.mira() {
            Some(F::Color(k)) => {
                c.i += 1;
                Ok(color(k[0], k[1], k[2]))
            }
            Some(F::Id(n)) if self.entornos.iter().any(|e| e.colores.contains_key(n)) || self.colores.contains_key(n) => {
                c.i += 1;
                if !self.entornos.iter().any(|e| e.colores.contains_key(n)) {
                    self.vigilar(n);
                }
                Ok(self.entornos.iter().rev().find_map(|e| e.colores.get(n)).unwrap_or_else(|| &self.colores[n]).clone())
            }
            Some(F::Id(m)) if m == "mix" => {
                c.i += 1;
                c.exige_sim("(")?;
                let a = self.color(c)?;
                c.exige_sim(",")?;
                let b = self.color(c)?;
                c.exige_sim(",")?;
                let t = self.expr(c)?;
                c.exige_sim(")")?;
                let [a0, a1, a2] = a;
                let [b0, b1, b2] = b;
                Ok([a0.clone() + (b0 - a0) * t.clone(), a1.clone() + (b1 - a1) * t.clone(), a2.clone() + (b2 - a2) * t])
            }
            _ => c.fallo("expected a colour here: #151616, the name of one, or mix(#a, #b, how much)"),
        }
    }

    // ── el bloque de un nodo, visto como propiedades ────────────

    fn propiedades<'n>(&self, n: &'n Nodo, validas: &[&str]) -> R<HashMap<&'n str, Cur<'n>>> {
        let mut m = HashMap::new();
        for e in n.cuerpo.as_deref().unwrap_or(&[]) {
            if let Entrada::Prop { nombre, valor, linea, col } = e {
                if !validas.contains(&nombre.as_str()) {
                    let v: Vec<String> = validas.iter().map(|s| s.to_string()).collect();
                    let pista = parecido(nombre, v.iter()).map_or(String::new(), |p| format!(" Did you mean '{p}'?"));
                    return Err(Fallo::en(*linea, *col, format!("'{nombre}' does not exist here.{pista} Valid ones: {}", validas.join(", "))));
                }
                m.insert(nombre.as_str(), Cur::de(valor, *linea, *col));
            }
        }
        Ok(m)
    }

    // ── formas ──────────────────────────────────────────────────

    /// `ellipse orb { at: …; radius: … }` → la forma, su nombre, y lo que lleve de pintura.
    fn forma(&mut self, n: &Nodo, desde: usize) -> R<FormaLeida> {
        let mut c = Cur::de(&n.cabeza[desde..], n.linea, n.col);
        let clase = c.id("a shape: ellipse, box, arc or line")?;
        let nombre = if c.acabo() { None } else { Some(c.id("a name for the shape")?) };
        c.nada_mas()?;
        let comunes = voz::propiedades("shape");
        let en_hueco = std::mem::take(&mut self.en_hueco);
        let propias: &[&str] = match clase.as_str() {
            "ellipse" | "box" | "arc" | "line" | "path" => voz::propiedades(&clase),
            otra => return Err(Fallo::en(n.linea, n.col, format!("I don\'t know the shape '{otra}': there are ellipse, box, arc, line and path"))),
        };
        let validas: Vec<&str> = propias.iter().chain(comunes.iter()).copied().collect();
        let mut p = self.propiedades(n, &validas)?;
        let falta = |que: &str| Fallo::en(n.linea, n.col, format!("this '{clase}' is missing '{que}'"));
        let mut una = |o: &Obra, k: &str| -> R<Option<Expr>> {
            match p.get_mut(k) {
                Some(c) => {
                    let e = o.expr(c)?;
                    c.nada_mas()?;
                    Ok(Some(e))
                }
                None => Ok(None),
            }
        };
        let (rotate, stroke, opacity, blend, active) = (una(self, "rotate")?, una(self, "stroke")?, una(self, "opacity")?, una(self, "blend")?, una(self, "active")?);
        let (radius, corner, span, width) = (una(self, "radius")?, una(self, "corner")?, una(self, "span")?, una(self, "width")?);
        let mut dos = |o: &Obra, k: &str| -> R<Option<Punto>> {
            match p.get_mut(k) {
                Some(c) => {
                    let e = o.punto(c)?;
                    c.nada_mas()?;
                    Ok(Some(e))
                }
                None => Ok(None),
            }
        };
        let (at, from, size, scale, to) = (dos(self, "at")?, dos(self, "from")?, dos(self, "size")?, dos(self, "scale")?, dos(self, "to")?);
        let color = match p.get_mut("color") {
            Some(c) => Some(self.color(c)?),
            None => None,
        };
        let cursor = match p.get_mut("cursor") {
            Some(c) => leer_cursor(c)?,
            None => Cursor::Normal,
        };
        let mut tam: Option<(Expr, Expr)> = None;
        let mut forma = match clase.as_str() {
            "ellipse" => {
                let radio = radius.ok_or_else(|| falta("radius"))?;
                let escala = scale.unwrap_or((1.0.into(), 1.0.into()));
                tam = Some((radio.clone() * 2.0 * escala.0.clone(), radio.clone() * 2.0 * escala.1.clone()));
                // En un hueco, pegada a su esquina.
                let centro = match at {
                    Some(a) => a,
                    None if en_hueco => (radio.clone() * escala.0.clone(), radio.clone() * escala.1.clone()),
                    None => return Err(falta("at")),
                };
                Forma::Elipse { centro, radio, escala }
            }
            "box" => {
                let (w, h) = size.ok_or_else(|| falta("size"))?;
                tam = Some((w.clone(), h.clone()));
                let centro = match (at, from) {
                    (None, None) if en_hueco => (w.clone() * 0.5, h.clone() * 0.5),
                    (Some(a), None) => a,
                    (None, Some((x, y))) => (x + w.clone() * 0.5, y + h.clone() * 0.5),
                    _ => return Err(Fallo::en(n.linea, n.col, "a box is placed with 'at' (its centre) or with 'from' (its corner), one of the two")),
                };
                Forma::Caja { centro, mitad: (w * 0.5, h * 0.5), radio: corner.unwrap_or(Expr::K(0.0)) }
            }
            // `path { move 0, 0; line 20, 10; curve 40, 0 via 32, 10; close }`
            "path" => {
                let base = at.unwrap_or((Expr::K(0.0), Expr::K(0.0)));
                let mueve = |o: &Obra, c: &mut Cur| -> R<Punto> {
                    let p = o.punto(c)?;
                    Ok((base.0.clone() + p.0, base.1.clone() + p.1))
                };
                let (mut origen, mut pasos, mut cerrado) = (None, Vec::new(), false);
                for e in n.cuerpo.as_deref().unwrap_or(&[]) {
                    let Entrada::Nodo(x) = e else { continue };
                    let mut c = Cur::de(&x.cabeza, x.linea, x.col);
                    match c.una_de(voz::DE_CAMINO, "a step of a path")?.as_str() {
                        "move" => {
                            if origen.is_some() || !pasos.is_empty() {
                                return Err(Fallo::en(x.linea, x.col, "a path starts at one place: `move` goes first, and only once. For several strokes, several `path`"));
                            }
                            origen = Some(mueve(self, &mut c)?);
                        }
                        "line" => pasos.push(Paso::Linea(mueve(self, &mut c)?)),
                        "curve" => {
                            let a = mueve(self, &mut c)?;
                            c.exige_palabra("via")?;
                            pasos.push(Paso::Curva { via: mueve(self, &mut c)?, a });
                        }
                        _ => cerrado = true,
                    }
                    c.nada_mas()?;
                }
                if pasos.is_empty() {
                    return Err(Fallo::en(n.linea, n.col, "a path goes somewhere: it needs at least one `line` or one `curve`"));
                }
                if pasos.len() + 1 > crate::formas::MAX_PUNTOS {
                    return Err(Fallo::en(n.linea, n.col, format!("a path has at most {} steps", crate::formas::MAX_PUNTOS - 1)));
                }
                tam = size;
                Forma::Camino { origen: origen.unwrap_or(base), pasos, cerrado }
            }
            "arc" => Forma::Arco { centro: at.ok_or_else(|| falta("at"))?, radio: radius.ok_or_else(|| falta("radius"))?, apertura: span.ok_or_else(|| falta("span"))? * 0.5, grosor: width.ok_or_else(|| falta("width"))? },
            _ => Forma::Segmento { de: from.ok_or_else(|| falta("from"))?, a: to.ok_or_else(|| falta("to"))?, grosor: width.ok_or_else(|| falta("width"))? },
        };
        if let Some(a) = rotate {
            forma = forma.girada(a);
        }
        if let Some(w) = stroke {
            forma = forma.trazo(w);
        }
        // Una forma con nombre puede ser una zona, con las transformaciones bajo
        // las que se pinta. Si lo es o no se decide al final: ver `zonas_de_verdad`.
        if let Some(nombre) = &nombre {
            let nombre = &self.declarar(nombre);
            self.candidatas.push(Candidata { nombre: nombre.clone(), forma: forma.clone(), activa: active, visible: None, bajo: self.bajo.clone(), forzada: false, cursor });
        }
        Ok(FormaLeida { forma, color, opacidad: opacity, fusion: blend, tam })
    }

    // ── lo que se pinta ─────────────────────────────────────────

    /// Un grupo: sus propiedades (transformación, opacidad) y sus hijos en orden.
    fn grupo(&mut self, entradas: impl Iterator<Item = &'a Entrada>) {
        let mut recortes = 0;
        for e in entradas {
            let Entrada::Nodo(n) = e else { continue };
            // Un fallo no para la lectura: se apunta y se sigue, para decirlos todos.
            if let Err(f) = self.sentencia(n, &mut recortes) {
                self.anotar(f);
            }
        }
        // Un recorte vale hasta el final de su grupo.
        for _ in 0..recortes {
            self.e.pintar(Instr::Recorte(None));
        }
    }

    fn sentencia(&mut self, n: &'a Nodo, recortes: &mut usize) -> R<()> {
        {
            let mut c = Cur::de(&n.cabeza, n.linea, n.col);
            let palabra = c.id("a declaration")?;
            if !voz::SENTENCIAS.contains(&palabra.as_str()) && !self.componentes.contains_key(&palabra) {
                let validas: Vec<String> = voz::SENTENCIAS.iter().map(|s| s.to_string()).chain(self.componentes.keys().cloned()).collect();
                let pista = parecido(&palabra, validas.iter()).map_or(String::new(), |p| format!(" Did you mean '{p}'?"));
                return Err(Fallo::en(n.linea, n.col, format!("I don\'t know what '{palabra}' is.{pista}")));
            }
            self.clase_actual = palabra.clone();
            match palabra.as_str() {
                "surface" => {
                    // Una superficie con nombre es otra ventana: empieza limpia. Un
                    // `clip` suelto de la escena llegaba hasta el final de su grupo
                    // —o sea, hasta el final del fichero— y recortaba también lo que
                    // dibujaba la otra ventana, que vive lejos en el mismo plano: no
                    // se veía nada, y nada lo decía. Los recortes abiertos acaban aquí.
                    if n.cuerpo.as_deref().unwrap_or(&[]).iter().any(|e| matches!(e, Entrada::Nodo(_))) {
                        for _ in 0..std::mem::take(recortes) {
                            self.e.pintar(Instr::Recorte(None));
                        }
                    }
                    self.superficie(n)?
                }
                "model" => self.modelo(n, &mut c)?,
                "service" => self.servicio(n, &mut c)?,
                "permissions" => {
                    // permissions { run: "date", "notify-send";  services: "audio", "apps" }
                    let mut p = self.propiedades(n, voz::propiedades("permissions"))?;
                    for (clave, destino) in [("run", 0), ("services", 1)] {
                        let Some(c) = p.get_mut(clave) else { continue };
                        let mut lista = vec![c.cadena()?];
                        while c.sim(",") {
                            lista.push(c.cadena()?);
                        }
                        c.nada_mas()?;
                        let de_quien = match n.linea / super::POR_FICHERO {
                            0 => &mut self.e.permisos,
                            k => self.permisos_de.entry(k).or_default(),
                        };
                        if destino == 0 { de_quien.ordenes = lista } else { de_quien.servicios = lista }
                    }
                }
                "spring" => {
                    let nombre = c.id("a name for the spring")?;
                    if n.linea >= super::POR_FICHERO {
                        self.de_biblioteca.insert(nombre.clone());
                    }
                    c.exige_sim("=")?;
                    let rigidez = c.num()?;
                    c.exige_sim(",")?;
                    self.muelles.insert(nombre, Muelle { rigidez, freno: c.num()? });
                }
                "prop" | "pose" => {
                    let nombre = self.declarar(&c.id("a name for the property")?);
                    c.exige_sim("=")?;
                    let v = c.num()?;
                    let muelle = if c.sim("~") { self.muelle(&mut c)? } else if palabra == "pose" { Muelle::POSE } else { Muelle::VIVO };
                    c.nada_mas()?;
                    let p = match self.props.get(&nombre) {
                        // Ya declarada: es la misma. Pasa cuando un `repeat` con declaraciones
                        // cae dentro de algo que se repite, como una superficie por monitor.
                        Some(ya) => *ya,
                        None => {
                            let p = self.e.prop_con(fijo(&nombre), v, muelle);
                            if palabra == "pose" {
                                self.e.pose.push(p);
                            }
                            p
                        }
                    };
                    self.props.insert(nombre, p);
                }
                "fact" => {
                    // fact open = false · fact count: number = 0 · fact mode: low | normal | critical = normal
                    let nombre = self.declarar(&c.id("a name for the fact")?);
                    let declarado = if c.sim(":") { Some(self.tipo_de_hecho(&mut c)?) } else { None };
                    c.exige_sim("=")?;
                    let (v, tipo) = match declarado {
                        Some(Some(TipoDeHecho::Enum(nombres))) => {
                            let cual = c.una_de(&nombres.iter().map(String::as_str).collect::<Vec<_>>(), "a value of this fact")?;
                            (nombres.iter().position(|x| *x == cual).unwrap() as f32, Some(TipoDeHecho::Enum(nombres)))
                        }
                        Some(Some(TipoDeHecho::Bool)) => (if c.palabra("true") { 1.0 } else if c.palabra("false") { 0.0 } else { return c.fallo("a `bool` fact is true or false") }, Some(TipoDeHecho::Bool)),
                        Some(None) => (if c.sim("-") { -c.num()? } else { c.num()? }, None),
                        // Sin tipo, lo dice lo que valga: `true` y `false` hacen un sí o no; un número, un número.
                        None => if c.palabra("true") { (1.0, Some(TipoDeHecho::Bool)) } else if c.palabra("false") { (0.0, Some(TipoDeHecho::Bool)) } else { (if c.sim("-") { -c.num()? } else { c.num()? }, None) },
                    };
                    c.nada_mas()?;
                    let h = match self.hechos.get(&nombre) {
                        Some(ya) => *ya,
                        None => {
                            if let Some(t) = tipo {
                                self.e.tipos.push((nombre.clone(), t));
                            }
                            self.e.hecho(fijo(&nombre), v)
                        }
                    };
                    self.hechos.insert(nombre, h);
                }
                "event" => {
                    let nombre = self.declarar(&c.id("a name for the event")?);
                    let sale = c.sim("->");
                    c.nada_mas()?;
                    let s = match self.sucesos.get(&nombre) {
                        Some(ya) => *ya,
                        None if sale => self.e.suceso_que_sale(fijo(&nombre)),
                        None => self.e.suceso(fijo(&nombre)),
                    };
                    self.sucesos.insert(nombre, s);
                }
                "text" if matches!(c.f.get(2).map(|x| &x.f), Some(F::Sim("="))) => {
                    let nombre = self.declarar(&c.id("a name for the text")?);
                    c.exige_sim("=")?;
                    let valor = c.cadena()?;
                    let t = match self.textos.get(&nombre) {
                        Some(ya) => *ya,
                        None => self.e.texto_vivo(fijo(&nombre), &valor),
                    };
                    self.textos.insert(nombre, t);
                }
                "image" if matches!(c.f.get(2).map(|x| &x.f), Some(F::Sim("="))) => {
                    let nombre = self.declarar(&c.id("a name for the image")?);
                    c.exige_sim("=")?;
                    let fuente = if c.palabra("icon") {
                        Fuente::Icono(c.cadena()?)
                    } else if c.palabra("file") {
                        // Relativa al fichero que la escribe, no a desde dónde se lance: así una
                        // biblioteca lleva sus imágenes consigo.
                        let escrita = std::path::PathBuf::from(c.cadena()?);
                        let carpeta = self.carpetas.get(n.linea / super::POR_FICHERO);
                        Fuente::Ruta(match carpeta { Some(k) if escrita.is_relative() => k.join(escrita), _ => escrita })
                    } else if c.palabra("from") {
                        // La que diga un texto vivo: así elige la lógica una imagen.
                        let t = self.global(&c.id("the name of a text")?);
                        match self.textos.get(&t) {
                            Some(t) => Fuente::Viva(*t),
                            None => return self.desconocido(&c, "no text", &t, self.textos.keys().collect()),
                        }
                    } else {
                        return c.fallo("an image is `icon \"name\"`, `file \"path\"` or `from some_text`");
                    };
                    c.exige_sim(",")?;
                    let w = c.num()?;
                    c.exige_sim(",")?;
                    let i = self.e.imagen(fuente, w as u32, c.num()? as u32);
                    self.imagenes.insert(nombre, i);
                }
                "measure" => {
                    let local = c.id("a name for the measure")?;
                    let nombre = self.declarar(&local);
                    let parte = self.interpolar_en(&local);
                    if let Some(e) = self.entornos.last_mut() {
                        e.con_partes.insert(parte);
                    }
                    let (w, h) = self.e.medida(fijo(&nombre));
                    self.props.insert(format!("{nombre}.width"), w);
                    self.props.insert(format!("{nombre}.height"), h);
                    self.medidas.insert(nombre, (w, h));
                }
                "let" => {
                    let nombre = c.id("a name")?;
                    if n.linea >= super::POR_FICHERO {
                        self.de_biblioteca.insert(nombre.clone());
                    }
                    c.exige_sim("=")?;
                    // `let mint = #9ed6bd`: un color con nombre.
                    let es_color = match (c.mira(), c.f.get(c.i + 2).map(|x| &x.f)) {
                        (Some(F::Color(_)), _) => true,
                        (Some(F::Id(m)), Some(F::Color(_))) if m == "mix" => true,
                        (Some(F::Id(m)), Some(F::Id(k))) if m == "mix" && self.colores.contains_key(k) => true,
                        (Some(F::Id(k)), _) if self.colores.contains_key(k) => true,
                        _ => false,
                    };
                    if es_color {
                        let k = self.color(&mut c)?;
                        c.nada_mas()?;
                        match self.entornos.last_mut() {
                            Some(e) => e.colores.insert(nombre, k),
                            None => self.colores.insert(nombre, k),
                        };
                        return Ok(());
                    }
                    let e = self.expr(&mut c)?;
                    c.nada_mas()?;
                    match self.entornos.last_mut() {
                        Some(env) => env.exprs.insert(nombre, e),
                        None => self.lets.insert(nombre, e),
                    };
                }
                "zone" => {
                    // Una forma que no se pinta: solo es sensible.
                    self.forma(n, 1)?;
                    if let Some(c) = self.candidatas.last_mut() {
                        c.forzada = true;
                    }
                }
                "body" => self.cuerpo(n)?,
                "ellipse" | "box" | "arc" | "line" | "path" => {
                    let f = self.forma(n, 0)?;
                    self.ultimo_tam = f.tam;
                    self.e.pintar(Instr::Plano { forma: f.forma, color: f.color.unwrap_or_else(|| color(1.0, 1.0, 1.0)), alfa: f.opacidad.unwrap_or(Expr::K(1.0)) });
                }
                "text" => self.texto(n)?,
                "image" => self.imagen(n)?,
                "input" => self.campo(n)?,
                "clip" => {
                    let margen = if c.palabra("inset") { c.num()? } else { 0.0 };
                    let desde = c.i;
                    let f = self.forma(n, desde)?;
                    self.e.pintar(Instr::Recorte(Some((f.forma, margen))));
                    *recortes += 1;
                }
                "group" => self.grupo_con_propiedades(n, n.cuerpo.as_deref().unwrap_or(&[]))?,
                "popup" => self.emergente(n, &mut c)?,
                "children" => self.hijos_de_fuera(n)?,
                // Dentro de un reparto, con varias cosas, hace de grupo; suelto no es nada.
                "between" if self.en_hueco => self.grupo_con_propiedades(n, n.cuerpo.as_deref().unwrap_or(&[]))?,
                "between" => return Err(Fallo::en(n.linea, n.col, "`between` only works inside a `row` or a `column`: it is what goes between every two children")),
                "component" => self.declarar_componente(n, &mut c)?,
                "repeat" => self.repetir(n, &mut c)?,
                "for" => self.para(n, &mut c)?,
                "row" | "column" => self.reparto(n, &mut c, palabra == "row")?,
                "space" => {
                    let v = self.expr(&mut c)?;
                    self.ultimo_tam = Some((v.clone(), v));
                }
                copia if self.componentes.contains_key(copia) => self.copia_de(n, &mut c)?,
                "layer" => self.capa(n, &mut c)?,
                "on" | "every" => self.reglas.push((n, self.entornos.clone())),
                "blink" | "wave" | "spin" | "follow" | "look" => self.comportamiento(&palabra, &mut c)?,
                "gesture" | "posture" => self.gesto(n, &palabra, &mut c)?,
                // La puerta de arriba solo deja pasar lo que está en el vocabulario: si se llega
                // aquí, es que se apuntó una palabra y nadie la atiende.
                otra => unreachable!("'{otra}' is in the vocabulary, but `sentencia` does not know what to do with it"),
            }
        }
        Ok(())
    }

    /// `n` trae las propiedades; `cuerpo`, los hijos: los del propio grupo, o los
    /// de un componente cuando `n` es una de sus copias.
    fn grupo_con_propiedades(&mut self, n: &Nodo, cuerpo: &'a [Entrada]) -> R<()> {
        let mut p = self.propiedades(n, voz::propiedades("group"))?;
        self.en_hueco = false;
        let tam = match p.get_mut("size") {
            Some(c) => Some(self.punto(c)?),
            None => None,
        };
        let mut t = Transformacion::en((0.0.into(), 0.0.into()));
        let mut transforma = false;
        if let Some(c) = p.get_mut("pivot") {
            t.pivote = self.punto(c)?;
        }
        if let Some(c) = p.get_mut("rotate") {
            t.giro = self.expr(c)?;
            transforma = true;
        }
        if let Some(c) = p.get_mut("scale") {
            let x = self.expr(c)?;
            t.escala = if c.sim(",") { (x, self.expr(c)?) } else { (x.clone(), x) };
            transforma = true;
        }
        if let Some(c) = p.get_mut("move") {
            t.mueve = self.punto(c)?;
            transforma = true;
        }
        let opacidad = match p.get_mut("opacity") {
            Some(c) => Some(self.expr(c)?),
            None => None,
        };
        if transforma {
            self.e.pintar(Instr::Transformar(Some(t.clone())));
            self.bajo.push(t);
        }
        if let Some(o) = &opacidad {
            self.e.pintar(Instr::Opacidad(Some(o.clone())));
        }
        self.grupo(cuerpo.iter());
        if tam.is_some() {
            self.ultimo_tam = tam;
        }
        if opacidad.is_some() {
            self.e.pintar(Instr::Opacidad(None));
        }
        if transforma {
            self.bajo.pop();
            self.e.pintar(Instr::Transformar(None));
        }
        Ok(())
    }

    /// Lo que ocupa un reparto con nombre se sabe al acabar de dibujarlo, pero se
    /// puede querer leer antes (el panel que envuelve a su lista, el `size:` de un
    /// componente): sus medidas se declaran ya, como propiedades, y el reparto las
    /// rellena al final. Dentro de una copia, con el nombre propio de esa copia.
    fn adelantar_medidas(&mut self, entradas: &'a [Entrada]) {
        for e in entradas {
            let Entrada::Nodo(n) = e else { continue };
            let palabra = |k: usize| match n.cabeza.get(k).map(|f| &f.f) { Some(F::Id(p)) => Some(p.as_str()), _ => None };
            match palabra(0) {
                Some("component" | "repeat" | "for") => continue,
                Some("row" | "column") => if let Some(local) = palabra(1) {
                    // Se adelanta para poder leerlo antes de llegar a él, pero el sitio
                    // que se apunta es el suyo: es donde el editor tiene que llevar.
                    MIRANDO.with(|m| m.set((n.linea, n.col)));
                    self.clase_actual = palabra(0).unwrap_or("row").to_owned();
                    let nombre = self.declarar(local);
                    if let Some(e) = self.entornos.last_mut() {
                        e.con_partes.insert(local.to_owned());
                    }
                    for parte in ["width", "height", "count"] {
                        let entero = format!("{nombre}.{parte}");
                        if !self.props.contains_key(&entero) {
                            let p = self.e.prop_con(fijo(&entero), 0.0, Muelle::VIVO);
                            self.props.insert(entero, p);
                        }
                    }
                },
                _ => {}
            }
            self.adelantar_medidas(n.cuerpo.as_deref().unwrap_or(&[]));
        }
    }

    /// Lo que la copia en curso trae para el hueco que nombra este `children`, y el ámbito
    /// de quien lo escribió. Un hueco se pone una vez.
    fn coger_hueco(&mut self, n: &Nodo) -> R<(Vec<&'a Entrada>, Vec<Entorno>)> {
        let cual = match n.cabeza.get(1).map(|f| &f.f) {
            Some(F::Id(p)) => p.clone(),
            None => String::new(),
            Some(_) => return Err(Fallo::en(n.linea, n.col, "after `children` only the slot name can go: `children header`")),
        };
        let Some(copia) = self.hijos_de_copia.last_mut() else {
            return Err(Fallo::en(n.linea, n.col, "`children` only works inside a component: it is where whatever each copy brings goes"));
        };
        let h = copia.huecos.iter_mut().find(|h| h.0 == cual).expect("the slots were noted when the component was declared");
        h.2 = true;
        Ok((h.1.clone(), copia.fuera.clone()))
    }

    /// `children { move: 12, 40 }`, dentro de un componente: aquí va lo que cada copia
    /// traiga en su bloque. Es un grupo, y lo de dentro se lee con los nombres de
    /// quien lo escribió: un componente no ve —ni pisa— lo que le meten.
    fn hijos_de_fuera(&mut self, n: &'a Nodo) -> R<()> {
        let (hijos, fuera) = self.coger_hueco(n)?;
        let mut p = self.propiedades(n, voz::propiedades("children"))?;
        let mueve = match p.get_mut("move") {
            Some(c) => Some(self.punto(c)?),
            None => None,
        };
        if let Some(m) = &mueve {
            let t = Transformacion { mueve: m.clone(), ..Transformacion::en((0.0.into(), 0.0.into())) };
            self.e.pintar(Instr::Transformar(Some(t.clone())));
            self.bajo.push(t);
        }
        // Mientras se leen, el componente no está: sus parámetros no tapan nada de fuera.
        let dentro = std::mem::replace(&mut self.entornos, fuera);
        let pendientes = std::mem::take(&mut self.hijos_de_copia);
        self.grupo(hijos.into_iter());
        self.hijos_de_copia = pendientes;
        self.entornos = dentro;
        if mueve.is_some() {
            self.bajo.pop();
            self.e.pintar(Instr::Transformar(None));
        }
        Ok(())
    }

    /// `popup menu { at: x, y; size: w, h; open: menu_open; … }`: una superficie
    /// hija. Lo de dentro se dibuja con (0, 0) en su esquina, como si fuera otra
    /// escena; en realidad es un trozo de esta, puesto lejos.
    fn emergente(&mut self, n: &'a Nodo, c: &mut Cur) -> R<()> {
        let local = c.id("a name for the popup")?;
        let nombre = self.declarar(&local);
        if !self.entornos.is_empty() || !self.bajo.is_empty() {
            return Err(Fallo::en(n.linea, n.col, "a `popup` goes at the scene level, not inside a group or a component"));
        }
        let mut p = self.propiedades(n, voz::propiedades("popup"))?;
        let falta = |q: &str| Fallo::en(n.linea, n.col, format!("this `popup` is missing '{q}'"));
        let en = self.punto(p.get_mut("at").ok_or_else(|| falta("at"))?)?;
        let tam = self.punto(p.get_mut("size").ok_or_else(|| falta("size"))?)?;
        let abierta = {
            let c = p.get_mut("open").ok_or_else(|| falta("open"))?;
            let h = self.global(&c.id("the fact that opens it")?);
            match self.hechos.get(&h) {
                Some(h) => *h,
                None => return self.desconocido(c, "no fact", &h, self.hechos.keys().collect()),
            }
        };
        // Cada una en su sitio, lejos de la superficie y de las demás.
        self.siguiente_origen += 10000.0;
        let origen = (0.0, self.siguiente_origen);
        let t = Transformacion::en((origen.0.into(), origen.1.into()));
        let t = Transformacion { mueve: (origen.0.into(), origen.1.into()), ..t };
        self.e.pintar(Instr::Transformar(Some(t.clone())));
        self.bajo.push(t);
        self.grupo(n.cuerpo.as_deref().unwrap_or(&[]).iter());
        self.bajo.pop();
        self.e.pintar(Instr::Transformar(None));
        self.e.emergentes.push(Emergente { nombre: fijo(&nombre), abierta, en, tam, origen });
        Ok(())
    }

    /// `body { color: …; shadow: …; ellipse {…}; box {… blend: …} }`
    fn cuerpo(&mut self, n: &Nodo) -> R<()> {
        let mut p = self.propiedades(n, voz::propiedades("body"))?;
        let en_hueco = std::mem::take(&mut self.en_hueco);
        let mut tam = None;
        let sombra = match p.get_mut("shadow") {
            Some(c) => {
                let dx = c.num()?;
                c.exige_sim(",")?;
                let dy = c.num()?;
                c.exige_sim(",")?;
                let difusa = c.num()?;
                c.exige_sim(",")?;
                Some(Sombra { desplazada: (dx, dy), difusa, alfa: c.num()? })
            }
            None => None,
        };
        self.e.pintar(Instr::Grupo { sombra });
        let mut formas = 0;
        for e in n.cuerpo.as_deref().unwrap_or(&[]) {
            if let Entrada::Nodo(h) = e {
                self.en_hueco = en_hueco && formas == 0;
                let f = self.forma(h, 0)?;
                if formas == 0 {
                    tam = f.tam;
                }
                self.e.pintar(Instr::Forma { forma: f.forma, fusion: f.fusion.unwrap_or(Expr::K(0.0)) });
                formas += 1;
            }
        }
        if formas == 0 {
            return Err(Fallo::en(n.linea, n.col, "a 'body' with no shapes paints nothing"));
        }
        let pintura = if let Some(c) = p.get_mut("gradient") {
            // `gradient: radial 20, 20 radius 30 { … }`: desde un centro hacia fuera.
            let radial = c.palabra("radial");
            let de = self.punto(c)?;
            let a = if radial {
                c.exige_palabra("radius")?;
                (self.expr(c)?, Expr::K(0.0))
            } else {
                // `0, 0 to 0, 44` se lee igual que `0, 0, 0, 44`.
                if !c.palabra("to") {
                    c.exige_sim(",")?;
                }
                self.punto(c)?
            };
            // Los colores, separados por comas. Cada uno puede decir dónde cae
            // (`sand 40%`); los que no lo digan se reparten por igual. Dos colores a
            // secas es el degradado de siempre.
            let mut paradas: Vec<(Expr, Color)> = Vec::new();
            while c.sim(",") {
                let col = self.color(c)?;
                let donde = if !c.acabo() && !matches!(c.mira(), Some(F::Sim(","))) { self.expr(c)? } else { Expr::K(-1.0) };
                paradas.push((donde, col));
            }
            c.nada_mas()?;
            if paradas.len() < 2 {
                return Err(Fallo::en(n.linea, n.col, "a gradient needs at least two colours: `gradient: 0, 0 to 0, 44, mint, sand 40%, coal`"));
            }
            if paradas.len() > 8 {
                return Err(Fallo::en(n.linea, n.col, "a gradient holds at most 8 colours"));
            }
            let ultimo = paradas.len() - 1;
            for (k, parada) in paradas.iter_mut().enumerate() {
                if matches!(parada.0, Expr::K(v) if v < 0.0) {
                    parada.0 = Expr::K(k as f32 / ultimo as f32);
                }
            }
            Pintura::Degradado { radial, de, a, paradas }
        } else if let Some(c) = p.get_mut("color") {
            Pintura::Color(self.color(c)?)
        } else {
            return Err(Fallo::en(n.linea, n.col, "this 'body' is missing 'color' or 'gradient'"));
        };
        let luz = match p.get_mut("light") {
            Some(c) => {
                let cantidad = c.num()?;
                c.exige_sim(",")?;
                let desde_y = self.expr(c)?;
                c.exige_sim(",")?;
                Some(Luz { cantidad, desde_y, alto: c.num()? })
            }
            None => None,
        };
        let borde = match p.get_mut("border") {
            Some(c) => {
                let w = self.expr(c)?;
                c.exige_sim(",")?;
                Some((w, self.color(c)?))
            }
            None => None,
        };
        let filo = match p.get_mut("rim") {
            Some(c) => c.num()?,
            None => 0.0,
        };
        let alfa = match p.get_mut("opacity") {
            Some(c) => self.expr(c)?,
            None => Expr::K(1.0),
        };
        self.e.pintar(Instr::Relleno { pintura, alfa, filo, luz, borde });
        self.ultimo_tam = tam;
        Ok(())
    }

    // ── textos con huecos ───────────────────────────────────────

    /// `"Descartar"` es un texto fijo; `"{r.title} · {volume * 100, 1} %"`, una plantilla.
    fn contenido_de(&self, s: &str, donde: &Ficha) -> R<Contenido> {
        if !s.contains('{') && !s.contains('}') {
            return Ok(Contenido::Fijo(s.to_owned()));
        }
        let letras: Vec<char> = s.chars().collect();
        let mut i = 0;
        let trozos = self.trozos(&letras, &mut i, false, donde)?;
        Ok(match trozos.as_slice() {
            [] => Contenido::Fijo(String::new()),
            [Trozo::Fijo(t)] => Contenido::Fijo(t.clone()),
            _ => Contenido::Plantilla(trozos),
        })
    }

    /// Hasta el final, o hasta la `}` que cierra un tramo opcional.
    fn trozos(&self, s: &[char], i: &mut usize, dentro: bool, donde: &Ficha) -> R<Vec<Trozo>> {
        // Dónde señalar si algo falla: la comilla, más lo andado (si no hay saltos de línea, exacto).
        let aqui = |i: usize| (donde.linea, donde.col + 1 + i);
        let mut fuera = Vec::new();
        let mut fijo = String::new();
        while *i < s.len() {
            match (s[*i], s.get(*i + 1)) {
                // Dos seguidas son una de verdad.
                ('{', Some('{')) | ('}', Some('}')) => {
                    fijo.push(s[*i]);
                    *i += 2;
                }
                ('}', _) if dentro => {
                    *i += 1;
                    if !fijo.is_empty() { fuera.push(Trozo::Fijo(fijo)) }
                    return Ok(fuera);
                }
                ('}', _) => {
                    let (l, c) = aqui(*i);
                    return Err(Fallo::en(l, c, "this `}` closes nothing. If it is a real brace, write it twice: `}}`"));
                }
                ('{', Some('?')) => {
                    if !fijo.is_empty() { fuera.push(Trozo::Fijo(std::mem::take(&mut fijo))) }
                    let abre = *i;
                    *i += 2;
                    let opcional = self.trozos(s, i, true, donde)?;
                    if s.get(*i - 1) != Some(&'}') || *i > s.len() {
                        let (l, c) = aqui(abre);
                        return Err(Fallo::en(l, c, "this `{? …}` part is missing its `}`"));
                    }
                    fuera.push(Trozo::Opcional(opcional));
                }
                ('{', _) => {
                    if !fijo.is_empty() { fuera.push(Trozo::Fijo(std::mem::take(&mut fijo))) }
                    let abre = *i;
                    let Some(cierra) = s[abre..].iter().position(|c| *c == '}').map(|k| abre + k) else {
                        let (l, c) = aqui(abre);
                        return Err(Fallo::en(l, c, "this hole is missing its `}`. If it is a real brace, write it twice: `{{`"));
                    };
                    let fuente: String = s[abre + 1..cierra].iter().collect();
                    fuera.push(self.hueco(&fuente, aqui(abre + 1))?);
                    *i = cierra + 1;
                }
                (c, _) => {
                    fijo.push(c);
                    *i += 1;
                }
            }
        }
        if dentro {
            // Se acabó el texto sin cerrar el tramo: lo dirá quien lo abrió.
            *i = s.len() + 1;
        }
        if !fijo.is_empty() { fuera.push(Trozo::Fijo(fijo)) }
        Ok(fuera)
    }

    /// Lo de dentro de un hueco: un texto vivo (`r.title`, `upper(r.app)`) o una
    /// expresión con sus decimales (`volume * 100, 1`).
    fn hueco(&self, fuente: &str, (linea, col): (usize, usize)) -> R<Trozo> {
        let mut fichas = super::fichas::trocear(fuente).map_err(|f| Fallo::en(linea, col + f.col.saturating_sub(1), f.mensaje))?;
        // El final de línea que pone el troceador aquí no significa nada.
        fichas.retain(|f| !matches!(f.f, F::Linea));
        for f in &mut fichas {
            (f.linea, f.col) = (linea, col + f.col.saturating_sub(1));
        }
        let mut c = Cur::de(&fichas, linea, col);
        if fichas.is_empty() {
            return c.fallo("an empty hole: inside goes the name of a text or an expression");
        }
        let es_texto = |o: &Self, n: &str| o.textos.get(&o.global(n)).copied();
        // upper(nombre) · lower(nombre)
        if let (Some(F::Id(f)), Some(F::Sim("("))) = (c.mira(), fichas.get(1).map(|x| &x.f)) {
            let letras = voz::DE_TEXTO.contains(&f.as_str()).then(|| match f.as_str() {
                "upper" => Letras::Mayusculas,
                "lower" => Letras::Minusculas,
                otra => unreachable!("'{otra}' is in the vocabulary, but a hole cannot apply it"),
            });
            if let Some(letras) = letras {
                c.i += 2;
                let nombre = c.id("the name of a text")?;
                // Primero lo de dentro: un parámetro de texto gana a un texto de la escena que se llame igual.
                if let Some(e) = self.entornos.iter().rev().find(|e| e.cadenas.contains_key(&nombre)) {
                    if e.contenidos.contains_key(&nombre) {
                        return c.fallo(format!("'{nombre}' arrived with holes, and `{f}` only knows whole texts: put the `{f}` inside those holes, where it was written"));
                    }
                    c.exige_sim(")")?;
                    c.nada_mas()?;
                    let fijo = &e.cadenas[&nombre];
                    return Ok(if fijo.is_empty() { Trozo::Vacio } else { Trozo::Fijo(if letras == Letras::Mayusculas { fijo.to_uppercase() } else { fijo.to_lowercase() }) });
                }
                let Some(t) = es_texto(self, &nombre) else {
                    let g = self.global(&nombre);
                    if self.hechos.contains_key(&g) || self.props.contains_key(&g) {
                        return c.fallo(format!("'{nombre}' is a number, and `{f}` is for texts"));
                    }
                    return self.desconocido(&c, "no text", &nombre, self.textos.keys().collect());
                };
                c.exige_sim(")")?;
                c.nada_mas()?;
                return Ok(Trozo::Vivo(t, letras));
            }
        }
        if let (Some(F::Id(n)), 1) = (c.mira(), fichas.len()) {
            // Un parámetro de texto del componente donde está esta cadena: lo que se le pasó.
            if let Some(e) = self.entornos.iter().rev().find(|e| e.cadenas.contains_key(n)) {
                return Ok(match e.contenidos.get(n) {
                    Some(Contenido::Plantilla(trozos)) => Trozo::Tramo(trozos.clone()),
                    _ if e.cadenas[n].is_empty() => Trozo::Vacio,
                    _ => Trozo::Fijo(e.cadenas[n].clone()),
                });
            }
            if let Some(t) = es_texto(self, n) {
                return Ok(Trozo::Vivo(t, Letras::Igual));
            }
            // Un hecho enumerado se enseña por su nombre, no por su número.
            let g = self.global(n);
            if let (Some(h), Some((_, TipoDeHecho::Enum(nombres)))) = (self.hechos.get(&g), self.e.tipos.iter().find(|(x, _)| *x == g)) {
                return Ok(Trozo::Nombre(h.e(), nombres.clone()));
            }
        }
        let e = self.expr(&mut c)?;
        // `{secs, time}`: unos segundos, como los escribe un reloj.
        if c.sim(",") {
            if c.palabra("time") {
                c.nada_mas()?;
                return Ok(Trozo::Duracion(e));
            }
            let decimales = c.num()? as u8;
            c.nada_mas()?;
            return Ok(Trozo::Numero(e, decimales));
        }
        c.nada_mas()?;
        Ok(Trozo::Numero(e, 0))
    }

    /// `text notice.title { at: …; size: 20 }` o `text "Descartar" { … }`
    fn texto(&mut self, n: &Nodo) -> R<()> {
        let mut c = Cur::de(&n.cabeza[1..], n.linea, n.col);
        let contenido = match c.mira() {
            Some(F::Cadena(s)) => self.contenido_de(s, &c.f[c.i])?,
            // `text number(volume * 100, 0, " %")`: un número que sale de una expresión.
            Some(F::Id(n)) if n == "number" && matches!(c.f.get(c.i + 1).map(|x| &x.f), Some(F::Sim("("))) => {
                c.i += 2;
                let e = self.expr(&mut c)?;
                let decimales = if c.sim(",") { c.num()? as u8 } else { 0 };
                let detras = if c.sim(",") { c.cadena()? } else { String::new() };
                c.exige_sim(")")?;
                Contenido::Numero(e, decimales, detras)
            }
            // Un parámetro de componente que vale un texto entre comillas.
            Some(F::Id(nombre)) if self.entornos.iter().any(|e| e.contenidos.contains_key(nombre)) => {
                self.entornos.iter().rev().find_map(|e| e.contenidos.get(nombre)).unwrap().clone()
            }
            Some(F::Id(nombre)) if self.entornos.iter().any(|e| e.cadenas.contains_key(nombre)) => {
                Contenido::Fijo(self.entornos.iter().rev().find_map(|e| e.cadenas.get(nombre)).unwrap().clone())
            }
            Some(F::Id(nombre)) => match self.textos.get(&{ c.nombre_aqui(); self.global(nombre) }) {
                Some(t) => Contenido::Vivo(*t),
                None => {
                    c.i += 1;
                    return self.desconocido(&c, "no text", nombre, self.textos.keys().collect());
                }
            },
            _ => return c.fallo("a text is `text \"literal\" { … }` or `text name { … }`"),
        };
        let mut p = self.propiedades(n, voz::propiedades("text"))?;
        let en_hueco = std::mem::take(&mut self.en_hueco);
        let en = match p.get_mut("at") {
            Some(c) => self.punto(c)?,
            None if en_hueco => (0.0.into(), 0.0.into()),
            None => return Err(Fallo::en(n.linea, n.col, "this text is missing 'at'")),
        };
        let mut estilo = Estilo::de(14.0, color(1.0, 1.0, 1.0));
        if let Some(c) = p.get_mut("size") {
            estilo.px = c.num()?;
        }
        if let Some(c) = p.get_mut("weight") {
            estilo.peso = c.num()? as u16;
        }
        if let Some(c) = p.get_mut("line_height") {
            estilo.interlinea = c.num()?;
        }
        if let Some(c) = p.get_mut("lines") {
            estilo.max_lineas = Some(c.num()? as usize);
        }
        if let Some(c) = p.get_mut("family") {
            estilo.familia = Some(fijo(&c.cadena()?));
        }
        if let Some(c) = p.get_mut("color") {
            estilo.color = self.color(c)?;
        }
        if let Some(c) = p.get_mut("align") {
            estilo.alineado = match c.una_de(voz::ALINEADOS_DE_TEXTO, "the alignment of a text")?.as_str() {
                "left" => Alineado::Izquierda,
                "center" => Alineado::Centro,
                "right" => Alineado::Derecha,
                _ => unreachable!(),
            };
        }
        let mut ancla = (0.0, 0.0);
        if let Some(c) = p.get_mut("anchor") {
            // `anchor: center` · `anchor: right center` · `anchor: left top`
            let mut ejes = [None, None];
            while !c.acabo() {
                let palabra = c.id("left, center, right, top o bottom")?;
                match palabra.as_str() {
                    "left" => ejes[0] = Some(0.0),
                    "right" => ejes[0] = Some(1.0),
                    "top" => ejes[1] = Some(0.0),
                    "bottom" => ejes[1] = Some(1.0),
                    "center" => {
                        let k = if ejes[0].is_none() { 0 } else { 1 };
                        ejes[k] = Some(0.5);
                    }
                    _ => return c.fallo("an anchor is left, center or right, and top, center or bottom"),
                }
            }
            ancla = (ejes[0].unwrap_or(0.5), ejes[1].unwrap_or(if ejes[0] == Some(0.5) { 0.5 } else { 0.0 }));
        }
        let ancho = match p.get_mut("width") {
            Some(c) => Some(self.expr(c)?),
            None => None,
        };
        let alfa = match p.get_mut("opacity") {
            Some(c) => self.expr(c)?,
            None => Expr::K(1.0),
        };
        let mide = match p.get_mut("measure") {
            Some(c) => {
                let nombre = self.global(&c.id("the name of a measure")?);
                match self.medidas.get(&nombre) {
                    Some(m) => Some(*m),
                    None => return self.desconocido(c, "ninguna medida", &nombre, self.medidas.keys().collect()),
                }
            }
            // Dentro de un reparto, quien reparte necesita saber cuánto ocupa.
            None => self.medida_impuesta.take(),
        };
        if let Some((w, h)) = mide {
            self.ultimo_tam = Some((ancho.clone().unwrap_or(w.e()), h.e()));
        }
        self.e.pintar(Instr::Texto { contenido, en, ancla, ancho, estilo, alfa, mide });
        Ok(())
    }

    /// `input query { at: x, y; width: 300; size: 16; placeholder: "Buscar…" }`: un
    /// campo donde escribir, que edita el texto vivo de ese nombre.
    fn campo(&mut self, n: &Nodo) -> R<()> {
        let mut c = Cur::de(&n.cabeza[1..], n.linea, n.col);
        let local = c.id("the name of the text it edits")?;
        let nombre = self.global(&local);
        let Some(texto) = self.textos.get(&nombre).copied() else {
            return self.desconocido(&c, "no text", &nombre, self.textos.keys().collect());
        };
        let mut p = self.propiedades(n, voz::propiedades("input"))?;
        let en_hueco = std::mem::take(&mut self.en_hueco);
        let en = match p.get_mut("at") {
            Some(c) => self.punto(c)?,
            None if en_hueco => (0.0.into(), 0.0.into()),
            None => return Err(Fallo::en(n.linea, n.col, "this input is missing 'at'")),
        };
        let ancho = match p.get_mut("width") {
            Some(c) => self.expr(c)?,
            None => return Err(Fallo::en(n.linea, n.col, "this input is missing 'width'")),
        };
        let mut estilo = Estilo::de(15.0, color(1.0, 1.0, 1.0));
        if let Some(c) = p.get_mut("size") {
            estilo.px = c.num()?;
        }
        if let Some(c) = p.get_mut("weight") {
            estilo.peso = c.num()? as u16;
        }
        if let Some(c) = p.get_mut("family") {
            estilo.familia = Some(fijo(&c.cadena()?));
        }
        if let Some(c) = p.get_mut("color") {
            estilo.color = self.color(c)?;
        }
        let marcador = match p.get_mut("placeholder") {
            Some(c) => c.cadena()?,
            None => String::new(),
        };
        let seleccion = match p.get_mut("selection") {
            Some(c) => self.color(c)?,
            None => color(0.25, 0.42, 0.62),
        };
        let alfa = match p.get_mut("opacity") {
            Some(c) => self.expr(c)?,
            None => Expr::K(1.0),
        };
        let alto = estilo.px * estilo.interlinea;
        // Su zona: pulsarlo lo enfoca, y encima el cursor es el de escribir.
        // Se llama como el texto: `on drop query`, `on enter query`.
        let zona = self.declarar(&local);
        self.candidatas.push(Candidata {
            nombre: zona.clone(),
            forma: Forma::Caja { centro: (en.0.clone() + ancho.clone() * 0.5, en.1.clone() + alto * 0.5), mitad: (ancho.clone() * 0.5, (alto * 0.5 + 3.0).into()), radio: 0.0.into() },
            activa: None, visible: None, bajo: self.bajo.clone(), forzada: true, cursor: Cursor::Texto,
        });
        self.ultimo_tam = Some((ancho.clone(), alto.into()));
        self.e.pintar(Instr::Campo { texto, zona: fijo(&zona), en, ancho, estilo, alfa, marcador, seleccion });
        Ok(())
    }

    fn imagen(&mut self, n: &Nodo) -> R<()> {
        let mut c = Cur::de(&n.cabeza[1..], n.linea, n.col);
        let nombre = self.global(&c.id("the name of an image")?);
        let Some(imagen) = self.imagenes.get(&nombre).copied() else {
            return self.desconocido(&c, "ninguna imagen", &nombre, self.imagenes.keys().collect());
        };
        let mut p = self.propiedades(n, voz::propiedades("image"))?;
        let en_hueco = std::mem::take(&mut self.en_hueco);
        let falta = |q: &str| Fallo::en(n.linea, n.col, format!("this image is missing '{q}'"));
        let (x, y) = match p.get_mut("at") {
            Some(c) => self.punto(c)?,
            None if en_hueco => (0.0.into(), 0.0.into()),
            None => return Err(falta("at")),
        };
        let (w, h) = self.punto(p.get_mut("size").ok_or_else(|| falta("size"))?)?;
        self.ultimo_tam = Some((w.clone(), h.clone()));
        let alfa = match p.get_mut("opacity") {
            Some(c) => self.expr(c)?,
            None => Expr::K(1.0),
        };
        let tinte = match p.get_mut("tint") {
            Some(c) => Some(self.color(c)?),
            None => None,
        };
        self.e.pintar(Instr::Imagen { imagen, destino: (x, y, w, h), alfa, tinte });
        Ok(())
    }

    /// `surface { … }` es la ventana de la escena; `surface bar { …; …dibujo… }`, una de
    /// varias, cada una con lo suyo dentro. Todas comparten propiedades, hechos y reglas:
    /// por dentro son trozos distintos del mismo plano, como las emergentes.
    fn superficie(&mut self, n: &'a Nodo) -> R<()> {
        let mut c = Cur::de(&n.cabeza[1..], n.linea, n.col);
        let nombre = match c.mira() {
            Some(F::Id(_)) => self.declarar(&c.id("a name for the surface")?),
            _ => String::new(),
        };
        c.nada_mas()?;
        let dibuja = n.cuerpo.as_deref().unwrap_or(&[]).iter().any(|e| matches!(e, Entrada::Nodo(_)));
        if nombre.is_empty() && dibuja {
            return Err(Fallo::en(n.linea, n.col, "a surface that carries what it draws needs a name: `surface bar { … }`. Without a name it is the scene\'s own, and it draws whatever is loose"));
        }
        // Vuelta 2: lo que dibuja, en su trozo del plano.
        if self.vuelta == 2 {
            if !dibuja {
                return Ok(());
            }
            let copias: Vec<(f32, f32, usize, bool)> = self.e.superficies.iter().filter(|s| s.nombre == nombre).map(|s| (s.origen.0, s.origen.1, s.instancia, matches!(s.pantallas, Pantallas::Numero(_)))).collect();
            for (ox, oy, instancia, por_pantalla) in copias {
                let marca = self.reglas.len();
                // Con `screens: each`, cada copia tiene lo suyo: sus propiedades, sus zonas
                // y sus reglas. `$screen` es su número, y `screen.name` el de su monitor.
                if por_pantalla {
                    self.entornos.push(self.ambito_de_pantalla(instancia));
                }
                let t = Transformacion { mueve: (ox.into(), oy.into()), ..Transformacion::en((0.0.into(), 0.0.into())) };
                self.e.pintar(Instr::Transformar(Some(t.clone())));
                self.bajo.push(t);
                self.grupo(n.cuerpo.as_deref().unwrap_or(&[]).iter());
                self.bajo.pop();
                self.e.pintar(Instr::Transformar(None));
                if por_pantalla {
                    self.cerrar_ambito(marca);
                }
            }
            return Ok(());
        }
        if self.e.superficies.iter().any(|s| s.nombre == nombre) {
            return Err(Fallo::en(n.linea, n.col, match nombre.as_str() {
                "" => "the scene already has its surface: the others carry a name (`surface panel { … }`)".to_owned(),
                _ => format!("there is already a surface called '{nombre}'"),
            }));
        }
        let mut p = self.propiedades(n, voz::propiedades("surface"))?;
        // `screens: each [max 4]`: una superficie por monitor, cada una con su estado.
        let mut cuantas = None;
        if let Some(c) = p.get_mut("screens") {
            if c.palabra("each") {
                let tope = if c.palabra("max") { c.num()? as usize } else { 4 };
                if !(1..=8).contains(&tope) {
                    return Err(Fallo::en(n.linea, n.col, "`each` takes between 1 and 8 monitors: each one unfolds when loading"));
                }
                cuantas = Some(tope);
            }
            // Se vuelve a leer más abajo, ya sobre la superficie.
            c.i = 0;
        }
        // La principal es la primera, tenga nombre o no.
        let nueva = Superficie { nombre: nombre.clone(), ..Default::default() };
        if nombre.is_empty() {
            self.e.superficies.insert(0, nueva);
        } else {
            self.siguiente_origen += 10000.0;
            self.e.superficies.push(Superficie { origen: (0.0, self.siguiente_origen), ..nueva });
        }
        let cual = self.e.superficies.iter().position(|s| s.nombre == nombre).unwrap();
        let mut abierta_pendiente = None;
        if let Some(c) = p.get_mut("open") {
            // Una cuenta entera, no solo un hecho: `open: tuck > 0.01` deja que una
            // superficie siga ahí mientras lo que lleva dentro termina de irse.
            abierta_pendiente = Some((&c.f[c.i..], c.pos()));
            c.i = c.f.len();
        }
        if let Some((hecho, donde)) = abierta_pendiente {
            // Como el `while` del teclado: puede nombrar un hecho declarado más abajo.
            self.superficies_pendientes.push((cual, hecho, donde));
        }
        let s = &mut self.e.superficies[cual];
        if let Some(c) = p.get_mut("size") {
            // `size: full, 36`: todo el ancho del monitor.
            s.ancho = if c.palabra("full") { 0 } else { c.num()? as u32 };
            c.exige_sim(",")?;
            s.alto = c.num()? as u32;
        }
        if let Some(c) = p.get_mut("anchor") {
            s.ancla = match c.una_de(voz::ANCLAS_DE_SUPERFICIE, "the anchor of a surface")?.as_str() {
                "top" => Ancla::Arriba,
                "bottom" => Ancla::Abajo,
                "left" => Ancla::Izquierda,
                "right" => Ancla::Derecha,
                "top_left" => Ancla::ArribaIzquierda,
                "top_right" => Ancla::ArribaDerecha,
                "bottom_left" => Ancla::AbajoIzquierda,
                "bottom_right" => Ancla::AbajoDerecha,
                "center" => Ancla::Centro,
                _ => unreachable!(),
            };
        }
        if let Some(c) = p.get_mut("margin") {
            for k in 0..4 {
                s.margen[k] = c.num()? as i32;
                if k < 3 && !c.sim(",") {
                    break;
                }
            }
        }
        if let Some(c) = p.get_mut("level") {
            s.nivel = match c.una_de(voz::NIVELES, "a level")?.as_str() {
                "background" => Nivel::Fondo,
                "bottom" => Nivel::Debajo,
                "top" => Nivel::Encima,
                "overlay" => Nivel::SobreTodo,
                _ => unreachable!(),
            };
        }
        // `kind: window`: una ventana normal, que el compositor decora y coloca. Lo
        // que es de un panel —ancla, nivel, reserva, monitor— no le vale.
        if let Some(c) = p.get_mut("kind") {
            if c.una_de(voz::CLASES_DE_SUPERFICIE, "what kind of surface this is")? == "window" {
                s.ventana = Some(String::new());
            }
        }
        if let Some(c) = p.get_mut("title") {
            let titulo = c.cadena()?;
            c.nada_mas()?;
            match &mut s.ventana {
                Some(t) => *t = titulo,
                None => return Err(Fallo::en(n.linea, n.col, "only a window has a title: add `kind: window`")),
            }
        }
        if s.ventana.is_some() && s.ancho == 0 {
            return Err(Fallo::en(n.linea, n.col, "a window says how wide it is: `full` is for a panel, which is as wide as its monitor"));
        }
        if let Some(c) = p.get_mut("keyboard") {
            s.teclado = match c.una_de(voz::TECLADOS, "how the keyboard is asked for")?.as_str() {
                "none" => Teclado::Nunca,
                "on_demand" => Teclado::AlPulsar,
                "exclusive" => Teclado::Siempre,
                _ => unreachable!(),
            };
            // `keyboard: exclusive while open`: solo mientras eso sea verdad.
            // La condición se lee al final: puede nombrar un hecho declarado más abajo.
            if c.palabra("while") {
                s.teclado_mientras = true;
                self.teclado_pendiente = Some((&c.f[c.i..], c.pos()));
            }
        }
        if let Some(c) = p.get_mut("reserve") {
            s.reserva = c.num()? as i32;
        }
        if let Some(c) = p.get_mut("screens") {
            s.pantallas = if c.palabra("all") {
                Pantallas::Todas
            } else if c.palabra("each") {
                if c.palabra("max") {
                    let _ = c.num()?;
                }
                Pantallas::Numero(0)
            } else {
                let mut v = vec![c.cadena()?];
                while c.sim(",") {
                    v.push(c.cadena()?);
                }
                Pantallas::Estas(v)
            };
        }
        // Una copia por monitor: la misma superficie, cada una en su trozo del plano.
        if let Some(tope) = cuantas {
            let base = self.e.superficies[cual].clone();
            for k in 1..tope {
                self.siguiente_origen += 10000.0;
                let copia = Superficie { instancia: k, origen: (0.0, self.siguiente_origen), pantallas: Pantallas::Numero(k), ..base.clone() };
                // Las de la principal van juntas al principio: la primera sigue siendo la principal.
                if base.nombre.is_empty() {
                    self.e.superficies.insert(k, copia);
                } else {
                    self.e.superficies.push(copia);
                }
            }
            // Lo que el render cuenta de cada monitor, y cuántos hay.
            for k in 0..tope {
                let t = self.e.texto_vivo(fijo(&format!("screen.{k}.name")), "");
                self.textos.insert(format!("screen.{k}.name"), t);
                for parte in ["width", "height"] {
                    let entero = format!("screen.{k}.{parte}");
                    let h = self.e.hecho(fijo(&entero), 0.0);
                    self.hechos.insert(entero, h);
                }
            }
            if !self.hechos.contains_key("screens.count") {
                let h = self.e.hecho("screens.count", 0.0);
                self.hechos.insert("screens.count".into(), h);
            }
        }
        Ok(())
    }

    /// Dentro de una superficie de `screens: each`: lo que vale para esa copia.
    fn ambito_de_pantalla(&self, k: usize) -> Entorno {
        let mut env = Entorno { sufijo: format!("#screen{k}"), ..Default::default() };
        // `$screen` en un nombre es su número, como `$i` en un `repeat`.
        env.exprs.insert("screen".to_owned(), Expr::K(k as f32));
        env.exprs.insert("screen.index".to_owned(), Expr::K(k as f32));
        // Y `screen.name`, `screen.width`, `screen.height` son los de SU monitor.
        for parte in ["name", "width", "height"] {
            env.alias.insert(format!("screen.{parte}"), format!("screen.{k}.{parte}"));
        }
        env.con_partes.insert("screen".to_owned());
        env
    }

    // ── capas ───────────────────────────────────────────────────

    /// `layer card ~calm { open while open { orb.x: 140 ~lively after 70ms } rest { … } }`
    fn capa(&mut self, n: &Nodo, c: &mut Cur) -> R<()> {
        let nombre = self.declarar(&c.id("a name for the layer")?);
        let muelle = if c.sim("~") { self.muelle(c)? } else { Muelle::RAPIDO };
        c.nada_mas()?;
        let mut reclamaciones = Vec::new();
        let mut nombres = Vec::new();
        for e in n.cuerpo.as_deref().unwrap_or(&[]) {
            let Entrada::Nodo(r) = e else {
                return Err(Fallo::en(n.linea, n.col, "a layer holds only claims: `name while …`, `name for 700ms after …`, `name { … }`"));
            };
            let mut c = Cur::de(&r.cabeza, r.linea, r.col);
            let quien = c.id("a name for the claim")?;
            let cuando = if c.palabra("while") {
                Cuando::Mientras(self.expr(&mut c)?)
            } else if c.palabra("for") {
                let dura = c.dur()?;
                c.exige_palabra("after")?;
                Cuando::Tras { sucesos: self.sucesos(&mut c)?, dura }
            } else if c.palabra("from") {
                let desde = self.sucesos(&mut c)?;
                c.exige_palabra("until")?;
                Cuando::DesdeHasta { desde, hasta: self.sucesos(&mut c)? }
            } else {
                Cuando::Siempre
            };
            c.nada_mas()?;
            let mut fija = Vec::new();
            for s in r.cuerpo.as_deref().unwrap_or(&[]) {
                match s {
                    Entrada::Prop { nombre, valor, linea, col } => fija.push(self.transicion(nombre, valor, *linea, *col)?),
                    Entrada::Nodo(x) => return Err(Fallo::en(x.linea, x.col, "here go properties and where they travel to: `orb.x: 140 ~lively after 70ms`")),
                }
            }
            nombres.push(quien.clone());
            reclamaciones.push(Reclamacion { nombre: fijo(&quien), cuando, fija });
        }
        if reclamaciones.is_empty() {
            return Err(Fallo::en(n.linea, n.col, "a layer with no claims decides nothing"));
        }
        let capa = self.e.capa(fijo(&nombre), muelle, reclamaciones);
        // `card.open` vale 1 mientras gane, y va y viene con el muelle de la capa.
        for (k, quien) in nombres.iter().enumerate() {
            self.props.insert(format!("{nombre}.{quien}"), capa.presencia(k));
        }
        Ok(())
    }

    /// `orb.x: 140 ~lively after 70ms`
    fn transicion(&self, nombre: &str, valor: &[Ficha], linea: usize, col: usize) -> R<Transicion> {
        MIRANDO.with(|m| m.set((linea, col)));
        let nombre = &self.global(nombre);
        let Some(prop) = self.props.get(nombre).copied() else {
            let pista = parecido(nombre, self.props.keys()).map_or(String::new(), |p| format!(" Did you mean '{p}'?"));
            return Err(Fallo::en(linea, col, format!("there is no property called '{nombre}'.{pista}")));
        };
        let mut c = Cur::de(valor, linea, col);
        let a = self.expr(&mut c)?;
        let muelle = if c.sim("~") { self.muelle(&mut c)? } else { Muelle::VIVO };
        let retraso = if c.palabra("after") { c.dur()? } else { Duration::ZERO };
        c.nada_mas()?;
        Ok(Transicion { prop, a, muelle, retraso })
    }

    // ── componentes, repeticiones y repartos ────────────────────

    /// `component Chip(label, tone) { size: …; … }`
    fn declarar_componente(&mut self, n: &'a Nodo, c: &mut Cur) -> R<()> {
        let nombre = c.id("a name for the component")?;
        let estricta = self.estrictos.contains(&(n.linea / super::POR_FICHERO));
        // (r: record, chosen: event, tone: color = mint, height = 30)
        let mut parametros: Vec<Parametro> = Vec::new();
        if c.sim("(") && !c.sim(")") {
            loop {
                let nombre = c.id("a parameter name")?;
                if parametros.iter().any(|p| p.nombre == nombre) {
                    c.i -= 1;
                    return c.fallo(format!("'{nombre}' is already a parameter of this component"));
                }
                let tipo = if c.sim(":") { Some(c.una_de(voz::TIPOS_DE_PARAMETRO, "the type of a parameter")?) } else { None };
                let por_defecto = if c.sim("=") {
                    // Hasta la coma o el paréntesis que lo cierre, sin contar los de dentro.
                    let (desde, mut hondo) = (c.i, 0);
                    while let Some(f) = c.mira() {
                        match f {
                            F::Sim("(") => hondo += 1,
                            F::Sim(")") if hondo == 0 => break,
                            F::Sim(")") => hondo -= 1,
                            F::Sim(",") if hondo == 0 => break,
                            _ => {}
                        }
                        c.i += 1;
                    }
                    if c.i == desde {
                        return c.fallo("the default value is missing here");
                    }
                    Some(c.f[desde..c.i].to_vec())
                } else {
                    // Tras uno con valor por defecto, todos: si no, no se sabría cuál se ha saltado.
                    if parametros.last().is_some_and(|p| p.por_defecto.is_some()) {
                        c.i -= 1;
                        return c.fallo(format!("'{nombre}' comes after a parameter with a default, so it needs one too"));
                    }
                    None
                };
                parametros.push(Parametro { nombre, tipo, por_defecto });
                if c.sim(")") {
                    break;
                }
                c.exige_sim(",")?;
            }
        }
        c.nada_mas()?;
        if let Some(ya) = self.componentes.get(&nombre) {
            return Err(Fallo::en(n.linea, n.col, format!("there is already a component '{nombre}', at {}. Two with the same name cannot live together: change one", super::sitio(self.ficheros, ya.nodo.linea))));
        }
        if n.cuerpo.is_none() {
            return Err(Fallo::en(n.linea, n.col, "this component is missing its `{ … }` block"));
        }
        // Qué huecos tiene: se necesita saber al usarlo, para repartir lo que traiga la copia.
        fn huecos_de(entradas: &[Entrada], en: &mut Vec<String>) {
            for e in entradas {
                let Entrada::Nodo(x) = e else { continue };
                let palabra = |k: usize| match x.cabeza.get(k).map(|f| &f.f) { Some(F::Id(p)) => Some(p.clone()), _ => None };
                match palabra(0).as_deref() {
                    Some("children") => en.push(palabra(1).unwrap_or_default()),
                    Some("component") => {}
                    _ => huecos_de(x.cuerpo.as_deref().unwrap_or(&[]), en),
                }
            }
        }
        let mut huecos = Vec::new();
        huecos_de(n.cuerpo.as_deref().unwrap_or(&[]), &mut huecos);
        if let Some(palabra) = huecos.iter().find(|h| voz::SENTENCIAS.contains(&h.as_str())) {
            return Err(Fallo::en(n.linea, n.col, format!("a slot cannot be called '{palabra}', which is a word of the language: in the copy, `{palabra} {{ … }}` already means something else")));
        }
        if let Some(repetido) = huecos.iter().enumerate().find(|(k, h)| huecos[..*k].contains(h)).map(|(_, h)| h.clone()) {
            let cual = if repetido.is_empty() { "a single unnamed `children`".to_owned() } else { format!("a single slot called '{repetido}'") };
            return Err(Fallo::en(n.linea, n.col, format!("a component has {cual}: give the other one a name (`children footer`)")));
        }
        self.componentes.insert(nombre, Componente { estricta, huecos, parametros, nodo: n });
        Ok(())
    }

    /// Lee un argumento y lo deja en el ámbito de la copia. Con tipo, se lee como
    /// lo que es y un fallo dice qué se esperaba; sin él, se adivina por su forma.
    fn argumento(&self, componente: &str, p: &Parametro, c: &mut Cur, env: &mut Entorno) -> R<()> {
        let nombre = &p.nombre;
        let esperaba = |c: &Cur, que: &str| c.fallo::<()>(format!("'{nombre}', of '{componente}', is {que}, and this is not")).unwrap_err();
        match p.tipo.as_deref() {
            Some("number" | "bool") => {
                env.exprs.insert(nombre.clone(), self.expr(c)?);
            }
            Some("spring") => {
                env.muelles.insert(nombre.clone(), self.muelle(c)?);
            }
            Some("gesture") => {
                let Some(F::Id(x)) = c.mira() else { return Err(esperaba(c, "a gesture")) };
                if !self.gestos.contains_key(x) {
                    c.i += 1;
                    return self.desconocido(c, "no gesture", x, self.gestos.keys().collect());
                }
                env.alias.insert(nombre.clone(), x.clone());
                c.i += 1;
            }
            Some("color") => {
                env.colores.insert(nombre.clone(), self.color(c).map_err(|_| esperaba(c, "a colour"))?);
            }
            Some("text") => match c.mira() {
                Some(F::Cadena(t)) => {
                    if let plantilla @ Contenido::Plantilla(_) = self.contenido_de(t, &c.f[c.i])? {
                        env.contenidos.insert(nombre.clone(), plantilla);
                    }
                    env.cadenas.insert(nombre.clone(), t.clone());
                    c.i += 1;
                }
                Some(F::Id(x)) if self.textos.contains_key(&self.global(x)) => {
                    env.alias.insert(nombre.clone(), self.global(x));
                    c.i += 1;
                }
                Some(F::Id(x)) if self.entornos.iter().any(|e| e.cadenas.contains_key(x)) => {
                    let de_fuera = self.entornos.iter().rev().find(|e| e.cadenas.contains_key(x)).unwrap();
                    env.cadenas.insert(nombre.clone(), de_fuera.cadenas[x].clone());
                    if let Some(k) = de_fuera.contenidos.get(x) {
                        env.contenidos.insert(nombre.clone(), k.clone());
                    }
                    c.i += 1;
                }
                _ => return Err(esperaba(c, "a text: quoted, or the name of a live text")),
            },
            Some("record") => {
                let Some(F::Id(x)) = c.mira() else { return Err(esperaba(c, "a record of a model")) };
                let Some((ficha, indice)) = self.ficha(x) else { return Err(esperaba(c, "a record of a model (the one from a `for`, or `rows.0`)")) };
                env.alias.insert(nombre.clone(), ficha);
                env.con_partes.insert(nombre.clone());
                env.exprs.insert(format!("{nombre}.index"), Expr::K(indice as f32));
                c.i += 1;
            }
            Some("event") => {
                // Dentro, `emit chosen` y `on chosen` hablan del suceso que se pasó.
                let Some(F::Id(x)) = c.mira() else { return Err(esperaba(c, "an event")) };
                let g = self.global(x);
                if !self.sucesos.contains_key(&g) {
                    c.i += 1;
                    return self.desconocido(c, "no event", &g, self.sucesos.keys().collect());
                }
                env.alias.insert(nombre.clone(), g);
                c.i += 1;
            }
            Some("image") => {
                let Some(F::Id(x)) = c.mira() else { return Err(esperaba(c, "an image")) };
                let g = self.global(x);
                if !self.imagenes.contains_key(&g) {
                    c.i += 1;
                    return self.desconocido(c, "ninguna imagen", &g, self.imagenes.keys().collect());
                }
                env.alias.insert(nombre.clone(), g);
                c.i += 1;
            }
            Some(otro) => unreachable!("'{otro}' is in the vocabulary, but `argumento` cannot read it"),
            None => self.argumento_sin_tipo(nombre, c, env)?,
        }
        Ok(())
    }

    /// Sin tipo declarado: lo que parezca. Es como se escribían los componentes antes de tenerlos.
    fn argumento_sin_tipo(&self, p: &String, c: &mut Cur, env: &mut Entorno) -> R<()> {
        let sigue = c.f.get(c.i + 1).map(|x| &x.f);
        let solo = matches!(sigue, Some(F::Sim(",")) | Some(F::Sim(")")) | None);
        match c.mira() {
            Some(F::Cadena(t)) => {
                // Los huecos hablan de los nombres de aquí, no de los de dentro del componente.
                if let plantilla @ Contenido::Plantilla(_) = self.contenido_de(t, &c.f[c.i])? {
                    env.contenidos.insert(p.clone(), plantilla);
                }
                env.cadenas.insert(p.clone(), t.clone());
                c.i += 1;
            }
            Some(F::Color(_)) => {
                env.colores.insert(p.clone(), self.color(c)?);
            }
            Some(F::Id(x)) if x == "mix" && matches!(c.f.get(c.i + 2).map(|y| &y.f), Some(F::Color(_))) => {
                env.colores.insert(p.clone(), self.color(c)?);
            }
            Some(F::Id(x)) if solo && (self.colores.contains_key(x) || self.entornos.iter().any(|e| e.colores.contains_key(x))) => {
                env.colores.insert(p.clone(), self.color(c)?);
            }
            // Una ficha de un modelo —la de un `for`, o `rows.3`—: dentro, `p.label` es su campo.
            Some(F::Id(x)) if solo && self.ficha(x).is_some() => {
                let (ficha, indice) = self.ficha(x).unwrap();
                env.alias.insert(p.clone(), ficha);
                env.con_partes.insert(p.clone());
                env.exprs.insert(format!("{p}.index"), Expr::K(indice as f32));
                c.i += 1;
            }
            // El nombre de un texto, una imagen o un gesto: el parámetro es otro nombre para él.
            Some(F::Id(x)) if solo && { let g = self.global(x); self.textos.contains_key(&g) || self.imagenes.contains_key(&g) || self.gestos.contains_key(&g) || self.sucesos.contains_key(&g) } => {
                env.alias.insert(p.clone(), self.global(x));
                c.i += 1;
            }
            Some(F::Id(x)) if solo && self.entornos.iter().any(|e| e.cadenas.contains_key(x)) => {
                env.cadenas.insert(p.clone(), self.entornos.iter().rev().find_map(|e| e.cadenas.get(x)).unwrap().clone());
                c.i += 1;
            }
            _ => {
                env.exprs.insert(p.clone(), self.expr(c)?);
            }
        }
        Ok(())
    }

    /// Las reglas que se apuntaron mientras se leía un ámbito se quedan con ese
    /// ámbito entero: así pueden nombrar una forma que se declaró después.
    fn cerrar_ambito(&mut self, desde: usize) {
        let ahora = self.entornos.clone();
        for r in &mut self.reglas[desde..] {
            if r.1.len() == ahora.len() {
                r.1 = ahora.clone();
            }
        }
        self.entornos.pop();
    }

    /// `Chip("Hola", mint) { move: 10, 20 }`: una copia, con sus parámetros y sus
    /// propios nombres por dentro. Por fuera es un grupo.
    fn copia_de(&mut self, n: &'a Nodo, c: &mut Cur) -> R<()> {
        let nombre = c.f[0].f.clone();
        let F::Id(nombre) = nombre else { unreachable!() };
        let (parametros, cuerpo) = {
            let k = &self.componentes[&nombre];
            (k.parametros.clone(), k.nodo.cuerpo.as_deref().unwrap_or(&[]))
        };
        let mut env = Entorno::default();
        let mut dados = vec![false; parametros.len()];
        let firma = || parametros.iter().map(|p| match (&p.tipo, &p.por_defecto) {
            (Some(t), None) => format!("{}: {t}", p.nombre),
            (Some(t), Some(_)) => format!("{}: {t} = …", p.nombre),
            (None, None) => p.nombre.clone(),
            (None, Some(_)) => format!("{} = …", p.nombre),
        }).collect::<Vec<_>>().join(", ");
        if c.sim("(") && !c.sim(")") {
            let mut por_nombre = false;
            let mut k = 0;
            loop {
                // `tone: mint`: desde el primero con nombre, todos con nombre.
                let con_nombre = matches!((c.mira(), c.f.get(c.i + 1).map(|x| &x.f)), (Some(F::Id(_)), Some(F::Sim(":"))));
                let cual = if con_nombre {
                    por_nombre = true;
                    let n = c.id("a parameter name")?;
                    c.exige_sim(":")?;
                    match parametros.iter().position(|p| p.nombre == n) {
                        Some(k) => k,
                        None => {
                            c.i -= 2;
                            let todos: Vec<String> = parametros.iter().map(|p| p.nombre.clone()).collect();
                            let pista = parecido(&n, todos.iter()).map_or(String::new(), |p| format!(" Did you mean '{p}'?"));
                            return c.fallo(format!("'{nombre}' has no parameter called '{n}'.{pista} It is {nombre}({})", firma()));
                        }
                    }
                } else if por_nombre {
                    return c.fallo("after one named argument, they all carry their name");
                } else {
                    k += 1;
                    k - 1
                };
                if cual >= parametros.len() {
                    return c.fallo(format!("'{nombre}' has too many arguments: it is {nombre}({})", firma()));
                }
                if std::mem::replace(&mut dados[cual], true) {
                    return c.fallo(format!("'{}' has already been given", parametros[cual].nombre));
                }
                self.argumento(&nombre, &parametros[cual], c, &mut env)?;
                if c.sim(")") {
                    break;
                }
                c.exige_sim(",")?;
            }
        }
        // Lo que no se ha dado: su valor por defecto, leído aquí, o un fallo que dice qué falta.
        for (p, dado) in parametros.iter().zip(&dados) {
            if *dado {
                continue;
            }
            let Some(fichas) = &p.por_defecto else {
                let que = p.tipo.as_ref().map_or(String::new(), |t| format!(" ({})", nombre_de_tipo(t)));
                return Err(Fallo::en(n.linea, n.col, format!("'{nombre}' is missing '{}'{que}: it is {nombre}({})", p.nombre, firma())));
            };
            let mut d = Cur::de(fichas, n.linea, n.col);
            self.argumento(&nombre, p, &mut d, &mut env)?;
            d.nada_mas()?;
        }
        c.nada_mas()?;
        self.copias += 1;
        env.sufijo = format!("#{nombre}{}", self.copias);
        env.copia = Some((nombre.clone(), n.linea));
        env.estricta = self.componentes[&nombre].estricta;
        env.biblioteca = Some(self.componentes[&nombre].nodo.linea / super::POR_FICHERO).filter(|k| *k > 0);
        let en_hueco = std::mem::take(&mut self.en_hueco);
        let desde = self.reglas.len();
        // Lo que la copia trae dentro de su bloque va donde el componente diga `children`,
        // y se lee con los nombres de aquí fuera.
        let huecos_que_hay = self.componentes[&nombre].huecos.clone();
        let mut huecos: Vec<(String, Vec<&'a Entrada>, bool)> = huecos_que_hay.iter().map(|h| (h.clone(), Vec::new(), false)).collect();
        let mut sin_sitio = None;
        for e in n.cuerpo.as_deref().unwrap_or(&[]) {
            let Entrada::Nodo(x) = e else { continue };
            // `header { … }`: un bloque con el nombre de un hueco es lo que va en ese hueco.
            let bloque = match (x.cabeza.as_slice(), &x.cuerpo) {
                ([Ficha { f: F::Id(p), .. }], Some(_)) if huecos_que_hay.contains(p) || (!voz::SENTENCIAS.contains(&p.as_str()) && !self.componentes.contains_key(p)) => Some(p.clone()),
                _ => None,
            };
            match bloque {
                Some(p) => match huecos.iter_mut().find(|h| h.0 == p) {
                    Some(h) => h.1.extend(x.cuerpo.as_deref().unwrap_or(&[]).iter()),
                    None => {
                        let con_nombre: Vec<String> = huecos_que_hay.iter().filter(|h| !h.is_empty()).cloned().collect();
                        let pista = parecido(&p, con_nombre.iter()).map_or(String::new(), |q| format!(" Did you mean '{q}'?"));
                        let tiene = if con_nombre.is_empty() { "has no named slots".to_owned() } else { format!("has {}", con_nombre.join(", ")) };
                        return Err(Fallo::en(x.linea, x.col, format!("'{nombre}' has no slot called '{p}': it {tiene}.{pista}")));
                    }
                },
                None => match huecos.iter_mut().find(|h| h.0.is_empty()) {
                    Some(h) => h.1.push(e),
                    None => sin_sitio = sin_sitio.or(Some((x.linea, x.col))),
                },
            }
        }
        if let Some((l, col)) = sin_sitio {
            return Err(Fallo::en(l, col, format!("'{nombre}' has nowhere to put what goes inside it: its component is missing a `children`")));
        }
        self.hijos_de_copia.push(HijosDeCopia { fuera: self.entornos.clone(), huecos });
        self.entornos.push(env);
        self.adelantar_medidas(cuerpo);
        // Cuánto ocupa lo dice el propio componente: `size: 300, 44`. Si falla, el ámbito
        // se cierra igual: si no, lo de después se leería como si estuviera aquí dentro.
        let mut tam = None;
        let mut r = Ok(());
        for e in cuerpo {
            if let Entrada::Prop { nombre, valor, linea, col } = e {
                if nombre == "size" {
                    let mut c = Cur::de(valor, *linea, *col);
                    match self.punto(&mut c) {
                        Ok(t) => tam = Some(t),
                        Err(f) => r = Err(f),
                    }
                }
            }
        }
        if r.is_ok() {
            r = self.grupo_con_propiedades(n, cuerpo);
        }
        self.cerrar_ambito(desde);
        self.hijos_de_copia.pop();
        let _ = en_hueco;
        if tam.is_some() {
            self.ultimo_tam = tam;
        }
        r
    }

    /// `repeat i in 0..6 { … }`: el bloque, una vez por cada valor. Se despliega
    /// al cargar; no hay bucles en marcha.
    fn repetir(&mut self, n: &'a Nodo, c: &mut Cur) -> R<()> {
        let (var, desde, hasta) = self.cabeza_de_repeat(c)?;
        for v in desde..hasta {
            let marca = self.reglas.len();
            self.abrir_vuelta(&var, v);
            self.grupo(n.cuerpo.as_deref().unwrap_or(&[]).iter());
            self.cerrar_ambito(marca);
        }
        Ok(())
    }

    fn cabeza_de_repeat(&self, c: &mut Cur) -> R<(String, i64, i64)> {
        let var = c.id("a name for the counter")?;
        c.exige_palabra("in")?;
        let cte = |o: &Self, c: &mut Cur| -> R<i64> {
            match o.expr(c)? {
                Expr::K(v) => Ok(v as i64),
                _ => c.fallo("the bounds of a `repeat` have to be numbers: it unfolds when loading"),
            }
        };
        let desde = cte(self, c)?;
        c.exige_sim("..")?;
        let hasta = cte(self, c)?;
        c.nada_mas()?;
        if hasta - desde > 512 {
            return c.fallo("more than 512 turns in a `repeat` means something is wrong");
        }
        Ok((var, desde, hasta))
    }

    fn abrir_vuelta(&mut self, var: &str, v: i64) {
        let mut env = Entorno { sufijo: format!("#{var}{v}"), ..Default::default() };
        env.exprs.insert(var.to_owned(), Expr::K(v as f32));
        self.entornos.push(env);
    }

    /// `row bar ~calm { at: x, y; gap: 8; padding: 6; align: center; fill: #222; corner: 12; …hijos… }`
    ///
    /// Reparte a sus hijos en fila o en columna. No hay motor de layout: el
    /// sitio de cada hijo es una expresión —lo que ocupan los anteriores—, así
    /// que si uno crece los demás se corren, y con un muelle se corren animados.
    fn reparto(&mut self, n: &'a Nodo, c: &mut Cur, fila: bool) -> R<()> {
        let nombre = match c.mira() {
            Some(F::Id(_)) => Some(c.id("a name")?),
            _ => None,
        };
        let muelle = if c.sim("~") { Some(self.muelle(c)?) } else { None };
        c.nada_mas()?;
        let mut p = self.propiedades(n, voz::propiedades("layout"))?;
        let cursor_del_reparto = match p.get_mut("cursor") {
            Some(c) => leer_cursor(c)?,
            None => Cursor::Normal,
        };
        self.en_hueco = false;
        let origen = match p.get_mut("at") {
            Some(c) => self.punto(c)?,
            None => (0.0.into(), 0.0.into()),
        };
        let mut una = |o: &Self, k: &str, defecto: f32| -> R<Expr> {
            match p.get_mut(k) {
                Some(c) => o.expr(c),
                None => Ok(Expr::K(defecto)),
            }
        };
        let (hueco, relleno, esquina) = (una(self, "gap", 0.0)?, una(self, "padding", 0.0)?, una(self, "corner", 0.0)?);
        let esquina_de_zona = esquina.clone();
        let alinea = match p.get_mut("align") {
            Some(c) => match c.una_de(voz::ALINEADOS_DE_REPARTO, "the alignment of a layout")?.as_str() {
                "start" => 0.0,
                "center" => 0.5,
                "end" => 1.0,
                _ => unreachable!(),
            },
            None => 0.0,
        };
        let fondo = match p.get_mut("fill") {
            Some(c) => Some(self.color(c)?),
            None => None,
        };
        // `view: 300, 200`: lo que se ve. Lo de dentro puede ser más largo, y se desplaza.
        let vista = match p.get_mut("view") {
            Some(c) => Some(self.punto(c)?),
            None => None,
        };
        let paso = match p.get_mut("step") {
            Some(c) => self.expr(c)?,
            None => Expr::K(60.0),
        };
        // `wrap: 5`: cinco por línea y a la siguiente. Una rejilla, con la celda del
        // tamaño del hijo más grande; lo que no está no deja hueco.
        let envuelve = match p.get_mut("wrap") {
            Some(c) => {
                let cuantos = c.num()? as usize;
                c.nada_mas()?;
                if !(1..=64).contains(&cuantos) {
                    return Err(Fallo::en(n.linea, n.col, "`wrap` goes from 1 to 64: how many fit in a line before jumping to the next"));
                }
                Some(cuantos)
            }
            None => None,
        };
        // `content: rows.total * 30`: lo que habría si estuviera todo. Para una lista
        // que no despliega más que su ventana, es el largo de verdad.
        let contenido_dicho = match p.get_mut("content") {
            Some(c) => Some(self.expr(c)?),
            None => None,
        };
        let opacidad = match p.get_mut("opacity") {
            Some(c) => Some(self.expr(c)?),
            None => None,
        };

        // Qué parte del reparto cae sobre `at`: `anchor: right` lo pega por la derecha
        // mida lo que mida, que es lo que quiere lo que va al final de una barra.
        let mut ancla = (0.0f32, 0.0f32);
        if let Some(c) = p.get_mut("anchor") {
            while !c.acabo() {
                match c.id("left, center, right, top o bottom")?.as_str() {
                    "left" => ancla.0 = 0.0,
                    "right" => ancla.0 = 1.0,
                    "top" => ancla.1 = 0.0,
                    "bottom" => ancla.1 = 1.0,
                    "center" => ancla.0 = 0.5,
                    "middle" => ancla.1 = 0.5,
                    _ => return c.fallo("an anchor is left, center or right, and top, middle or bottom"),
                }
            }
        }
        // Todo el reparto vive bajo una transformación que lo lleva a su origen. Si
        // tiene ancla, el origen depende de lo que mida, y eso se sabe al final.
        let base = Transformacion::en((0.0.into(), 0.0.into())).mueve(origen.0.clone(), origen.1.clone());
        let (instr_base, nivel_base, candidatas_base) = (self.e.instrs.len(), self.bajo.len(), self.candidatas.len());
        self.e.pintar(Instr::Transformar(Some(base.clone())));
        self.bajo.push(base);
        if let Some(o) = &opacidad {
            self.e.pintar(Instr::Opacidad(Some(o.clone())));
        }
        // El fondo se pinta antes que los hijos, pero su tamaño se sabe después:
        // se deja el sitio y se rellena al final.
        let sitio_del_fondo = fondo.as_ref().map(|_| {
            let k = self.e.instrs.len();
            self.e.pintar(Instr::Recorte(None));
            k
        });

        // Los hijos, con los `repeat` ya desplegados.
        let mut hijos: Vec<(&'a Nodo, Vec<Entorno>)> = Vec::new();
        self.desplegar(n.cuerpo.as_deref().unwrap_or(&[]).iter().collect(), &mut Vec::new(), &mut hijos)?;

        struct Puesto {
            instr: usize,
            candidatas: std::ops::Range<usize>,
            tam: (Expr, Expr),
            visible: Expr,
            /// Es lo que va entre dos hijos, no un hijo.
            separa: bool,
        }
        // Lo que se ve es una ventana a lo que hay: se recorta, y lo de dentro va corrido.
        let desplaza = vista.as_ref().map(|(vw, vh)| {
            self.copias += 1;
            let prop = self.e.prop_con(fijo(&format!("·corrido{}", self.copias)), 0.0, muelle.unwrap_or(Muelle::RAPIDO));
            let caja = Forma::Caja { centro: (vw.clone() * 0.5, vh.clone() * 0.5), mitad: (vw.clone() * 0.5, vh.clone() * 0.5), radio: esquina.clone() };
            self.e.pintar(Instr::Recorte(Some((caja, 0.0))));
            let corrido = Expr::K(0.0) - prop.e();
            let mueve = if fila { (corrido, Expr::K(0.0)) } else { (Expr::K(0.0), corrido) };
            let t = Transformacion { mueve, ..Transformacion::en((0.0.into(), 0.0.into())) };
            self.e.pintar(Instr::Transformar(Some(t.clone())));
            self.bajo.push(t);
            prop
        });
        let nivel = self.bajo.len();
        let mut puestos: Vec<Puesto> = Vec::new();
        // `between { … }`: lo que va entre cada dos hijos que estén.
        let mut separador: Option<(&'a Nodo, Option<String>)> = None;
        for e in n.cuerpo.as_deref().unwrap_or(&[]) {
            let Entrada::Nodo(x) = e else { continue };
            if !matches!(x.cabeza.first().map(|f| &f.f), Some(F::Id(p)) if p == "between") {
                continue;
            }
            if separador.is_some() {
                return Err(Fallo::en(x.linea, x.col, "a layout has a single `between`"));
            }
            // `between i { … }`: dentro, `i` es entre quiénes está: 1 tras el primer hijo, 2 tras el segundo…
            let contador = match x.cabeza.as_slice() {
                [_] => None,
                [_, Ficha { f: F::Id(v), .. }] => Some(v.clone()),
                _ => return Err(Fallo::en(x.linea, x.col, "after `between` only a name for its position can go: `between i { … }`")),
            };
            let cuerpo = x.cuerpo.as_deref().unwrap_or(&[]);
            let dentro: Vec<&'a Nodo> = cuerpo.iter().filter_map(|e| if let Entrada::Nodo(y) = e { Some(y) } else { None }).collect();
            let con_tamano = cuerpo.iter().any(|e| matches!(e, Entrada::Prop { nombre, .. } if nombre == "size"));
            separador = Some(match (dentro.as_slice(), con_tamano) {
                // Una sola cosa, que dice ella cuánto ocupa.
                ([una], false) => (*una, contador),
                // Varias (o una con su sitio alrededor): el `between` hace de grupo, y dice cuánto ocupa.
                ([_, ..], true) => (x, contador),
                ([], _) => return Err(Fallo::en(x.linea, x.col, "an empty `between` separates nothing: `between { box { size: 200, 1; color: ink } }`")),
                _ => return Err(Fallo::en(x.linea, x.col, "a `between` with several things has to say how much room it takes: `between { size: 200, 9; … }`")),
            });
        }
        // Los hijos van saliendo de una cola: tras cada uno (menos el primero) se cuela su separador,
        // que se ve si ese hijo está y alguno de los de antes también.
        let mut cola: std::collections::VecDeque<(&'a Nodo, Vec<Entorno>, Option<Expr>)> = hijos.into_iter().map(|(h, e)| (h, e, None)).collect();
        let mut presencias: Vec<Expr> = Vec::new();
        while let Some((hijo, entornos, forzada)) = cola.pop_front() {
            let marca = self.reglas.len();
            // Un hijo que viene de fuera del componente se lee con el ámbito de quien lo escribió.
            let de_fuera = entornos.first().and_then(|e| e.ambito_de_fuera.clone());
            let (extra, dentro, pendientes) = match de_fuera {
                Some(fuera) => {
                    // Tras la marca van las vueltas de `repeat` o `for` que lo envuelvan, si las hay.
                    let dentro = std::mem::replace(&mut self.entornos, fuera);
                    let extra = entornos.len() - 1;
                    self.entornos.extend(entornos.into_iter().skip(1));
                    (extra, Some(dentro), std::mem::take(&mut self.hijos_de_copia))
                }
                None => {
                    let extra = entornos.len();
                    self.entornos.extend(entornos);
                    (extra, None, Vec::new())
                }
            };
            // `show:` decide si el hijo está: ocupa y se ve, o ni lo uno ni lo otro.
            let mut visible = Expr::K(1.0);
            for e in hijo.cuerpo.as_deref().unwrap_or(&[]) {
                if let Entrada::Prop { nombre, valor, linea, col } = e {
                    if nombre == "show" {
                        let mut c = Cur::de(valor, *linea, *col);
                        visible = self.expr(&mut c)?;
                    }
                }
            }
            for e in &self.entornos[self.entornos.len() - extra..] {
                if let Some(v) = &e.visible {
                    visible = if matches!(visible, Expr::K(k) if k == 1.0) { v.clone() } else { visible * v.clone() };
                }
            }
            if let Some(f) = &forzada {
                visible = f.clone();
            }
            // Lo que decide si está, sin muelles: es lo que apaga sus zonas.
            let esta = (!matches!(visible, Expr::K(_))).then(|| visible.clone());
            if let (Some(m), false) = (muelle, matches!(visible, Expr::K(_))) {
                // Con muelle, aparecer y desaparecer también es un viaje.
                self.copias += 1;
                let v = self.e.prop_con(fijo(&format!("·visible{}", self.copias)), 0.0, m);
                self.e.comportamientos.push(Comportamiento::Sigue { prop: v, a: visible });
                visible = v.e().acotar(0.0, 1.0);
            }
            let con_opacidad = !matches!(visible, Expr::K(_));
            if con_opacidad {
                self.e.pintar(Instr::Opacidad(Some(visible.clone())));
            }
            let instr = self.e.instrs.len();
            let hueco_del_hijo = Transformacion::en((0.0.into(), 0.0.into()));
            self.e.pintar(Instr::Transformar(Some(hueco_del_hijo.clone())));
            self.bajo.push(hueco_del_hijo);
            let desde = self.candidatas.len();
            if matches!(hijo.cabeza.first().map(|f| &f.f), Some(F::Id(t)) if t == "text") {
                self.copias += 1;
                self.medida_impuesta = Some(self.e.medida(fijo(&format!("·medida{}", self.copias))));
            }
            self.en_hueco = true;
            self.ultimo_tam = None;
            let mut sin_recortes = 0;
            let r = self.sentencia(hijo, &mut sin_recortes);
            self.en_hueco = false;
            self.medida_impuesta = None;
            self.bajo.pop();
            self.e.pintar(Instr::Transformar(None));
            if con_opacidad {
                self.e.pintar(Instr::Opacidad(None));
            }
            for _ in 0..extra {
                self.cerrar_ambito(marca);
            }
            if let Some(dentro) = dentro {
                self.entornos = dentro;
                self.hijos_de_copia = pendientes;
            }
            r?;
            let Some(tam) = self.ultimo_tam.take() else {
                return Err(Fallo::en(hijo.linea, hijo.col, "I don\'t know how much room this takes inside a layout: put it in a `group` with `size: width, height`"));
            };
            if let Some(esta) = &esta {
                for c in &mut self.candidatas[desde..] {
                    c.visible = Some(match c.visible.take() { Some(v) => v * esta.clone(), None => esta.clone() });
                }
            }
            let puesto = Puesto { instr, candidatas: desde..self.candidatas.len(), tam, visible, separa: forzada.is_some() };
            match forzada {
                // Un separador se pinta después de su hijo, pero su sitio es justo antes.
                Some(_) => puestos.insert(puestos.len() - 1, puesto),
                None => {
                    let presencia = esta.unwrap_or(Expr::K(1.0));
                    if let (Some((sep, contador)), Some(antes)) = (&separador, presencias.iter().cloned().reduce(|a, b| a + b)) {
                        self.copias += 1;
                        let mut env = Entorno { sufijo: format!("#between{}", self.copias), ..Default::default() };
                        if let Some(v) = contador {
                            env.exprs.insert(v.clone(), Expr::K(presencias.len() as f32));
                        }
                        let sep = *sep;
                        cola.push_front((sep, vec![env], Some(presencia.clone() * antes.min(Expr::K(1.0)))));
                    }
                    presencias.push(presencia);
                    puestos.push(puesto);
                }
            }
        }
        // Cuántos hijos están ahora mismo: `list.count`.
        let cuantos = presencias.into_iter().reduce(|a, b| a + b).unwrap_or(Expr::K(0.0));

        // Lo que ocupa cada uno a lo largo, y lo más que ocupa cualquiera a lo ancho.
        let largo = |p: &Puesto| if fila { p.tam.0.clone() } else { p.tam.1.clone() };
        let ancho = |p: &Puesto| if fila { p.tam.1.clone() } else { p.tam.0.clone() };
        let maximo = puestos.iter().fold(Expr::K(0.0), |m, p| m.max(ancho(p) * p.visible.clone()));
        // Con `wrap`, la celda mide lo que el hijo más grande, y cada uno va a la suya.
        let celda_largo = puestos.iter().fold(Expr::K(0.0), |m, p| m.max(largo(p))) + hueco.clone();
        let celda_ancho = maximo.clone() + hueco.clone();
        let mut corrido = relleno.clone();
        // Cuántos de los anteriores están: lo que no se ve no ocupa celda.
        let mut van = Expr::K(0.0);
        for (k, p) in puestos.iter().enumerate() {
            // Un separador no abre otro hueco: se pone en medio del que ya hay entre sus vecinos.
            let mut a_lo_largo = if p.separa { corrido.clone() - hueco.clone() * 0.5 } else { corrido.clone() };
            let mut de_rejilla = None;
            if let Some(cuantos) = envuelve {
                let por_linea = Expr::K(cuantos as f32);
                let fila_k = (van.clone() / por_linea.clone()).suelo();
                let columna = van.clone() - fila_k.clone() * por_linea;
                a_lo_largo = relleno.clone() + columna * celda_largo.clone();
                de_rejilla = Some(relleno.clone() + fila_k * celda_ancho.clone());
                van = van + p.visible.clone();
            }
            if let Some(m) = muelle {
                // El hueco es un destino: el hijo va hacia él con el muelle del reparto.
                self.copias += 1;
                let prop = self.e.prop_con(fijo(&format!("·hueco{}", self.copias)), 0.0, m);
                self.e.comportamientos.push(Comportamiento::Sigue { prop, a: a_lo_largo });
                a_lo_largo = prop.e();
                if let Some(a) = de_rejilla.take() {
                    self.copias += 1;
                    let otra = self.e.prop_con(fijo(&format!("·salto{}", self.copias)), 0.0, m);
                    self.e.comportamientos.push(Comportamiento::Sigue { prop: otra, a });
                    de_rejilla = Some(otra.e());
                }
            }
            let a_lo_ancho = de_rejilla.unwrap_or_else(|| relleno.clone() + (maximo.clone() - ancho(p)) * alinea);
            let mueve = if fila { (a_lo_largo, a_lo_ancho) } else { (a_lo_ancho, a_lo_largo) };
            if let Instr::Transformar(Some(t)) = &mut self.e.instrs[p.instr] {
                t.mueve = mueve.clone();
            }
            for c in &mut self.candidatas[p.candidatas.clone()] {
                c.bajo[nivel].mueve = mueve.clone();
            }
            let ultimo = k + 1 == puestos.len();
            corrido = corrido + (largo(p) + if ultimo || p.separa { Expr::K(0.0) } else { hueco.clone() }) * p.visible.clone();
        }
        let (total_largo, total_ancho) = match envuelve {
            // Una rejilla mide lo que sus líneas: la última no lleva hueco detrás.
            Some(cabidos) => {
                let por_linea = Expr::K(cabidos as f32);
                let en_linea = cuantos.clone().min(por_linea.clone());
                let lineas = (cuantos.clone() / por_linea).techo();
                (
                    (en_linea * celda_largo - hueco.clone()).max(Expr::K(0.0)) + relleno.clone() * 2.0,
                    (lineas * celda_ancho - hueco.clone()).max(Expr::K(0.0)) + relleno.clone() * 2.0,
                )
            }
            None => (corrido + relleno.clone(), maximo + relleno.clone() * 2.0),
        };
        let mut contenido = if fila { (total_largo, total_ancho) } else { (total_ancho, total_largo) };
        // Lo que dice la escena manda: el reparto solo lleva la ventana, pero el
        // desplazamiento va sobre la lista entera.
        if let Some(e) = &contenido_dicho {
            if fila { contenido.0 = e.clone() } else { contenido.1 = e.clone() }
        }
        // Con `view:`, hacia fuera ocupa lo que se ve, no lo que lleva dentro.
        let tam = vista.clone().unwrap_or_else(|| contenido.clone());

        if let (Some(k), Some(color)) = (sitio_del_fondo, fondo) {
            self.e.instrs[k] = Instr::Plano {
                forma: Forma::Caja { centro: (tam.0.clone() * 0.5, tam.1.clone() * 0.5), mitad: (tam.0.clone() * 0.5, tam.1.clone() * 0.5), radio: esquina },
                color,
                alfa: Expr::K(1.0),
            };
        }
        if ancla != (0.0, 0.0) {
            let mueve = (origen.0 - tam.0.clone() * ancla.0, origen.1 - tam.1.clone() * ancla.1);
            if let Instr::Transformar(Some(t)) = &mut self.e.instrs[instr_base] {
                t.mueve = mueve.clone();
            }
            for c in &mut self.candidatas[candidatas_base..] {
                c.bajo[nivel_base].mueve = mueve.clone();
            }
        }
        if desplaza.is_some() {
            self.bajo.pop();
            self.e.pintar(Instr::Transformar(None));
            self.e.pintar(Instr::Recorte(None));
        }
        if opacidad.is_some() {
            self.e.pintar(Instr::Opacidad(None));
        }
        self.bajo.pop();
        self.e.pintar(Instr::Transformar(None));
        // Con nombre, su tamaño se puede usar más abajo (`bar.width`), y su caja
        // entera es una zona si alguna regla la nombra. Va debajo de las de sus
        // hijos: la rueda sobre el reparto no le quita el clic a lo de dentro.
        if let Some(local) = nombre {
            // El nombre de un reparto se declara cuando ya se han leído sus hijos, así
            // que hay que volver a decir dónde estaba: si no, el editor lleva al último.
            MIRANDO.with(|m| m.set((n.linea, n.col)));
            self.clase_actual = if fila { "row".to_owned() } else { "column".to_owned() };
            let nombre = self.declarar(&local);
            if let Some(e) = self.entornos.last_mut() {
                e.con_partes.insert(local.clone());
            }
            let mut bajo = self.bajo.clone();
            if let Instr::Transformar(Some(t)) = &self.e.instrs[instr_base] {
                bajo.push(t.clone());
            }
            let caja = Forma::Caja { centro: (tam.0.clone() * 0.5, tam.1.clone() * 0.5), mitad: (tam.0.clone() * 0.5, tam.1.clone() * 0.5), radio: esquina_de_zona };
            self.candidatas.insert(candidatas_base, Candidata { nombre: nombre.clone(), forma: caja, activa: None, visible: None, bajo, forzada: desplaza.is_some(), cursor: cursor_del_reparto });
            if let Some(prop) = &desplaza {
                // La rueda sobre él lo corre, sin pasarse de lo que hay. La regla se crea al
                // final, cuando ya se sabe qué formas con nombre son zonas de verdad.
                let visible = if fila { tam.0.clone() } else { tam.1.clone() };
                let hasta = (if fila { contenido.0.clone() } else { contenido.1.clone() } - visible).max(Expr::K(0.0));
                // Y por dónde iba al agarrarlo, para poder arrastrarlo.
                let agarre = self.e.hecho(fijo(&format!("{nombre}.grab")), 0.0);
                if fila {
                    self.fila_de_scroll.insert(nombre.clone());
                }
                self.scrolls.push((nombre.clone(), *prop, hasta, paso.clone(), muelle.unwrap_or(Muelle::RAPIDO), agarre));
            }
            let destino = match self.entornos.last_mut() {
                Some(e) => &mut e.exprs,
                None => &mut self.lets,
            };
            destino.insert(format!("{nombre}.width"), tam.0.clone());
            destino.insert(format!("{nombre}.height"), tam.1.clone());
            destino.insert(format!("{nombre}.count"), cuantos.clone());
            // Con `view:`: cuánto hay de verdad, y por dónde va.
            let largo_del_contenido = if fila { contenido.0.clone() } else { contenido.1.clone() };
            destino.insert(format!("{nombre}.content"), largo_del_contenido);
            if let Some(prop) = &desplaza {
                destino.insert(format!("{nombre}.scroll"), prop.e());
                // Y como propiedad de verdad: una regla puede llevarlo donde quiera
                // (`list.scroll: 0 ~calm`), no solo la rueda.
                self.props.insert(format!("{nombre}.scroll"), *prop);
            }
            // Quien lo leyó antes de este punto leyó la propiedad adelantada: aquí se rellena.
            for (parte, a) in [("width", &tam.0), ("height", &tam.1), ("count", &cuantos)] {
                if let Some(prop) = self.props.get(&format!("{nombre}.{parte}")).copied() {
                    self.e.comportamientos.push(Comportamiento::Es { prop, a: a.clone() });
                }
            }
        }
        self.ultimo_tam = Some(tam);
        Ok(())
    }

    /// Los hijos de un reparto, con cada `repeat` desplegado en sus vueltas.
    fn desplegar(&mut self, entradas: Vec<&'a Entrada>, ambito: &mut Vec<Entorno>, hijos: &mut Vec<(&'a Nodo, Vec<Entorno>)>) -> R<()> {
        for e in entradas {
            let Entrada::Nodo(n) = e else { continue };
            if matches!(n.cabeza.first().map(|f| &f.f), Some(F::Id(p)) if p == "repeat") {
                let mut c = Cur::de(&n.cabeza[1..], n.linea, n.col);
                self.entornos.extend(ambito.iter().cloned());
                let cabeza = self.cabeza_de_repeat(&mut c);
                self.entornos.truncate(self.entornos.len() - ambito.len());
                let (var, desde, hasta) = cabeza?;
                for v in desde..hasta {
                    let mut env = Entorno { sufijo: format!("#{var}{v}"), ..Default::default() };
                    env.exprs.insert(var.clone(), Expr::K(v as f32));
                    ambito.push(env);
                    self.desplegar(n.cuerpo.as_deref().unwrap_or(&[]).iter().collect(), ambito, hijos)?;
                    ambito.pop();
                }
            } else if matches!(n.cabeza.first().map(|f| &f.f), Some(F::Id(p)) if p == "between") {
                // No es un hijo: es lo que va entre ellos. Lo coloca el reparto.
            } else if matches!(n.cabeza.first().map(|f| &f.f), Some(F::Id(p)) if p == "children") {
                let (de_fuera, fuera) = self.coger_hueco(n)?;
                // Cada uno ocupa su sitio en el reparto, y se lee con los nombres de quien lo
                // escribió. Un `repeat` o un `for` de fuera se despliega como los de dentro.
                let mut ambito_de_fuera = vec![Entorno { ambito_de_fuera: Some(fuera.clone()), ..Default::default() }];
                let dentro = std::mem::replace(&mut self.entornos, fuera);
                let r = self.desplegar(de_fuera, &mut ambito_de_fuera, hijos);
                self.entornos = dentro;
                r?;
            } else if matches!(n.cabeza.first().map(|f| &f.f), Some(F::Id(p)) if p == "for") {
                let mut c = Cur::de(&n.cabeza[1..], n.linea, n.col);
                // Con las vueltas de fuera a la vista: `for c in m.children` necesita saber quién es `m`.
                self.entornos.extend(ambito.iter().cloned());
                let cabeza = self.cabeza_de_for(&mut c);
                self.entornos.truncate(self.entornos.len() - ambito.len());
                let (var, modelo, caben, desde) = cabeza?;
                for k in 0..caben {
                    ambito.push(self.vuelta_de_for(&var, &modelo, k, &desde));
                    self.desplegar(n.cuerpo.as_deref().unwrap_or(&[]).iter().collect(), ambito, hijos)?;
                    ambito.pop();
                }
            } else {
                hijos.push((n, ambito.clone()));
            }
        }
        Ok(())
    }

    // ── modelos ─────────────────────────────────────────────────

    /// `model rows max 14 { label: text;  enabled: bool = true;  depth: number }`
    /// El tipo de un hecho: `number`, `bool`, o un enumerado (`low | normal | critical`).
    /// `None` es un número a secas.
    fn tipo_de_hecho(&mut self, c: &mut Cur) -> R<Option<TipoDeHecho>> {
        if let Some(nombres) = self.enumerado(c)? {
            return Ok(Some(TipoDeHecho::Enum(nombres)));
        }
        let o_enumerado = |mut f: Fallo| { f.mensaje.push_str(" Or an enum: `low | normal | critical`."); f };
        Ok(match c.una_de(voz::TIPOS_DE_HECHO, "the type of a fact").map_err(o_enumerado)?.as_str() {
            "number" => None,
            "bool" => Some(TipoDeHecho::Bool),
            _ => unreachable!(),
        })
    }

    /// `low | normal | critical`, si es lo que viene. Sus nombres pasan a valer su posición
    /// en cualquier expresión: `mode == critical`.
    fn enumerado(&mut self, c: &mut Cur) -> R<Option<Vec<String>>> {
        if !matches!((c.mira(), c.f.get(c.i + 1).map(|x| &x.f)), (Some(F::Id(_)), Some(F::Sim("|")))) {
            return Ok(None);
        }
        let mut nombres = vec![c.id("a value")?];
        while c.sim("|") {
            let n = c.id("otro valor del enumerado")?;
            if nombres.contains(&n) {
                c.i -= 1;
                return c.fallo(format!("'{n}' is there twice"));
            }
            nombres.push(n);
        }
        for (k, n) in nombres.iter().enumerate() {
            // El mismo nombre en dos enumerados vale mientras signifique el mismo número.
            // El mismo nombre en dos enumerados vale. Si además es otro número, suelto ya no dice
            // nada: habrá que compararlo con su hecho (`speed == normal`) o escribirlo entero.
            match self.valores.get(n) {
                Some(v) if *v != k as f32 => {
                    self.valores.remove(n);
                    self.ambiguos.insert(n.clone());
                }
                _ if self.ambiguos.contains(n) => {}
                _ => { self.valores.insert(n.clone(), k as f32); }
            }
            if self.hechos.contains_key(n) || self.props.contains_key(n) || self.lets.contains_key(n) {
                return c.fallo(format!("'{n}' is already something else in this scene, and as an enum value it would hide it"));
            }
        }
        Ok(Some(nombres))
    }

    /// `model rows max 14 { label: text;  enabled: bool = true;  list items max 8 { … } }`
    /// `service clock as now { time: text; hour: number }`
    ///
    /// Lo que el sistema cuente rellena `now.time` y `now.hour` solo, sin una línea de
    /// lógica. Qué trae cada servicio está en el vocabulario: pedirle lo que no tiene es
    /// un fallo al cargar, como todo lo demás.
    fn servicio(&mut self, n: &'a Nodo, c: &mut Cur) -> R<()> {
        let cuales: Vec<&str> = voz::SERVICIOS.iter().map(|(x, _)| *x).collect();
        let nombre = c.una_de(&cuales, "a service")?;
        // Sin `as`, sus campos van detrás de su propio nombre: `audio.volume`.
        let alias = if c.palabra("as") { c.id("a name to put before its fields")? } else { nombre.clone() };
        c.nada_mas()?;
        let suyos = voz::SERVICIOS.iter().find(|(x, _)| *x == nombre).map_or(&[][..], |(_, k)| *k);
        let alias = self.declarar(&alias);
        if self.e.servicios.iter().any(|s| s.alias == alias) {
            return Err(Fallo::en(n.linea, n.col, format!("'{alias}' is already the name of another service: give this one another with `as`")));
        }
        let campos = self.campos_de(n, 1)?;
        for k in &campos {
            if !suyos.contains(&k.nombre.as_str()) {
                let pista = parecido(&k.nombre, suyos.iter().map(|s| s.to_string()).collect::<Vec<_>>().iter()).map_or(String::new(), |p| format!(" Did you mean '{p}'?"));
                return Err(Fallo::en(n.linea, n.col, format!("'{nombre}' does not report '{}': it reports {}.{pista}", k.nombre, enumerar(suyos))));
            }
        }
        // `now.time`, no `now.0.time`: de un servicio hay uno, no una lista.
        for campo in &campos {
            let entero = format!("{alias}.{}", campo.nombre);
            match &campo.por_defecto {
                ValorDeCampo::Texto(t) => {
                    let id = self.e.texto_vivo(fijo(&entero), t);
                    if let TipoDeCampo::Imagen(w, h) = campo.tipo {
                        let imagen = self.e.imagen(Fuente::Viva(id), w, h);
                        self.imagenes.insert(entero.clone(), imagen);
                    }
                    self.textos.insert(entero, id);
                }
                ValorDeCampo::Numero(v) => {
                    let id = self.e.hecho(fijo(&entero), *v);
                    match &campo.tipo {
                        TipoDeCampo::Bool => self.e.tipos.push((entero.clone(), TipoDeHecho::Bool)),
                        TipoDeCampo::Enum(x) => self.e.tipos.push((entero.clone(), TipoDeHecho::Enum(x.clone()))),
                        _ => {}
                    }
                    self.hechos.insert(entero, id);
                }
            }
        }
        self.e.servicios.push(crate::escena::Servicio { nombre, alias, campos });
        Ok(())
    }

    fn modelo(&mut self, n: &'a Nodo, c: &mut Cur) -> R<()> {
        let nombre = self.declarar(&c.id("a name for the model")?);
        let caben = if c.palabra("max") { c.num()? as usize } else { 16 };
        c.nada_mas()?;
        let campos = self.campos_de(n, caben)?;
        let modelo = Modelo { nombre: nombre.clone(), caben, campos };
        let mut cuantas = 0;
        self.fichas_de(&nombre, &modelo, &mut cuantas).map_err(|m| Fallo::en(n.linea, n.col, m))?;
        self.e.modelos.push(modelo);
        Ok(())
    }

    /// Los campos de un modelo, o de una lista de dentro de un modelo.
    fn campos_de(&mut self, n: &'a Nodo, caben: usize) -> R<Vec<Campo>> {
        if !(1..=256).contains(&caben) {
            return Err(Fallo::en(n.linea, n.col, "a list holds between 1 and 256 records: each one unfolds when loading"));
        }
        let mut campos: Vec<Campo> = Vec::new();
        let mut recursiva: Option<(String, usize, usize)> = None;
        for e in n.cuerpo.as_deref().unwrap_or(&[]) {
            let campo = match e {
                // `list items max 8 { label: text }`: fichas dentro de la ficha.
                Entrada::Nodo(x) => {
                    let mut c = Cur::de(&x.cabeza, x.linea, x.col);
                    c.una_de(voz::DE_MODELO, "what a model holds besides fields")?;
                    let nombre = c.id("a name for the list")?;
                    let caben = if c.palabra("max") { c.num()? as usize } else { 8 };
                    // `list children max 6 depth 3`, sin bloque: fichas como la de fuera, unas dentro de
                    // otras hasta esa hondura. Un árbol: el menú de una aplicación, con sus submenús.
                    if c.palabra("depth") {
                        let hondura = c.num()? as usize;
                        c.nada_mas()?;
                        if x.cuerpo.is_some() {
                            return Err(Fallo::en(x.linea, x.col, "a list with `depth` carries no block: its records are like the outer one"));
                        }
                        if !(1..=6).contains(&hondura) || !(1..=256).contains(&caben) {
                            return Err(Fallo::en(x.linea, x.col, "`depth` goes from 1 to 6, and `max` from 1 to 256: every level multiplies the records that unfold"));
                        }
                        if recursiva.is_some() {
                            return Err(Fallo::en(x.linea, x.col, "a record has a single list with `depth`"));
                        }
                        recursiva = Some((nombre, caben, hondura));
                        continue;
                    }
                    c.nada_mas()?;
                    let dentro = self.campos_de(x, caben)?;
                    Campo { nombre: nombre.clone(), tipo: TipoDeCampo::Lista(Box::new(Modelo { nombre, caben, campos: dentro })), por_defecto: ValorDeCampo::Numero(0.0) }
                }
                Entrada::Prop { nombre, valor, linea, col } => {
                    let mut c = Cur::de(valor, *linea, *col);
                    let (tipo, por_defecto) = if let Some(nombres) = self.enumerado(&mut c)? {
                        let v = if c.sim("=") {
                            let cual = c.una_de(&nombres.iter().map(String::as_str).collect::<Vec<_>>(), "a value of this field")?;
                            nombres.iter().position(|x| *x == cual).unwrap() as f32
                        } else { 0.0 };
                        (TipoDeCampo::Enum(nombres), ValorDeCampo::Numero(v))
                    } else {
                        let o_enumerado = |mut f: Fallo| { f.mensaje.push_str(" Or an enum: `low | normal | critical`."); f };
                        match c.una_de(voz::TIPOS, "the type of a field").map_err(o_enumerado)?.as_str() {
                            "text" => (TipoDeCampo::Texto, ValorDeCampo::Texto(if c.sim("=") { c.cadena()? } else { String::new() })),
                            "number" => (TipoDeCampo::Numero, ValorDeCampo::Numero(if c.sim("=") { if c.sim("-") { -c.num()? } else { c.num()? } } else { 0.0 })),
                            "bool" => (TipoDeCampo::Bool, ValorDeCampo::Numero(if !c.sim("=") || c.palabra("false") { 0.0 } else if c.palabra("true") { 1.0 } else { return c.fallo("a bool is true or false") })),
                            // `icon: image 24, 24`: el nombre de un icono o una ruta, y la imagen que diga.
                            "image" => {
                                let w = c.num()?;
                                c.exige_sim(",")?;
                                (TipoDeCampo::Imagen(w as u32, c.num()? as u32), ValorDeCampo::Texto(if c.sim("=") { c.cadena()? } else { String::new() }))
                            }
                            _ => unreachable!(),
                        }
                    };
                    c.nada_mas()?;
                    Campo { nombre: nombre.clone(), tipo, por_defecto }
                }
            };
            let (l, col) = match e { Entrada::Nodo(x) => (x.linea, x.col), Entrada::Prop { linea, col, .. } => (*linea, *col) };
            if ["index", "count", "total"].contains(&campo.nombre.as_str()) {
                return Err(Fallo::en(l, col, format!("`{}` already exists: the language provides it", campo.nombre)));
            }
            if campos.iter().any(|k| k.nombre == campo.nombre) {
                return Err(Fallo::en(l, col, format!("field '{}' is there twice", campo.nombre)));
            }
            campos.push(campo);
        }
        if campos.is_empty() {
            return Err(Fallo::en(n.linea, n.col, "this list is missing its fields: `label: text`"));
        }
        // La lista que se contiene a sí misma se desenrolla de dentro afuera: el último nivel
        // ya no tiene hijos; cada uno de los de encima, una lista de los de debajo.
        if let Some((nombre, caben, hondura)) = recursiva {
            if campos.iter().any(|k| k.nombre == nombre) {
                return Err(Fallo::en(n.linea, n.col, format!("field '{nombre}' is there twice")));
            }
            let mut nivel = campos.clone();
            for _ in 0..hondura {
                let mut encima = campos.clone();
                encima.push(Campo { nombre: nombre.clone(), tipo: TipoDeCampo::Lista(Box::new(Modelo { nombre: nombre.clone(), caben, campos: nivel })), por_defecto: ValorDeCampo::Numero(0.0) });
                nivel = encima;
            }
            campos = nivel;
        }
        Ok(campos)
    }

    /// Por dentro, cada campo de cada ficha es un texto o un hecho con nombre:
    /// `rows.3.label`, y si hay listas dentro, `rows.3.items.0.label`.
    fn fichas_de(&mut self, prefijo: &str, m: &Modelo, cuantas: &mut usize) -> Result<(), String> {
        for k in 0..m.caben {
            *cuantas += 1;
            if *cuantas > 4096 {
                return Err("this model unfolds more than 4096 records across its lists: lower some `max`".into());
            }
            for campo in &m.campos {
                let entero = format!("{prefijo}.{k}.{}", campo.nombre);
                if let TipoDeCampo::Lista(dentro) = &campo.tipo {
                    self.fichas_de(&entero, dentro, cuantas)?;
                    continue;
                }
                match &campo.por_defecto {
                    ValorDeCampo::Texto(t) => {
                        let id = self.e.texto_vivo(fijo(&entero), t);
                        if let TipoDeCampo::Imagen(w, h) = campo.tipo {
                            let imagen = self.e.imagen(Fuente::Viva(id), w, h);
                            self.imagenes.insert(entero.clone(), imagen);
                        }
                        self.textos.insert(entero, id);
                    }
                    ValorDeCampo::Numero(v) => {
                        let id = self.e.hecho(fijo(&entero), *v);
                        match &campo.tipo {
                            TipoDeCampo::Bool => self.e.tipos.push((entero.clone(), TipoDeHecho::Bool)),
                            TipoDeCampo::Enum(n) => self.e.tipos.push((entero.clone(), TipoDeHecho::Enum(n.clone()))),
                            _ => {}
                        }
                        self.hechos.insert(entero, id);
                    }
                }
            }
        }
        for parte in ["count", "total"] {
            let entero = format!("{prefijo}.{parte}");
            let id = self.e.hecho(fijo(&entero), 0.0);
            self.hechos.insert(entero, id);
        }
        self.modelos.insert(prefijo.to_owned(), m.caben);
        Ok(())
    }

    /// Si ese nombre es una ficha de un modelo: cómo se llama de verdad, y cuál es.
    fn ficha(&self, n: &str) -> Option<(String, usize)> {
        let g = self.global(n);
        let (modelo, k) = g.rsplit_once('.')?;
        let k: usize = k.parse().ok()?;
        (k < *self.modelos.get(modelo)?).then_some((g, k))
    }

    /// `for r in rows`: el nombre de la ficha, el modelo y cuántas caben.
    fn cabeza_de_for(&mut self, c: &mut Cur) -> R<(String, String, usize, Expr)> {
        let var = c.id("a name for the record")?;
        c.exige_palabra("in")?;
        let modelo = self.global(&c.id("the name of a model")?);
        // `for r in rows from first`: la ficha 0 de lo desplegado es la `first` de la
        // lista de verdad, así que `r.index` cuenta desde ahí. Es lo que hace que una
        // lista de cinco mil quepa en doce copias.
        let desde = if c.palabra("from") { self.expr(c)? } else { Expr::K(0.0) };
        c.nada_mas()?;
        match self.modelos.get(&modelo) {
            Some(caben) => Ok((var, modelo, *caben, desde)),
            None => self.desconocido(c, "no model", &modelo, self.modelos.keys().collect()),
        }
    }

    /// Dentro de la vuelta `k`, `r.label` es `rows.k.label`, `r.index` es `k`, y
    /// todo lo que se dibuje solo existe si la lista llega hasta ahí.
    fn vuelta_de_for(&self, var: &str, modelo: &str, k: usize, desde: &Expr) -> Entorno {
        let mut env = Entorno { sufijo: format!("#{var}{k}"), ..Default::default() };
        env.alias.insert(var.to_owned(), format!("{modelo}.{k}"));
        env.con_partes.insert(var.to_owned());
        env.exprs.insert(format!("{var}.index"), desde.clone() + Expr::K(k as f32));
        env.visible = Some(self.hechos[&format!("{modelo}.count")].e().mayor(Expr::K(k as f32 + 0.5)));
        env
    }

    /// Un `for` suelto, fuera de un reparto: cada vuelta se pinta donde diga, si existe.
    fn para(&mut self, n: &'a Nodo, c: &mut Cur) -> R<()> {
        let (var, modelo, caben, desde) = self.cabeza_de_for(c)?;
        for k in 0..caben {
            let marca = self.reglas.len();
            let env = self.vuelta_de_for(&var, &modelo, k, &desde);
            let esta = env.visible.clone().unwrap();
            self.entornos.push(env);
            let desde = self.candidatas.len();
            self.e.pintar(Instr::Opacidad(Some(esta.clone())));
            self.grupo(n.cuerpo.as_deref().unwrap_or(&[]).iter());
            self.e.pintar(Instr::Opacidad(None));
            for c in &mut self.candidatas[desde..] {
                c.visible = Some(match c.visible.take() { Some(v) => v * esta.clone(), None => esta.clone() });
            }
            self.cerrar_ambito(marca);
        }
        Ok(())
    }

    // ── zonas ───────────────────────────────────────────────────

    /// De las formas con nombre, son zonas las que alguna regla nombra, las
    /// declaradas con `zone` y las que llevan `active`. Las demás tenían nombre
    /// solo para leerse mejor, y no tienen por qué parar el clic. Se crean en el
    /// orden en que se escribieron: la de más abajo en el fichero queda encima.
    fn zonas_de_verdad(&mut self) {
        // Cada regla nombra desde su ámbito: dentro de una copia de un componente,
        // `hit` es la zona de esa copia y no la de otra.
        let mut nombradas = std::collections::HashSet::new();
        let reglas = std::mem::take(&mut self.reglas);
        // (Esto mira todas las palabras de cada regla, también `on` y `press`: no es leer nada.)
        self.sin_vigilar.set(true);
        for (n, entornos) in &reglas {
            self.entornos = entornos.clone();
            for f in &n.cabeza {
                if let F::Id(s) = &f.f {
                    nombradas.insert(self.global(s));
                }
            }
        }
        self.sin_vigilar.set(false);
        self.entornos.clear();
        self.reglas = reglas;
        for k in std::mem::take(&mut self.candidatas) {
            if k.forzada || k.activa.is_some() || nombradas.contains(&k.nombre) {
                // Una zona de algo que no está no para el clic de nadie.
                let activa = match (k.activa, k.visible) {
                    (Some(a), Some(v)) => a * v,
                    (a, v) => a.or(v).unwrap_or(Expr::K(1.0)),
                };
                let z = self.e.zona_bajo(fijo(&k.nombre), k.forma, activa, k.bajo);
                self.e.zonas[z.0 as usize].cursor = k.cursor;
                self.zonas.insert(k.nombre, z);
            }
        }
        // Y las reglas de los repartos que se desplazan, ahora que sus zonas existen.
        let rueda = self.hechos["wheel"].e();
        let (dx, dy) = (self.hechos["drag.dx"].e(), self.hechos["drag.dy"].e());
        for (nombre, prop, hasta, paso, muelle, agarre) in std::mem::take(&mut self.scrolls) {
            let Some(zona) = self.zonas.get(&nombre).copied() else { continue };
            let a = (prop.e() - rueda.clone() * paso).max(Expr::K(0.0)).min(hasta.clone());
            self.e.regla(Disparador::Rueda(zona), vec![Efecto::Animar(Transicion { prop, a, muelle, retraso: Duration::ZERO })]);
            // Arrastrarlo: al pulsar se apunta por dónde iba, y mientras se mueve va de ahí.
            // La zona del reparto está debajo de las de sus hijos, y el arrastre le llega igual.
            let cuanto = if self.fila_de_scroll.contains(&nombre) { dx.clone() } else { dy.clone() };
            self.e.regla(Disparador::Pulsa(zona), vec![Efecto::Hecho(agarre, prop.e())]);
            let a = (agarre.e() - cuanto).max(Expr::K(0.0)).min(hasta);
            self.e.regla(Disparador::Arrastra(zona), vec![Efecto::Animar(Transicion { prop, a, muelle: Muelle::RAPIDO, retraso: Duration::ZERO })]);
        }
    }

    // ── reglas ──────────────────────────────────────────────────

    fn regla(&mut self, n: &Nodo, palabra: &str, c: &mut Cur) -> R<()> {
        let mientras = |o: &Obra, c: &mut Cur| -> R<Expr> { if c.palabra("while") { o.expr(c) } else { Ok(Expr::K(1.0)) } };
        let cuando = if palabra == "every" {
            let a = c.dur()?.as_secs_f32();
            let b = if c.sim("..") { c.dur()?.as_secs_f32() } else { a };
            Disparador::Cada { entre: (a, b), mientras: mientras(self, c)? }
        } else {
            let que = c.id("what has to happen: press, release, scroll, drag, hold, key, submit, focus, blur, drop, enter, leave, hover, away, idle, or an event")?;
            // Solo es un disparador lo que el vocabulario diga; lo demás es el nombre de un suceso.
            match if voz::DISPARADORES.contains(&que.as_str()) { que.as_str() } else { "" } {
                // `on press orb`, o con otro botón: `on press right orb`.
                "press" => {
                    if c.palabra("right") {
                        self.e.superficie_mut().derecho_cierra = false;
                        Disparador::PulsaCon(self.zona(c)?, 1)
                    } else if c.palabra("middle") {
                        Disparador::PulsaCon(self.zona(c)?, 2)
                    } else {
                        Disparador::Pulsa(self.zona(c)?)
                    }
                }
                "release" => Disparador::Suelta(self.zona(c)?),
                "scroll" => Disparador::Rueda(self.zona(c)?),
                "drag" => Disparador::Arrastra(self.zona(c)?),
                "hold" => {
                    let zona = self.zona(c)?;
                    c.exige_palabra("for")?;
                    Disparador::Mantiene { zona, durante: c.dur()? }
                }
                // `on key Escape`, `on key Ctrl+k`: el nombre puede llevar un `+`.
                "key" => {
                    let mut nombre = c.id("the name of a key: Escape, Return, a, Ctrl+k…")?;
                    while c.sim("+") {
                        nombre = format!("{nombre}+{}", c.id("the key")?);
                    }
                    Disparador::Tecla(nombre)
                }
                "submit" => {
                    let n = self.global(&c.id("the name of the input")?);
                    match self.textos.get(&n) {
                        Some(t) => Disparador::Envia(*t),
                        None => return self.desconocido(c, "no text", &n, self.textos.keys().collect()),
                    }
                }
                "focus" => Disparador::GanaFoco,
                "blur" => Disparador::PierdeFoco,
                // `on change floor(list.scroll / 34) { … }`: cuando esa cuenta cambie.
                "change" => Disparador::Cambia(self.expr(c)?),
                // `on still audio.volume for 1.1s { … }`: cuando lleve ese rato igual.
                "still" => {
                    let que = self.expr(c)?;
                    c.exige_palabra("for")?;
                    Disparador::Quieta { que, durante: c.dur()? }
                }
                "drop" => Disparador::Recibe(self.zona(c)?),
                "enter" => Disparador::Entra(self.zona(c)?),
                "leave" => Disparador::Sale(self.zona(c)?),
                "hover" | "away" => {
                    let zona = self.zona(c)?;
                    c.exige_palabra("for")?;
                    let durante = c.dur()?;
                    if que == "hover" { Disparador::Encima { zona, durante } } else { Disparador::Fuera { zona, durante } }
                }
                "idle" => {
                    c.exige_palabra("for")?;
                    let durante = c.dur()?;
                    Disparador::Quieto { durante, mientras: mientras(self, c)? }
                }
                otra if voz::DISPARADORES.contains(&otra) => unreachable!("'{otra}' is in the vocabulary, but `regla` does not handle it"),
                _ => {
                    c.i -= 1;
                    Disparador::Al(self.suceso(c)?)
                }
            }
        };
        // `while` vale en cualquier regla: se mira en el momento de dispararse.
        // (`idle` y `every` ya se lo han quedado: en ellas decide también si el rato cuenta.)
        let si = if c.palabra("while") { Some(self.expr(c)?) } else { None };
        c.nada_mas()?;
        let mut efectos = Vec::new();
        for e in n.cuerpo.as_deref().unwrap_or(&[]) {
            match e {
                Entrada::Prop { nombre, valor, linea, col } => efectos.push(Efecto::Animar(self.transicion(nombre, valor, *linea, *col)?)),
                Entrada::Nodo(x) => {
                    let mut c = Cur::de(&x.cabeza, x.linea, x.col);
                    let p = c.id("an effect")?;
                    efectos.push(match if voz::EFECTOS.contains(&p.as_str()) { p.as_str() } else { "" } {
                        "toggle" => Efecto::Alternar(self.hecho(&mut c)?),
                        "blur" => Efecto::Enfocar(None),
                        "focus" => {
                            let n = self.global(&c.id("the name of the input")?);
                            match self.textos.get(&n) {
                                Some(t) => Efecto::Enfocar(Some(*t)),
                                None => return self.desconocido(&c, "no text", &n, self.textos.keys().collect()),
                            }
                        }
                        // `emit opened` o, con carga, `emit opened(i)`.
                        "emit" => {
                            let s = self.suceso(&mut c)?;
                            let carga = if c.sim("(") {
                                let e = self.expr(&mut c)?;
                                c.exige_sim(")")?;
                                Some(e)
                            } else {
                                None
                            };
                            Efecto::Suceso(s, carga)
                        }
                        "impulse" => Efecto::Impulso(self.prop(&mut c)?, self.expr(&mut c)?),
                        "play" => {
                            let g = self.global(&c.id("the name of a gesture")?);
                            match self.gestos.get(&g) {
                                Some(id) => Efecto::Gesto(*id),
                                None => return self.desconocido(&c, "no gesture", &g, self.gestos.keys().collect()),
                            }
                        }
                        otra if voz::EFECTOS.contains(&otra) => unreachable!("'{otra}' is in the vocabulary, but `regla` cannot carry it out"),
                        _ => {
                            // `open = true`
                            c.i -= 1;
                            let desde = c.i;
                            let h = self.hecho(&mut c)?;
                            let enumerado = self.enumerado_suelto(&c, desde);
                            c.exige_sim("=")?;
                            // `mode = critical`: el valor, de la lista de ese hecho.
                            let valor = match (&enumerado, c.mira()) {
                                (Some((hecho, nombres)), Some(F::Id(v))) if c.f.len() == c.i + 1 && self.es_valor(v) => match nombres.iter().position(|n| n == v) {
                                    Some(k) => {
                                        c.i += 1;
                                        Expr::K(k as f32)
                                    }
                                    None => return c.fallo(format!("'{v}' is not a value of '{hecho}': it can be {}", enumerar(&nombres.iter().map(String::as_str).collect::<Vec<_>>()))),
                                },
                                // Una expresión, que se evalúa al dispararse: `level = clamp(local.x / 64, 0, 1)`.
                                _ => self.expr(&mut c)?,
                            };
                            Efecto::Hecho(h, valor)
                        }
                    });
                    c.nada_mas()?;
                }
            }
        }
        self.e.regla(cuando, efectos);
        if let Some(r) = self.e.reglas.last_mut() {
            r.si = si;
        }
        Ok(())
    }

    // ── lo que el render lleva solo ─────────────────────────────

    fn comportamiento(&mut self, palabra: &str, c: &mut Cur) -> R<()> {
        let comp = match palabra {
            // blink eyelid every 2.4s..6s for 170ms · blink lid every 5.2s for 120ms
            "blink" => {
                let prop = self.prop(c)?;
                c.exige_palabra("every")?;
                let a = c.dur()?.as_secs_f32();
                // Sin `..`, siempre el mismo rato. El periodo se cuenta de comienzo a
                // comienzo: «cada 5,2 s» es cada 5,2 s, no 5,2 s después de cerrar.
                let b = if c.sim("..") { c.dur()?.as_secs_f32() } else { a };
                c.exige_palabra("for")?;
                Comportamiento::Parpadeo { prop, cada: (a, b), dura: c.dur()?.as_secs_f32() }
            }
            // wave breath = asleep * 1.3 at 1.7
            "wave" => {
                let prop = self.prop(c)?;
                c.exige_sim("=")?;
                let amplitud = self.expr(c)?;
                c.exige_palabra("at")?;
                Comportamiento::Onda { prop, frecuencia: c.num()?, amplitud }
            }
            // spin angle by 0.9
            "spin" => {
                let prop = self.prop(c)?;
                c.exige_palabra("by")?;
                Comportamiento::Avance { prop, por_segundo: self.expr(c)? }
            }
            // follow chip.w = label.width + 32
            "follow" => {
                let prop = self.prop(c)?;
                c.exige_sim("=")?;
                Comportamiento::Sigue { prop, a: self.expr(c)? }
            }
            // look gaze.x, gaze.y at orb.x, orb.y reach 5, 3.2 within 140 rest 3.2, 0.6
            _ => {
                let x = self.prop(c)?;
                c.exige_sim(",")?;
                let y = self.prop(c)?;
                c.exige_palabra("at")?;
                let centro = self.punto(c)?;
                c.exige_palabra("reach")?;
                let rx = c.num()?;
                c.exige_sim(",")?;
                let ry = c.num()?;
                c.exige_palabra("within")?;
                let distancia = c.num()?;
                let reposo = if c.palabra("rest") { self.punto(c)? } else { (0.0.into(), 0.0.into()) };
                Comportamiento::Mirada { x, y, centro, alcance: (rx, ry), distancia, reposo }
            }
        };
        c.nada_mas()?;
        self.e.comportamientos.push(comp);
        Ok(())
    }

    // ── gestos ──────────────────────────────────────────────────

    /// `gesture nod reflex { 130ms out_quad { look.y: 4; eyes: 10 } … }`
    fn gesto(&mut self, n: &Nodo, palabra: &str, c: &mut Cur) -> R<()> {
        let nombre = self.declarar(&c.id("a name for the gesture")?);
        let (clase, mientras) = if palabra == "posture" {
            c.exige_palabra("while")?;
            (Clase::Postura, Some(self.expr(c)?))
        } else {
            let k = match c.una_de(voz::CLASES, "the class of a gesture")?.as_str() {
                "ambient" => Clase::Ambiente,
                "reflex" => Clase::Reflejo,
                "asked" => Clase::Pedido,
                "state" => Clase::Estado,
                _ => unreachable!(),
            };
            (k, None)
        };
        c.nada_mas()?;
        let mut fotogramas = Vec::new();
        for e in n.cuerpo.as_deref().unwrap_or(&[]) {
            let Entrada::Nodo(f) = e else {
                return Err(Fallo::en(n.linea, n.col, "a gesture is made of frames: `130ms out_quad { eyes: 10 }`"));
            };
            let mut c = Cur::de(&f.cabeza, f.linea, f.col);
            let ms = (c.dur()?.as_secs_f32() * 1000.0) as u32;
            let mut foto = foto(ms, Curva::InOutSine);
            while !c.acabo() {
                let de_fotograma: Vec<&str> = voz::CURVAS.iter().chain(voz::DE_FOTOGRAMA).copied().collect();
                let p = c.una_de(&de_fotograma, "what a frame carries")?;
                match p.as_str() {
                    "hold" => foto.aguanta = (c.dur()?.as_secs_f32() * 1000.0) as u32,
                    "emit" => foto.emite = Some(self.suceso(&mut c)?),
                    "linear" => foto.curva = Curva::Lineal,
                    "in_quad" => foto.curva = Curva::InQuad,
                    "out_quad" => foto.curva = Curva::OutQuad,
                    "in_cubic" => foto.curva = Curva::InCubic,
                    "out_cubic" => foto.curva = Curva::OutCubic,
                    "in_out_sine" => foto.curva = Curva::InOutSine,
                    "out_back" => foto.curva = Curva::OutBack,
                    _ => unreachable!(),
                }
            }
            for v in f.cuerpo.as_deref().unwrap_or(&[]) {
                let Entrada::Prop { nombre, valor, linea, col } = v else { continue };
                let nombre = &self.global(nombre);
                let Some(prop) = self.props.get(nombre).copied() else {
                    let pista = parecido(nombre, self.props.keys()).map_or(String::new(), |p| format!(" Did you mean '{p}'?"));
                    return Err(Fallo::en(*linea, *col, format!("there is no property called '{nombre}'.{pista}")));
                };
                if !self.e.pose.contains(&prop) {
                    return Err(Fallo::en(*linea, *col, format!("'{nombre}' is not part of the pose: a gesture only leads by the hand what was declared with `pose`")));
                }
                let mut c = Cur::de(valor, *linea, *col);
                foto.valores.push((prop, self.expr(&mut c)?));
                c.nada_mas()?;
            }
            fotogramas.push(foto);
        }
        if fotogramas.is_empty() {
            return Err(Fallo::en(n.linea, n.col, "a gesture with no frames does nothing"));
        }
        let g = self.e.gesto(fijo(&nombre), clase, fotogramas);
        if let Some(m) = mientras {
            self.e.postura(g, m);
        }
        self.gestos.insert(nombre, g);
        Ok(())
    }
}

/// «a, b o c»
fn enumerar(lista: &[&str]) -> String {
    match lista {
        [] => String::new(),
        [una] => (*una).to_owned(),
        [antes @ .., ultima] => format!("{} or {ultima}", antes.join(", ")),
    }
}

fn nombre_de_tipo(t: &str) -> &'static str {
    match t {
        "number" => "a number",
        "color" => "a colour",
        "text" => "a text",
        "record" => "a record of a model",
        "event" => "an event",
        "image" => "an image",
        "bool" => "a yes or no",
        "spring" => "a spring",
        "gesture" => "a gesture",
        _ => "algo",
    }
}

fn leer_cursor(c: &mut Cur) -> R<Cursor> {
    Ok(match c.una_de(voz::CURSORES, "a cursor")?.as_str() {
        "default" => Cursor::Normal,
        "pointer" => Cursor::Mano,
        "text" => Cursor::Texto,
        "grab" => Cursor::Agarrar,
        "grabbing" => Cursor::Agarrando,
        _ => unreachable!(),
    })
}

struct FormaLeida {
    /// Cuánto ocupa, si se sabe: lo que necesita un `row` para repartir.
    tam: Option<(Expr, Expr)>,
    forma: Forma,
    color: Option<Color>,
    opacidad: Option<Expr>,
    fusion: Option<Expr>,
}
