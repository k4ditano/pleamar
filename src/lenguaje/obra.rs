//! Del árbol a la escena: qué significa cada nodo, y si los nombres existen.
//!
//! Se lee en cuatro vueltas, para que el orden en que se escribe sea el que le
//! convenga a quien lee y no al programa: primero lo que se declara; luego los
//! nombres (`let`) y las capas; luego el dibujo, y al final las reglas, que ya
//! pueden nombrar cualquier forma. Solo un `let` tiene que ir antes de quien lo usa.

use super::arbol::{Entrada, Nodo};
use super::fichas::{Ficha, F};
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
        self.f.get(self.i).map(|x| &x.f)
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
        if self.sim(s) { Ok(()) } else { self.fallo(format!("aquí esperaba «{s}»")) }
    }
    fn exige_palabra(&mut self, p: &str) -> R<()> {
        if self.palabra(p) { Ok(()) } else { self.fallo(format!("aquí esperaba «{p}»")) }
    }
    fn id(&mut self, que: &str) -> R<String> {
        match self.mira() {
            Some(F::Id(x)) => {
                self.i += 1;
                Ok(x.clone())
            }
            _ => self.fallo(format!("aquí esperaba {que}")),
        }
    }
    fn num(&mut self) -> R<f32> {
        let menos = self.sim("-");
        match self.mira() {
            Some(F::Num(n)) => {
                self.i += 1;
                Ok(if menos { -n } else { *n })
            }
            _ => self.fallo("aquí esperaba un número"),
        }
    }
    fn dur(&mut self) -> R<Duration> {
        match self.mira() {
            Some(F::Dur(s)) => {
                self.i += 1;
                Ok(Duration::from_secs_f32(*s))
            }
            _ => self.fallo("aquí esperaba una duración, como 320ms o 14s"),
        }
    }
    fn cadena(&mut self) -> R<String> {
        match self.mira() {
            Some(F::Cadena(s)) => {
                self.i += 1;
                Ok(s.clone())
            }
            _ => self.fallo("aquí esperaba un texto entre comillas"),
        }
    }
    fn nada_mas(&self) -> R<()> {
        if self.acabo() { Ok(()) } else { self.fallo("esto sobra") }
    }
}

/// Una forma con nombre: será zona si alguna regla la nombra, si se declaró
/// con `zone` o si lleva `active`.
struct Candidata {
    nombre: String,
    forma: Forma,
    activa: Option<Expr>,
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
    alias: HashMap<String, String>,
    /// De esos nombres, los que tienen partes: `label.width` es de la medida `label`.
    con_partes: std::collections::HashSet<String>,
    sufijo: String,
}

struct Componente<'a> {
    parametros: Vec<String>,
    nodo: &'a Nodo,
}

struct Obra<'a> {
    e: Escena,
    props: HashMap<String, PropId>,
    hechos: HashMap<String, HechoId>,
    sucesos: HashMap<String, SucesoId>,
    textos: HashMap<String, TextoId>,
    imagenes: HashMap<String, ImagenId>,
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

