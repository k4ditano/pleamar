//! Del árbol a la escena: qué significa cada nodo, y si los nombres existen.
//!
//! Se lee de arriba abajo y una sola vez: lo que se usa tiene que estar
//! declarado más arriba. A cambio, cada fallo sabe su línea.

use super::arbol::{Entrada, Nodo};
use super::fichas::{Ficha, F};
use super::{parecido, Fallo};
use crate::escena::*;
use std::collections::HashMap;
use std::time::Duration;

type R<T> = Result<T, Fallo>;

/// Los nombres viven lo que el programa. Cada recarga pierde unos bytes; está
/// apuntado como limitación.
fn fijo(s: &str) -> &'static str {
    Box::leak(s.to_owned().into_boxed_str())
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

struct Obra {
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
    muelles: HashMap<String, Muelle>,
    /// Las transformaciones bajo las que se está pintando: una zona las hereda.
    bajo: Vec<Transformacion>,
}

pub fn levantar(arbol: &[Entrada]) -> R<Escena> {
    let escena = match arbol {
        [Entrada::Nodo(n)] if matches!(n.cabeza.first().map(|f| &f.f), Some(F::Id(p)) if p == "scene") => n,
        _ => return Err(Fallo::en(1, 1, "un fichero es una escena: `scene Nombre { … }`")),
    };
    let mut o = Obra {
        e: Escena::default(),
        props: HashMap::new(), hechos: HashMap::new(), sucesos: HashMap::new(), textos: HashMap::new(), imagenes: HashMap::new(),
        medidas: HashMap::new(), gestos: HashMap::new(), zonas: HashMap::new(), lets: HashMap::new(),
        muelles: [("lively", Muelle::VIVO), ("calm", Muelle::SERENO), ("quick", Muelle::RAPIDO), ("slow", Muelle::LENTO), ("eyes", Muelle::OJOS), ("pose", Muelle::POSE)]
            .into_iter().map(|(n, m)| (n.to_owned(), m)).collect(),
        bajo: Vec::new(),
    };
    // Un suceso que siempre existe: lo dispara `--demo`, para escenas sin ratón.
    let demo = o.e.suceso("demo");
    o.sucesos.insert("demo".into(), demo);
    let cuerpo = escena.cuerpo.as_ref().ok_or_else(|| Fallo::en(escena.linea, escena.col, "a la escena le falta su bloque `{ … }`"))?;
    o.grupo(cuerpo)?;
    Ok(o.e)
}

impl Obra {
    // ── nombres ─────────────────────────────────────────────────

    fn desconocido<T>(&self, c: &Cur, que: &str, nombre: &str, conocidos: Vec<&String>) -> R<T> {
        let pista = parecido(nombre, conocidos.into_iter()).map_or(String::new(), |p| format!(" ¿Querías decir «{p}»?"));
        let (l, col) = c.f.get(c.i.saturating_sub(1)).map_or(c.fin, |x| (x.linea, x.col));
        Err(Fallo::en(l, col, format!("no hay {que} que se llame «{nombre}».{pista} Lo que se usa tiene que estar declarado más arriba.")))
    }

    fn prop(&self, c: &mut Cur) -> R<PropId> {
        let n = c.id("el nombre de una propiedad")?;
        match self.props.get(&n) {
            Some(p) => Ok(*p),
            None => self.desconocido(c, "ninguna propiedad", &n, self.props.keys().collect()),
        }
    }
    fn hecho(&self, c: &mut Cur) -> R<HechoId> {
        let n = c.id("el nombre de un hecho")?;
        match self.hechos.get(&n) {
            Some(h) => Ok(*h),
            None => self.desconocido(c, "ningún hecho", &n, self.hechos.keys().collect()),
        }
    }
    fn suceso(&self, c: &mut Cur) -> R<SucesoId> {
        let n = c.id("el nombre de un suceso")?;
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
        let n = c.id("el nombre de una forma o de una zona")?;
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
            _ => c.fallo("aquí esperaba un color, como #151616"),
        }
    }

    // ── el bloque de un nodo, visto como propiedades ────────────

    fn propiedades<'a>(&self, n: &'a Nodo, validas: &[&str]) -> R<HashMap<&'a str, Cur<'a>>> {
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
        let comunes = ["rotate", "stroke", "color", "opacity", "blend", "active"];
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
        let mut forma = match clase.as_str() {
            "ellipse" => Forma::Elipse { centro: at.ok_or_else(|| falta("at"))?, radio: radius.ok_or_else(|| falta("radius"))?, escala: scale.unwrap_or((1.0.into(), 1.0.into())) },
            "box" => {
                let (w, h) = size.ok_or_else(|| falta("size"))?;
                let centro = match (at, from) {
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
        // Una forma con nombre es también una zona: se puede pulsar, y el ratón
        // entra por ella. Hereda las transformaciones bajo las que se pinta.
        if let Some(nombre) = &nombre {
            let z = self.e.zona_bajo(fijo(nombre), forma.clone(), active.unwrap_or(Expr::K(1.0)), self.bajo.clone());
            self.zonas.insert(nombre.clone(), z);
        }
        Ok(FormaLeida { forma, color, opacidad: opacity, fusion: blend })
    }

    // ── lo que se pinta ─────────────────────────────────────────

    /// Un grupo: sus propiedades (transformación, opacidad) y sus hijos en orden.
    fn grupo(&mut self, entradas: &[Entrada]) -> R<()> {
        let mut recortes = 0;
        for e in entradas {
            let Entrada::Nodo(n) = e else { continue };
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
                    let nombre = c.id("un nombre para la propiedad")?;
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
                    let nombre = c.id("un nombre para el hecho")?;
                    c.exige_sim("=")?;
                    let v = if c.palabra("true") { 1.0 } else if c.palabra("false") { 0.0 } else { c.num()? };
                    c.nada_mas()?;
                    let h = self.e.hecho(fijo(&nombre), v);
                    self.hechos.insert(nombre, h);
                }
                "event" => {
                    let nombre = c.id("un nombre para el suceso")?;
                    let s = if c.sim("->") { self.e.suceso_que_sale(fijo(&nombre)) } else { self.e.suceso(fijo(&nombre)) };
                    c.nada_mas()?;
                    self.sucesos.insert(nombre, s);
                }
                "text" if matches!(c.f.get(2).map(|x| &x.f), Some(F::Sim("="))) => {
                    let nombre = c.id("un nombre para el texto")?;
                    c.exige_sim("=")?;
                    let t = self.e.texto_vivo(fijo(&nombre), &c.cadena()?);
                    self.textos.insert(nombre, t);
                }
                "image" if matches!(c.f.get(2).map(|x| &x.f), Some(F::Sim("="))) => {
                    let nombre = c.id("un nombre para la imagen")?;
                    c.exige_sim("=")?;
                    let fuente = if c.palabra("icon") { Fuente::Icono(c.cadena()?) } else if c.palabra("file") { Fuente::Ruta(c.cadena()?.into()) } else { return c.fallo("una imagen es `icon \"nombre\"` o `file \"ruta\"`") };
                    c.exige_sim(",")?;
                    let w = c.num()?;
                    c.exige_sim(",")?;
                    let i = self.e.imagen(fuente, w as u32, c.num()? as u32);
                    self.imagenes.insert(nombre, i);
                }
                "measure" => {
                    let nombre = c.id("un nombre para la medida")?;
                    let (w, h) = self.e.medida(fijo(&nombre));
                    self.props.insert(format!("{nombre}.width"), w);
                    self.props.insert(format!("{nombre}.height"), h);
                    self.medidas.insert(nombre, (w, h));
                }
                "let" => {
                    let nombre = c.id("un nombre")?;
                    c.exige_sim("=")?;
                    let e = self.expr(&mut c)?;
                    c.nada_mas()?;
                    self.lets.insert(nombre, e);
                }
                "zone" => {
                    // Una forma que no se pinta: solo es sensible.
                    self.forma(n, 1)?;
                }
                "body" => self.cuerpo(n)?,
                "ellipse" | "box" | "arc" | "line" => {
                    let f = self.forma(n, 0)?;
                    self.e.pintar(Instr::Plano { forma: f.forma, color: f.color.unwrap_or_else(|| color(1.0, 1.0, 1.0)), alfa: f.opacidad.unwrap_or(Expr::K(1.0)) });
                }
                "text" => self.texto(n)?,
                "image" => self.imagen(n)?,
                "clip" => {
                    let margen = if c.palabra("inset") { c.num()? } else { 0.0 };
                    let desde = c.i;
                    let f = self.forma(n, desde)?;
                    self.e.pintar(Instr::Recorte(Some((f.forma, margen))));
                    recortes += 1;
                }
                "group" => self.grupo_con_propiedades(n)?,
                "layer" => self.capa(n, &mut c)?,
                "on" | "every" => self.regla(n, &palabra, &mut c)?,
                "blink" | "wave" | "spin" | "follow" | "look" => self.comportamiento(&palabra, &mut c)?,
                "gesture" | "posture" => self.gesto(n, &palabra, &mut c)?,
                otra => {
                    let validas: Vec<String> = ["surface", "prop", "pose", "fact", "event", "text", "image", "measure", "let", "spring", "body", "ellipse", "box", "arc", "line", "zone", "clip", "group", "layer", "on", "every", "blink", "wave", "spin", "follow", "look", "gesture", "posture"].iter().map(|s| s.to_string()).collect();
                    let pista = parecido(otra, validas.iter()).map_or(String::new(), |p| format!(" ¿Querías decir «{p}»?"));
                    return Err(Fallo::en(n.linea, n.col, format!("no sé qué es «{otra}».{pista}")));
                }
            }
        }
        // Un recorte vale hasta el final de su grupo.
        for _ in 0..recortes {
            self.e.pintar(Instr::Recorte(None));
        }
        Ok(())
    }

    fn grupo_con_propiedades(&mut self, n: &Nodo) -> R<()> {
        let mut p = self.propiedades(n, &["pivot", "rotate", "scale", "move", "opacity"])?;
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
        self.grupo(n.cuerpo.as_deref().unwrap_or(&[]))?;
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
        let mut p = self.propiedades(n, &["color", "gradient", "rim", "light", "shadow", "border", "opacity"])?;
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
                let f = self.forma(h, 0)?;
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
        Ok(())
    }

    /// `text notice.title { at: …; size: 20 }` o `text "Descartar" { … }`
    fn texto(&mut self, n: &Nodo) -> R<()> {
        let mut c = Cur::de(&n.cabeza[1..], n.linea, n.col);
        let contenido = match c.mira() {
            Some(F::Cadena(s)) => Contenido::Fijo(s.clone()),
            Some(F::Id(nombre)) => match self.textos.get(nombre) {
                Some(t) => Contenido::Vivo(*t),
                None => {
                    c.i += 1;
                    return self.desconocido(&c, "ningún texto", nombre, self.textos.keys().collect());
                }
            },
            _ => return c.fallo("un texto es `text \"literal\" { … }` o `text nombre { … }`"),
        };
        let mut p = self.propiedades(n, &["at", "anchor", "width", "size", "weight", "color", "opacity", "lines", "align", "line_height", "family", "measure"])?;
        let en = match p.get_mut("at") {
            Some(c) => self.punto(c)?,
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
                let nombre = c.id("el nombre de una medida")?;
                match self.medidas.get(&nombre) {
                    Some(m) => Some(*m),
                    None => return self.desconocido(c, "ninguna medida", &nombre, self.medidas.keys().collect()),
                }
            }
            None => None,
        };
        self.e.pintar(Instr::Texto { contenido, en, ancla, ancho, estilo, alfa, mide });
        Ok(())
    }

    fn imagen(&mut self, n: &Nodo) -> R<()> {
        let mut c = Cur::de(&n.cabeza[1..], n.linea, n.col);
        let nombre = c.id("el nombre de una imagen")?;
        let Some(imagen) = self.imagenes.get(&nombre).copied() else {
            return self.desconocido(&c, "ninguna imagen", &nombre, self.imagenes.keys().collect());
        };
        let mut p = self.propiedades(n, &["at", "size", "opacity", "tint"])?;
        let falta = |q: &str| Fallo::en(n.linea, n.col, format!("a esta imagen le falta «{q}»"));
        let (x, y) = self.punto(p.get_mut("at").ok_or_else(|| falta("at"))?)?;
        let (w, h) = self.punto(p.get_mut("size").ok_or_else(|| falta("size"))?)?;
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

    fn superficie(&mut self, n: &Nodo) -> R<()> {
        let mut p = self.propiedades(n, &["size", "anchor", "margin", "level", "reserve", "screens"])?;
        let s = &mut self.e.superficie;
        if let Some(c) = p.get_mut("size") {
            s.ancho = c.num()? as u32;
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
        let Some(prop) = self.props.get(nombre).copied() else {
            let pista = parecido(nombre, self.props.keys()).map_or(String::new(), |p| format!(" ¿Querías decir «{p}»?"));
            return Err(Fallo::en(linea, col, format!("no hay ninguna propiedad que se llame «{nombre}».{pista}")));
        };
        let mut c = Cur::de(valor, linea, col);
        let a = c.num()?;
        let muelle = if c.sim("~") { self.muelle(&mut c)? } else { Muelle::VIVO };
        let retraso = if c.palabra("after") { c.dur()? } else { Duration::ZERO };
        c.nada_mas()?;
        Ok(Transicion { prop, a, muelle, retraso })
    }

    // ── reglas ──────────────────────────────────────────────────

    fn regla(&mut self, n: &Nodo, palabra: &str, c: &mut Cur) -> R<()> {
        let mientras = |o: &Obra, c: &mut Cur| -> R<Expr> { if c.palabra("while") { o.expr(c) } else { Ok(Expr::K(1.0)) } };
        let cuando = if palabra == "every" {
            let a = c.dur()?.as_secs_f32();
            let b = if c.sim("..") { c.dur()?.as_secs_f32() } else { a };
            Disparador::Cada { entre: (a, b), mientras: mientras(self, c)? }
        } else {
            let que = c.id("qué tiene que pasar: press, enter, leave, hover, away, idle, o un suceso")?;
            match que.as_str() {
                "press" => Disparador::Pulsa(self.zona(c)?),
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
                        "emit" => Efecto::Suceso(self.suceso(&mut c)?),
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
                            let v = if c.palabra("true") { 1.0 } else if c.palabra("false") { 0.0 } else { c.num()? };
                            Efecto::Hecho(h, v)
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
                let Some(prop) = self.props.get(nombre).copied() else {
                    let pista = parecido(nombre, self.props.keys()).map_or(String::new(), |p| format!(" ¿Querías decir «{p}»?"));
                    return Err(Fallo::en(*linea, *col, format!("no hay ninguna propiedad que se llame «{nombre}».{pista}")));
                };
                if !self.e.pose.contains(&prop) {
                    return Err(Fallo::en(*linea, *col, format!("«{nombre}» no es de la pose: un gesto solo lleva de la mano lo que se declaró con `pose`")));
                }
                let mut c = Cur::de(valor, *linea, *col);
                foto.valores.push((prop, c.num()?));
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

struct FormaLeida {
    forma: Forma,
    color: Option<Color>,
    opacidad: Option<Expr>,
    fusion: Option<Expr>,
}