pub fn levantar(arbol: &[Entrada]) -> Result<Escena, Vec<Fallo>> {
    let escena = match arbol {
        [Entrada::Nodo(n)] if matches!(n.cabeza.first().map(|f| &f.f), Some(F::Id(p)) if p == "scene") => n,
        _ => return Err(vec![Fallo::en(1, 1, "un fichero es una escena: `scene Nombre { … }`")]),
    };
    let mut o = Obra {
        e: Escena::default(),
        props: HashMap::new(), hechos: HashMap::new(), sucesos: HashMap::new(), textos: HashMap::new(), imagenes: HashMap::new(),
        medidas: HashMap::new(), gestos: HashMap::new(), zonas: HashMap::new(), lets: HashMap::new(), colores: HashMap::new(),
        muelles: [("lively", Muelle::VIVO), ("calm", Muelle::SERENO), ("quick", Muelle::RAPIDO), ("slow", Muelle::LENTO), ("eyes", Muelle::OJOS), ("pose", Muelle::POSE)]
            .into_iter().map(|(n, m)| (n.to_owned(), m)).collect(),
        bajo: Vec::new(), candidatas: Vec::new(), reglas: Vec::new(), fallos: Vec::new(),
        entornos: Vec::new(), componentes: HashMap::new(), copias: 0, en_hueco: false, ultimo_tam: None, medida_impuesta: None, teclado_pendiente: None,
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
        return Err(vec![Fallo::en(escena.linea, escena.col, "a la escena le falta su bloque `{ … }`")]);
    };
    // Cuatro vueltas: declaraciones; nombres y capas; dibujo; reglas.
    for vuelta in 0..3 {
        let de_esta: Vec<&Entrada> = cuerpo.iter().filter(|e| vuelta_de(e) == vuelta).collect();
        o.grupo(de_esta.into_iter());
    }
    o.zonas_de_verdad();
    for (n, entornos) in std::mem::take(&mut o.reglas) {
        o.entornos = entornos;
        let mut c = Cur::de(&n.cabeza, n.linea, n.col);
        let palabra = c.id("on o every").unwrap_or_default();
        if let Err(f) = o.regla(n, &palabra, &mut c) {
            if o.fallos.len() < 8 {
                o.fallos.push(f);
            }
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
    let (w, h) = (o.e.superficie.ancho as f32, o.e.superficie.alto as f32);
    o.e.hechos[0].1 = if w > 0.0 { w } else { 1920.0 };
    o.e.hechos[1].1 = h;
    if o.fallos.is_empty() { Ok(o.e) } else { Err(o.fallos) }
}

/// En qué vuelta se lee cada sentencia del nivel de la escena.
fn vuelta_de(e: &Entrada) -> u8 {
    let Entrada::Nodo(n) = e else { return 2 };
    let es_asignacion = matches!(n.cabeza.get(2).map(|x| &x.f), Some(F::Sim("=")));
    match n.cabeza.first().map(|f| &f.f) {
        Some(F::Id(p)) => match p.as_str() {
            "surface" | "spring" | "prop" | "pose" | "fact" | "event" | "measure" | "component" => 0,
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
        n
    }

    /// El nombre con el que se declara algo desde aquí dentro.
    fn declarar(&mut self, local: &str) -> String {
        let interpolado = self.interpolar(local);
        match self.entornos.last_mut() {
            Some(e) if !e.sufijo.is_empty() && !local.contains('$') => {
                let g = format!("{interpolado}{}", e.sufijo);
                e.alias.insert(interpolado, g.clone());
                g
            }
            _ => interpolado,
        }
    }

    fn desconocido<T>(&self, c: &Cur, que: &str, nombre: &str, conocidos: Vec<&String>) -> R<T> {
        let pista = parecido(nombre, conocidos.into_iter()).map_or(String::new(), |p| format!(" ¿Querías decir «{p}»?"));
        let (l, col) = c.f.get(c.i.saturating_sub(1)).map_or(c.fin, |x| (x.linea, x.col));
        Err(Fallo::en(l, col, format!("no hay {que} que se llame «{nombre}».{pista} Un `let` tiene que ir antes de quien lo usa; lo demás, donde quieras.")))
    }

    fn prop(&self, c: &mut Cur) -> R<PropId> {
        let n = self.global(&c.id("el nombre de una propiedad")?);
        match self.props.get(&n) {
            Some(p) => Ok(*p),
            None => self.desconocido(c, "ninguna propiedad", &n, self.props.keys().collect()),
        }
    }
    fn hecho(&self, c: &mut Cur) -> R<HechoId> {
        let n = self.global(&c.id("el nombre de un hecho")?);
        match self.hechos.get(&n) {
            Some(h) => Ok(*h),
            None => self.desconocido(c, "ningún hecho", &n, self.hechos.keys().collect()),
        }
    }
    fn suceso(&self, c: &mut Cur) -> R<SucesoId> {
        let n = self.global(&c.id("el nombre de un suceso")?);
        match self.sucesos.get(&n) {
            Some(s) => Ok(*s),
            None => self.desconocido(c, "ningún suceso", &n, self.sucesos.keys().collect()),
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
        let n = self.global(&c.id("el nombre de una forma o de una zona")?);
        match self.zonas.get(&n) {
            Some(z) => Ok(*z),
            None => self.desconocido(c, "ninguna forma con nombre ni zona", &n, self.zonas.keys().collect()),
        }
    }
    fn muelle(&self, c: &mut Cur) -> R<Muelle> {
        let n = c.id("el nombre de un muelle")?;
        if n == "spring" {
            c.exige_sim("(")?;
            let rigidez = c.num()?;
            c.exige_sim(",")?;
            let freno = c.num()?;
            c.exige_sim(")")?;
            return Ok(Muelle { rigidez, freno });
        }
        match self.muelles.get(&n) {
            Some(m) => Ok(*m),
            None => self.desconocido(c, "ningún muelle", &n, self.muelles.keys().collect()),
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
        let a = self.expr_suma(c)?;
        for (s, f) in [(">=", 0), ("<=", 1), (">", 2), ("<", 3)] {
            if c.sim(s) {
                let b = self.expr_suma(c)?;
                return Ok(match f {
                    0 => b.mayor(a).no(),
                    1 => a.mayor(b).no(),
                    2 => a.mayor(b),
                    _ => b.mayor(a),
                });
            }
        }
        Ok(a)
    }
    fn expr_suma(&self, c: &mut Cur) -> R<Expr> {
        let mut a = self.expr_prod(c)?;
        loop {
            if c.sim("+") {
                a = a + self.expr_prod(c)?;
            } else if c.sim("-") {
                a = a - self.expr_prod(c)?;
            } else {
                return Ok(a);
            }
        }
    }
    fn expr_prod(&self, c: &mut Cur) -> R<Expr> {
        let mut a = self.expr_uno(c)?;
        loop {
            if c.sim("*") {
                a = a * self.expr_uno(c)?;
            } else if c.sim("/") {
                a = a / self.expr_uno(c)?;
            } else {
                return Ok(a);
            }
        }
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
                } else {
                    let conocidos: Vec<&String> = self.lets.keys().chain(self.props.keys()).chain(self.hechos.keys()).collect();
                    self.desconocido(c, "nada", n, conocidos)
                }
            }
            _ => c.fallo("aquí esperaba un número, un nombre o un paréntesis"),
        }
    }
    fn funcion(&self, nombre: &str, c: &mut Cur) -> R<Expr> {
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
            _ => Err(Fallo::en(l, col, format!("en «{nombre}», los dos primeros tienen que ser números"))),
        };
        let mut a = a.into_iter();
        let mut toma = || a.next().ok_or_else(|| Fallo::en(l, col, format!("a «{nombre}» le faltan argumentos")));
        Ok(match nombre {
            "min" => toma()?.min(toma()?),
            "max" => toma()?.max(toma()?),
            "abs" => toma()?.abs(),
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
            otra => return Err(Fallo::en(l, col, format!("no conozco la función «{otra}»: hay min, max, abs, clamp, smooth, mix, if y vel"))),
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
            _ => c.fallo("aquí esperaba un color: #151616, el nombre de uno, o mix(#a, #b, cuánto)"),
        }
    }

    // ── el bloque de un nodo, visto como propiedades ────────────

    fn propiedades<'n>(&self, n: &'n Nodo, validas: &[&str]) -> R<HashMap<&'n str, Cur<'n>>> {
        let mut m = HashMap::new();
        for e in n.cuerpo.as_deref().unwrap_or(&[]) {
            if let Entrada::Prop { nombre, valor, linea, col } = e {
                if !validas.contains(&nombre.as_str()) {
                    let v: Vec<String> = validas.iter().map(|s| s.to_string()).collect();
                    let pista = parecido(nombre, v.iter()).map_or(String::new(), |p| format!(" ¿Querías decir «{p}»?"));
                    return Err(Fallo::en(*linea, *col, format!("aquí no existe «{nombre}».{pista} Valen: {}", validas.join(", "))));
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
        let clase = c.id("una forma: ellipse, box, arc o line")?;
        let nombre = if c.acabo() { None } else { Some(c.id("un nombre para la forma")?) };
        c.nada_mas()?;
        let comunes = ["rotate", "stroke", "color", "opacity", "blend", "active", "show", "cursor"];
        let en_hueco = std::mem::take(&mut self.en_hueco);
        let propias: &[&str] = match clase.as_str() {
            "ellipse" => &["at", "radius", "scale"],
            "box" => &["at", "from", "size", "corner"],
            "arc" => &["at", "radius", "span", "width"],
            "line" => &["from", "to", "width"],
            otra => return Err(Fallo::en(n.linea, n.col, format!("no conozco la forma «{otra}»: hay ellipse, box, arc y line"))),
        };
        let validas: Vec<&str> = propias.iter().chain(comunes.iter()).copied().collect();
        let mut p = self.propiedades(n, &validas)?;
        let falta = |que: &str| Fallo::en(n.linea, n.col, format!("a este «{clase}» le falta «{que}»"));
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
                    _ => return Err(Fallo::en(n.linea, n.col, "una caja se coloca con «at» (su centro) o con «from» (su esquina), una de las dos")),
                };
                Forma::Caja { centro, mitad: (w * 0.5, h * 0.5), radio: corner.unwrap_or(Expr::K(0.0)) }
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
            self.candidatas.push(Candidata { nombre: nombre.clone(), forma: forma.clone(), activa: active, bajo: self.bajo.clone(), forzada: false, cursor });
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
                if self.fallos.len() < 8 {
                    self.fallos.push(f);
                }
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
            let palabra = c.id("una declaración")?;
            match palabra.as_str() {
                "surface" => self.superficie(n)?,
                "spring" => {
                    let nombre = c.id("un nombre para el muelle")?;
                    c.exige_sim("=")?;
                    let rigidez = c.num()?;
                    c.exige_sim(",")?;
                    self.muelles.insert(nombre, Muelle { rigidez, freno: c.num()? });
                }
                "prop" | "pose" => {
                    let nombre = self.declarar(&c.id("un nombre para la propiedad")?);
                    c.exige_sim("=")?;
                    let v = c.num()?;
                    let muelle = if c.sim("~") { self.muelle(&mut c)? } else if palabra == "pose" { Muelle::POSE } else { Muelle::VIVO };
                    c.nada_mas()?;
                    let p = self.e.prop_con(fijo(&nombre), v, muelle);
                    if palabra == "pose" {
                        self.e.pose.push(p);
                    }
                    self.props.insert(nombre, p);
                }
                "fact" => {
                    let nombre = self.declarar(&c.id("un nombre para el hecho")?);
                    c.exige_sim("=")?;
                    let v = if c.palabra("true") { 1.0 } else if c.palabra("false") { 0.0 } else { c.num()? };
                    c.nada_mas()?;
                    let h = self.e.hecho(fijo(&nombre), v);
                    self.hechos.insert(nombre, h);
                }
                "event" => {
                    let nombre = self.declarar(&c.id("un nombre para el suceso")?);
                    let s = if c.sim("->") { self.e.suceso_que_sale(fijo(&nombre)) } else { self.e.suceso(fijo(&nombre)) };
                    c.nada_mas()?;
                    self.sucesos.insert(nombre, s);
                }
                "text" if matches!(c.f.get(2).map(|x| &x.f), Some(F::Sim("="))) => {
                    let nombre = self.declarar(&c.id("un nombre para el texto")?);
                    c.exige_sim("=")?;
                    let t = self.e.texto_vivo(fijo(&nombre), &c.cadena()?);
                    self.textos.insert(nombre, t);
                }
                "image" if matches!(c.f.get(2).map(|x| &x.f), Some(F::Sim("="))) => {
                    let nombre = self.declarar(&c.id("un nombre para la imagen")?);
                    c.exige_sim("=")?;
                    let fuente = if c.palabra("icon") { Fuente::Icono(c.cadena()?) } else if c.palabra("file") { Fuente::Ruta(c.cadena()?.into()) } else { return c.fallo("una imagen es `icon \"nombre\"` o `file \"ruta\"`") };
                    c.exige_sim(",")?;
                    let w = c.num()?;
                    c.exige_sim(",")?;
                    let i = self.e.imagen(fuente, w as u32, c.num()? as u32);
                    self.imagenes.insert(nombre, i);
                }
                "measure" => {
                    let local = c.id("un nombre para la medida")?;
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
                    let nombre = c.id("un nombre")?;
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
                "ellipse" | "box" | "arc" | "line" => {
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
                "component" => self.declarar_componente(n, &mut c)?,
                "repeat" => self.repetir(n, &mut c)?,
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
                otra => {
                    let validas: Vec<String> = ["surface", "prop", "pose", "fact", "event", "text", "image", "measure", "let", "spring", "body", "ellipse", "box", "arc", "line", "input", "zone", "clip", "group", "row", "column", "repeat", "component", "layer", "on", "every", "blink", "wave", "spin", "follow", "look", "gesture", "posture"].iter().map(|s| s.to_string()).collect();
                    let pista = parecido(otra, validas.iter()).map_or(String::new(), |p| format!(" ¿Querías decir «{p}»?"));
                    return Err(Fallo::en(n.linea, n.col, format!("no sé qué es «{otra}».{pista}")));
                }
            }
        }
        Ok(())
    }

    /// `n` trae las propiedades; `cuerpo`, los hijos: los del propio grupo, o los
    /// de un componente cuando `n` es una de sus copias.
    fn grupo_con_propiedades(&mut self, n: &Nodo, cuerpo: &'a [Entrada]) -> R<()> {
        let mut p = self.propiedades(n, &["pivot", "rotate", "scale", "move", "opacity", "size", "show"])?;
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

    /// `body { color: …; shadow: …; ellipse {…}; box {… blend: …} }`
    fn cuerpo(&mut self, n: &Nodo) -> R<()> {
        let mut p = self.propiedades(n, &["color", "gradient", "rim", "light", "shadow", "border", "opacity", "show"])?;
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
            return Err(Fallo::en(n.linea, n.col, "un «body» sin formas no pinta nada"));
        }
        let pintura = if let Some(c) = p.get_mut("gradient") {
            let de = self.punto(c)?;
            c.exige_sim(",")?;
            let a = self.punto(c)?;
            c.exige_sim(",")?;
            let c0 = self.color(c)?;
            c.exige_sim(",")?;
            Pintura::Lineal { de, a, c0, c1: self.color(c)? }
        } else if let Some(c) = p.get_mut("color") {
            Pintura::Color(self.color(c)?)
        } else {
            return Err(Fallo::en(n.linea, n.col, "a este «body» le falta «color» o «gradient»"));
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

    /// `text notice.title { at: …; size: 20 }` o `text "Descartar" { … }`
    fn texto(&mut self, n: &Nodo) -> R<()> {
        let mut c = Cur::de(&n.cabeza[1..], n.linea, n.col);
        let contenido = match c.mira() {
            Some(F::Cadena(s)) => Contenido::Fijo(s.clone()),
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
            Some(F::Id(nombre)) if self.entornos.iter().any(|e| e.cadenas.contains_key(nombre)) => {
                Contenido::Fijo(self.entornos.iter().rev().find_map(|e| e.cadenas.get(nombre)).unwrap().clone())
            }
            Some(F::Id(nombre)) => match self.textos.get(&self.global(nombre)) {
                Some(t) => Contenido::Vivo(*t),
                None => {
                    c.i += 1;
                    return self.desconocido(&c, "ningún texto", nombre, self.textos.keys().collect());
                }
            },
            _ => return c.fallo("un texto es `text \"literal\" { … }` o `text nombre { … }`"),
        };
        let mut p = self.propiedades(n, &["at", "anchor", "width", "size", "weight", "color", "opacity", "lines", "align", "line_height", "family", "measure", "show"])?;
        let en_hueco = std::mem::take(&mut self.en_hueco);
        let en = match p.get_mut("at") {
            Some(c) => self.punto(c)?,
            None if en_hueco => (0.0.into(), 0.0.into()),
            None => return Err(Fallo::en(n.linea, n.col, "a este texto le falta «at»")),
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
            estilo.alineado = match c.id("left, center o right")?.as_str() {
                "left" => Alineado::Izquierda,
                "center" => Alineado::Centro,
                "right" => Alineado::Derecha,
                _ => return c.fallo("se alinea a left, center o right"),
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
                    _ => return c.fallo("un ancla es left, center o right, y top, center o bottom"),
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
                let nombre = self.global(&c.id("el nombre de una medida")?);
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
        let local = c.id("el nombre del texto que edita")?;
        let nombre = self.global(&local);
        let Some(texto) = self.textos.get(&nombre).copied() else {
            return self.desconocido(&c, "ningún texto", &nombre, self.textos.keys().collect());
        };
        let mut p = self.propiedades(n, &["at", "width", "size", "weight", "color", "opacity", "family", "placeholder", "selection", "show"])?;
        let en_hueco = std::mem::take(&mut self.en_hueco);
        let en = match p.get_mut("at") {
            Some(c) => self.punto(c)?,
            None if en_hueco => (0.0.into(), 0.0.into()),
            None => return Err(Fallo::en(n.linea, n.col, "a este campo le falta «at»")),
        };
        let ancho = match p.get_mut("width") {
            Some(c) => self.expr(c)?,
            None => return Err(Fallo::en(n.linea, n.col, "a este campo le falta «width»")),
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
            activa: None, bajo: self.bajo.clone(), forzada: true, cursor: Cursor::Texto,
        });
        self.ultimo_tam = Some((ancho.clone(), alto.into()));
        self.e.pintar(Instr::Campo { texto, zona: fijo(&zona), en, ancho, estilo, alfa, marcador, seleccion });
        Ok(())
    }

    fn imagen(&mut self, n: &Nodo) -> R<()> {
        let mut c = Cur::de(&n.cabeza[1..], n.linea, n.col);
        let nombre = self.global(&c.id("el nombre de una imagen")?);
        let Some(imagen) = self.imagenes.get(&nombre).copied() else {
            return self.desconocido(&c, "ninguna imagen", &nombre, self.imagenes.keys().collect());
        };
        let mut p = self.propiedades(n, &["at", "size", "opacity", "tint", "show"])?;
        let en_hueco = std::mem::take(&mut self.en_hueco);
        let falta = |q: &str| Fallo::en(n.linea, n.col, format!("a esta imagen le falta «{q}»"));
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

    fn superficie(&mut self, n: &'a Nodo) -> R<()> {
        let mut p = self.propiedades(n, &["size", "anchor", "margin", "level", "reserve", "screens", "keyboard"])?;
        let s = &mut self.e.superficie;
        if let Some(c) = p.get_mut("size") {
            // `size: full, 36`: todo el ancho del monitor.
            s.ancho = if c.palabra("full") { 0 } else { c.num()? as u32 };
            c.exige_sim(",")?;
            s.alto = c.num()? as u32;
        }
        if let Some(c) = p.get_mut("anchor") {
            s.ancla = match c.id("dónde anclarla")?.as_str() {
                "top" => Ancla::Arriba,
                "bottom" => Ancla::Abajo,
                "left" => Ancla::Izquierda,
                "right" => Ancla::Derecha,
                "top_left" => Ancla::ArribaIzquierda,
                "top_right" => Ancla::ArribaDerecha,
                "bottom_left" => Ancla::AbajoIzquierda,
                "bottom_right" => Ancla::AbajoDerecha,
                "center" => Ancla::Centro,
                _ => return c.fallo("se ancla a top, bottom, left, right, top_left, top_right, bottom_left, bottom_right o center"),
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
            s.nivel = match c.id("un nivel")?.as_str() {
                "background" => Nivel::Fondo,
                "bottom" => Nivel::Debajo,
                "top" => Nivel::Encima,
                "overlay" => Nivel::SobreTodo,
                _ => return c.fallo("los niveles son background, bottom, top y overlay"),
            };
        }
        if let Some(c) = p.get_mut("keyboard") {
            s.teclado = match c.id("none, on_demand o exclusive")?.as_str() {
                "none" => Teclado::Nunca,
                "on_demand" => Teclado::AlPulsar,
                "exclusive" => Teclado::Siempre,
                _ => return c.fallo("el teclado se pide con none, on_demand (al pulsar) o exclusive (todo para ella)"),
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
            } else {
                let mut v = vec![c.cadena()?];
                while c.sim(",") {
                    v.push(c.cadena()?);
                }
                Pantallas::Estas(v)
            };
        }
        Ok(())
    }

    // ── capas ───────────────────────────────────────────────────

    /// `layer card ~calm { open while open { orb.x: 140 ~lively after 70ms } rest { … } }`
    fn capa(&mut self, n: &Nodo, c: &mut Cur) -> R<()> {
        let nombre = c.id("un nombre para la capa")?;
        let muelle = if c.sim("~") { self.muelle(c)? } else { Muelle::RAPIDO };
        c.nada_mas()?;
        let mut reclamaciones = Vec::new();
        let mut nombres = Vec::new();
        for e in n.cuerpo.as_deref().unwrap_or(&[]) {
            let Entrada::Nodo(r) = e else {
                return Err(Fallo::en(n.linea, n.col, "dentro de una capa solo hay reclamaciones: `nombre while …`, `nombre for 700ms after …`, `nombre { … }`"));
            };
            let mut c = Cur::de(&r.cabeza, r.linea, r.col);
            let quien = c.id("un nombre para la reclamación")?;
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
                    Entrada::Nodo(x) => return Err(Fallo::en(x.linea, x.col, "aquí van propiedades y su destino: `orb.x: 140 ~lively after 70ms`")),
                }
            }
            nombres.push(quien.clone());
            reclamaciones.push(Reclamacion { nombre: fijo(&quien), cuando, fija });
        }
        if reclamaciones.is_empty() {
            return Err(Fallo::en(n.linea, n.col, "una capa sin reclamaciones no decide nada"));
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
        let nombre = &self.global(nombre);
        let Some(prop) = self.props.get(nombre).copied() else {
            let pista = parecido(nombre, self.props.keys()).map_or(String::new(), |p| format!(" ¿Querías decir «{p}»?"));
            return Err(Fallo::en(linea, col, format!("no hay ninguna propiedad que se llame «{nombre}».{pista}")));
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
        let nombre = c.id("un nombre para el componente")?;
        let mut parametros = Vec::new();
        if c.sim("(") && !c.sim(")") {
            loop {
                parametros.push(c.id("el nombre de un parámetro")?);
                if c.sim(")") {
                    break;
                }
                c.exige_sim(",")?;
            }
        }
        c.nada_mas()?;
        if n.cuerpo.is_none() {
            return Err(Fallo::en(n.linea, n.col, "a este componente le falta su bloque `{ … }`"));
        }
        self.componentes.insert(nombre, Componente { parametros, nodo: n });
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
        if c.sim("(") {
            for (k, p) in parametros.iter().enumerate() {
                if k > 0 {
                    c.exige_sim(",")?;
                }
                let sigue = c.f.get(c.i + 1).map(|x| &x.f);
                let solo = matches!(sigue, Some(F::Sim(",")) | Some(F::Sim(")")));
                match c.mira() {
                    Some(F::Cadena(t)) => {
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
                    // El nombre de un texto, una imagen o un gesto: el parámetro es otro nombre para él.
                    Some(F::Id(x)) if solo && { let g = self.global(x); self.textos.contains_key(&g) || self.imagenes.contains_key(&g) || self.gestos.contains_key(&g) } => {
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
            }
            if !c.sim(")") {
                return c.fallo(format!("«{nombre}» tiene {} parámetros: {}", parametros.len(), parametros.join(", ")));
            }
        } else if !parametros.is_empty() {
            return c.fallo(format!("«{nombre}» pide parámetros: {nombre}({})", parametros.join(", ")));
        }
        c.nada_mas()?;
        self.copias += 1;
        env.sufijo = format!("#{nombre}{}", self.copias);
        let en_hueco = std::mem::take(&mut self.en_hueco);
        let desde = self.reglas.len();
        self.entornos.push(env);
        // Cuánto ocupa lo dice el propio componente: `size: 300, 44`.
        let mut tam = None;
        for e in cuerpo {
            if let Entrada::Prop { nombre, valor, linea, col } = e {
                if nombre == "size" {
                    let mut c = Cur::de(valor, *linea, *col);
                    tam = Some(self.punto(&mut c)?);
                }
            }
        }
        let r = self.grupo_con_propiedades(n, cuerpo);
        self.cerrar_ambito(desde);
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
        let var = c.id("un nombre para el contador")?;
        c.exige_palabra("in")?;
        let cte = |o: &Self, c: &mut Cur| -> R<i64> {
            match o.expr(c)? {
                Expr::K(v) => Ok(v as i64),
                _ => c.fallo("los límites de un `repeat` tienen que ser números: se despliega al cargar"),
            }
        };
        let desde = cte(self, c)?;
        c.exige_sim("..")?;
        let hasta = cte(self, c)?;
        c.nada_mas()?;
        if hasta - desde > 512 {
            return c.fallo("más de 512 vueltas en un `repeat` es que algo va mal");
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
            Some(F::Id(_)) => Some(c.id("un nombre")?),
            _ => None,
        };
        let muelle = if c.sim("~") { Some(self.muelle(c)?) } else { None };
        c.nada_mas()?;
        let mut p = self.propiedades(n, &["at", "anchor", "gap", "padding", "align", "fill", "corner", "show", "opacity", "cursor"])?;
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
            Some(c) => match c.id("start, center o end")?.as_str() {
                "start" => 0.0,
                "center" => 0.5,
                "end" => 1.0,
                _ => return c.fallo("se alinea a start, center o end"),
            },
            None => 0.0,
        };
        let fondo = match p.get_mut("fill") {
            Some(c) => Some(self.color(c)?),
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
                    _ => return c.fallo("un ancla es left, center o right, y top, middle o bottom"),
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
        self.desplegar(n.cuerpo.as_deref().unwrap_or(&[]), &mut Vec::new(), &mut hijos)?;

        struct Puesto {
            instr: usize,
            candidatas: std::ops::Range<usize>,
            tam: (Expr, Expr),
            visible: Expr,
        }
        let nivel = self.bajo.len();
        let mut puestos: Vec<Puesto> = Vec::new();
        for (hijo, entornos) in hijos {
            let marca = self.reglas.len();
            let extra = entornos.len();
            self.entornos.extend(entornos);
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
            r?;
            let Some(tam) = self.ultimo_tam.take() else {
                return Err(Fallo::en(hijo.linea, hijo.col, "no sé cuánto ocupa esto dentro de un reparto: mételo en un `group` con `size: ancho, alto`"));
            };
            puestos.push(Puesto { instr, candidatas: desde..self.candidatas.len(), tam, visible });
        }

        // Lo que ocupa cada uno a lo largo, y lo más que ocupa cualquiera a lo ancho.
        let largo = |p: &Puesto| if fila { p.tam.0.clone() } else { p.tam.1.clone() };
        let ancho = |p: &Puesto| if fila { p.tam.1.clone() } else { p.tam.0.clone() };
        let maximo = puestos.iter().fold(Expr::K(0.0), |m, p| m.max(ancho(p) * p.visible.clone()));
        let mut corrido = relleno.clone();
        for (k, p) in puestos.iter().enumerate() {
            let mut a_lo_largo = corrido.clone();
            if let Some(m) = muelle {
                // El hueco es un destino: el hijo va hacia él con el muelle del reparto.
                self.copias += 1;
                let prop = self.e.prop_con(fijo(&format!("·hueco{}", self.copias)), 0.0, m);
                self.e.comportamientos.push(Comportamiento::Sigue { prop, a: a_lo_largo });
                a_lo_largo = prop.e();
            }
            let a_lo_ancho = relleno.clone() + (maximo.clone() - ancho(p)) * alinea;
            let mueve = if fila { (a_lo_largo, a_lo_ancho) } else { (a_lo_ancho, a_lo_largo) };
            if let Instr::Transformar(Some(t)) = &mut self.e.instrs[p.instr] {
                t.mueve = mueve.clone();
            }
            for c in &mut self.candidatas[p.candidatas.clone()] {
                c.bajo[nivel].mueve = mueve.clone();
            }
            let ultimo = k + 1 == puestos.len();
            corrido = corrido + (largo(p) + if ultimo { Expr::K(0.0) } else { hueco.clone() }) * p.visible.clone();
        }
        let total_largo = corrido + relleno.clone();
        let total_ancho = maximo + relleno * 2.0;
        let tam = if fila { (total_largo, total_ancho) } else { (total_ancho, total_largo) };

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
        if opacidad.is_some() {
            self.e.pintar(Instr::Opacidad(None));
        }
        self.bajo.pop();
        self.e.pintar(Instr::Transformar(None));
        // Con nombre, su tamaño se puede usar más abajo (`bar.width`), y su caja
        // entera es una zona si alguna regla la nombra. Va debajo de las de sus
        // hijos: la rueda sobre el reparto no le quita el clic a lo de dentro.
        if let Some(local) = nombre {
            let nombre = self.declarar(&local);
            if let Some(e) = self.entornos.last_mut() {
                e.con_partes.insert(local.clone());
            }
            let mut bajo = self.bajo.clone();
            if let Instr::Transformar(Some(t)) = &self.e.instrs[instr_base] {
                bajo.push(t.clone());
            }
            let caja = Forma::Caja { centro: (tam.0.clone() * 0.5, tam.1.clone() * 0.5), mitad: (tam.0.clone() * 0.5, tam.1.clone() * 0.5), radio: esquina_de_zona };
            self.candidatas.insert(candidatas_base, Candidata { nombre: nombre.clone(), forma: caja, activa: None, bajo, forzada: false, cursor: cursor_del_reparto });
            let destino = match self.entornos.last_mut() {
                Some(e) => &mut e.exprs,
                None => &mut self.lets,
            };
            destino.insert(format!("{nombre}.width"), tam.0.clone());
            destino.insert(format!("{nombre}.height"), tam.1.clone());
        }
        self.ultimo_tam = Some(tam);
        Ok(())
    }

    /// Los hijos de un reparto, con cada `repeat` desplegado en sus vueltas.
    fn desplegar(&mut self, entradas: &'a [Entrada], ambito: &mut Vec<Entorno>, hijos: &mut Vec<(&'a Nodo, Vec<Entorno>)>) -> R<()> {
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
                    self.desplegar(n.cuerpo.as_deref().unwrap_or(&[]), ambito, hijos)?;
                    ambito.pop();
                }
            } else {
                hijos.push((n, ambito.clone()));
            }
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
        for (n, entornos) in &reglas {
            self.entornos = entornos.clone();
            for f in &n.cabeza {
                if let F::Id(s) = &f.f {
                    nombradas.insert(self.global(s));
                }
            }
        }
        self.entornos.clear();
        self.reglas = reglas;
        for k in std::mem::take(&mut self.candidatas) {
            if k.forzada || k.activa.is_some() || nombradas.contains(&k.nombre) {
                let z = self.e.zona_bajo(fijo(&k.nombre), k.forma, k.activa.unwrap_or(Expr::K(1.0)), k.bajo);
                self.e.zonas[z.0 as usize].cursor = k.cursor;
                self.zonas.insert(k.nombre, z);
            }
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
            let que = c.id("qué tiene que pasar: press, release, scroll, drag, hold, key, submit, focus, blur, drop, enter, leave, hover, away, idle, o un suceso")?;
            match que.as_str() {
                // `on press orb`, o con otro botón: `on press right orb`.
                "press" => {
                    if c.palabra("right") {
                        self.e.superficie.derecho_cierra = false;
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
                    let mut nombre = c.id("el nombre de una tecla: Escape, Return, a, Ctrl+k…")?;
                    while c.sim("+") {
                        nombre = format!("{nombre}+{}", c.id("la tecla")?);
                    }
                    Disparador::Tecla(nombre)
                }
                "submit" => {
                    let n = self.global(&c.id("el nombre del campo")?);
                    match self.textos.get(&n) {
                        Some(t) => Disparador::Envia(*t),
                        None => return self.desconocido(c, "ningún texto", &n, self.textos.keys().collect()),
                    }
                }
                "focus" => Disparador::GanaFoco,
                "blur" => Disparador::PierdeFoco,
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
                _ => {
                    c.i -= 1;
                    Disparador::Al(self.suceso(c)?)
                }
            }
        };
        c.nada_mas()?;
        let mut efectos = Vec::new();
        for e in n.cuerpo.as_deref().unwrap_or(&[]) {
            match e {
                Entrada::Prop { nombre, valor, linea, col } => efectos.push(Efecto::Animar(self.transicion(nombre, valor, *linea, *col)?)),
                Entrada::Nodo(x) => {
                    let mut c = Cur::de(&x.cabeza, x.linea, x.col);
                    let p = c.id("un efecto")?;
                    efectos.push(match p.as_str() {
                        "toggle" => Efecto::Alternar(self.hecho(&mut c)?),
                        "blur" => Efecto::Enfocar(None),
                        "focus" => {
                            let n = self.global(&c.id("el nombre del campo")?);
                            match self.textos.get(&n) {
                                Some(t) => Efecto::Enfocar(Some(*t)),
                                None => return self.desconocido(&c, "ningún texto", &n, self.textos.keys().collect()),
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
                        "impulse" => Efecto::Impulso(self.prop(&mut c)?, c.num()?),
                        "play" => {
                            let g = c.id("el nombre de un gesto")?;
                            match self.gestos.get(&g) {
                                Some(id) => Efecto::Gesto(*id),
                                None => return self.desconocido(&c, "ningún gesto", &g, self.gestos.keys().collect()),
                            }
                        }
                        _ => {
                            // `open = true`
                            c.i -= 1;
                            let h = self.hecho(&mut c)?;
                            c.exige_sim("=")?;
                            // Una expresión, que se evalúa al dispararse: `level = clamp(local.x / 64, 0, 1)`.
                            Efecto::Hecho(h, self.expr(&mut c)?)
                        }
                    });
                    c.nada_mas()?;
                }
            }
        }
        self.e.regla(cuando, efectos);
        Ok(())
    }

    // ── lo que el render lleva solo ─────────────────────────────

    fn comportamiento(&mut self, palabra: &str, c: &mut Cur) -> R<()> {
        let comp = match palabra {
            // blink eyelid every 2.4s..6s for 170ms
            "blink" => {
                let prop = self.prop(c)?;
                c.exige_palabra("every")?;
                let a = c.dur()?.as_secs_f32();
                c.exige_sim("..")?;
                let b = c.dur()?.as_secs_f32();
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
        let nombre = c.id("un nombre para el gesto")?;
        let (clase, mientras) = if palabra == "posture" {
            c.exige_palabra("while")?;
            (Clase::Postura, Some(self.expr(c)?))
        } else {
            let k = match c.id("su clase: ambient, reflex, asked o state")?.as_str() {
                "ambient" => Clase::Ambiente,
                "reflex" => Clase::Reflejo,
                "asked" => Clase::Pedido,
                "state" => Clase::Estado,
                _ => return c.fallo("las clases son ambient, reflex, asked y state; una postura se declara con `posture`"),
            };
            (k, None)
        };
        c.nada_mas()?;
        let mut fotogramas = Vec::new();
        for e in n.cuerpo.as_deref().unwrap_or(&[]) {
            let Entrada::Nodo(f) = e else {
                return Err(Fallo::en(n.linea, n.col, "un gesto son fotogramas: `130ms out_quad { eyes: 10 }`"));
            };
            let mut c = Cur::de(&f.cabeza, f.linea, f.col);
            let ms = (c.dur()?.as_secs_f32() * 1000.0) as u32;
            let mut foto = foto(ms, Curva::InOutSine);
            while !c.acabo() {
                let p = c.id("una curva, `hold` o `emit`")?;
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
                    _ => {
                        c.i -= 1;
                        return c.fallo("las curvas son linear, in_quad, out_quad, in_cubic, out_cubic, in_out_sine y out_back");
                    }
                }
            }
            for v in f.cuerpo.as_deref().unwrap_or(&[]) {
                let Entrada::Prop { nombre, valor, linea, col } = v else { continue };
                let nombre = &self.global(nombre);
                let Some(prop) = self.props.get(nombre).copied() else {
                    let pista = parecido(nombre, self.props.keys()).map_or(String::new(), |p| format!(" ¿Querías decir «{p}»?"));
                    return Err(Fallo::en(*linea, *col, format!("no hay ninguna propiedad que se llame «{nombre}».{pista}")));
                };
                if !self.e.pose.contains(&prop) {
                    return Err(Fallo::en(*linea, *col, format!("«{nombre}» no es de la pose: un gesto solo lleva de la mano lo que se declaró con `pose`")));
                }
                let mut c = Cur::de(valor, *linea, *col);
                foto.valores.push((prop, self.expr(&mut c)?));
                c.nada_mas()?;
            }
            fotogramas.push(foto);
        }
        if fotogramas.is_empty() {
            return Err(Fallo::en(n.linea, n.col, "un gesto sin fotogramas no hace nada"));
        }
        let g = self.e.gesto(fijo(&nombre), clase, fotogramas);
        if let Some(m) = mientras {
            self.e.postura(g, m);
        }
        self.gestos.insert(nombre, g);
        Ok(())
    }
}

fn leer_cursor(c: &mut Cur) -> R<Cursor> {
    Ok(match c.id("un cursor")?.as_str() {
        "default" => Cursor::Normal,
        "pointer" => Cursor::Mano,
        "text" => Cursor::Texto,
        "grab" => Cursor::Agarrar,
        "grabbing" => Cursor::Agarrando,
        _ => return c.fallo("los cursores son default, pointer, text, grab y grabbing"),
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
