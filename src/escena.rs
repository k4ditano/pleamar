//! El contrato entre quien piensa y quien pinta.
//!
//! Una escena es *datos*: propiedades con nombre, una lista de instrucciones de
//! dibujo cuyos números son expresiones sobre esas propiedades, comportamientos
//! que el render lleva solo (parpadear, mirar, respirar) y zonas sensibles al
//! ratón. El render no sabe qué es una bolita: evalúa, pinta y avisa.
//!
//! La lógica nunca manda valores: manda *intenciones* («esta propiedad va a 406
//! con este muelle, dentro de 60 ms»).

#![allow(dead_code)] // el contrato va por delante de las escenas que lo usan

use std::ops::{Add, Div, Mul, Sub};
use std::time::Duration;

pub const ANCHO: u32 = 720;
pub const ALTO: u32 = 300;
pub const MAX_INSTR: usize = 64;

// ── propiedades y expresiones ───────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropId(pub u16);
/// Algo que la lógica (o una regla) dice que es verdad. Un número; sí y no son 1 y 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HechoId(pub u16);
/// Algo que ocurre en un instante.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SucesoId(pub u16);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZonaId(pub u16);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GestoId(pub u16);

/// Con qué se evalúa una expresión: lo que se mueve y lo que es verdad.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub props: &'a [Animada],
    pub hechos: &'a [f32],
}

/// Una expresión pura sobre las propiedades. Como no tiene efectos, el render
/// puede evaluarla cuando quiera y donde quiera: es lo que sería un binding.
#[derive(Clone, Debug)]
pub enum Expr {
    K(f32),
    P(PropId),
    H(HechoId),
    /// La velocidad de una propiedad: el render la conoce, la lógica no.
    Vel(PropId),
    Suma(Box<Expr>, Box<Expr>),
    Resta(Box<Expr>, Box<Expr>),
    Por(Box<Expr>, Box<Expr>),
    Entre(Box<Expr>, Box<Expr>),
    Min(Box<Expr>, Box<Expr>),
    Max(Box<Expr>, Box<Expr>),
    Abs(Box<Expr>),
    /// smoothstep(a, b, x)
    Suave(f32, f32, Box<Expr>),
    // Condiciones: verdad es > 0.5, y devuelven 1 o 0.
    Mayor(Box<Expr>, Box<Expr>),
    Y(Box<Expr>, Box<Expr>),
    O(Box<Expr>, Box<Expr>),
    No(Box<Expr>),
}

impl Expr {
    pub fn evaluar(&self, c: Ctx) -> f32 {
        use Expr::*;
        match self {
            K(v) => *v,
            P(p) => c.props[p.0 as usize].x,
            H(h) => c.hechos[h.0 as usize],
            Vel(p) => c.props[p.0 as usize].v,
            Suma(a, b) => a.evaluar(c) + b.evaluar(c),
            Resta(a, b) => a.evaluar(c) - b.evaluar(c),
            Por(a, b) => a.evaluar(c) * b.evaluar(c),
            Entre(a, b) => a.evaluar(c) / b.evaluar(c),
            Min(a, b) => a.evaluar(c).min(b.evaluar(c)),
            Max(a, b) => a.evaluar(c).max(b.evaluar(c)),
            Abs(a) => a.evaluar(c).abs(),
            Suave(a, b, x) => {
                let t = ((x.evaluar(c) - a) / (b - a)).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            }
            Mayor(a, b) => (a.evaluar(c) > b.evaluar(c)) as u8 as f32,
            Y(a, b) => (a.evaluar(c) > 0.5 && b.evaluar(c) > 0.5) as u8 as f32,
            O(a, b) => (a.evaluar(c) > 0.5 || b.evaluar(c) > 0.5) as u8 as f32,
            No(a) => (a.evaluar(c) <= 0.5) as u8 as f32,
        }
    }
    pub fn es_verdad(&self, c: Ctx) -> bool {
        self.evaluar(c) > 0.5
    }
    pub fn mayor(self, o: impl Into<Expr>) -> Expr {
        Expr::Mayor(Box::new(self), Box::new(o.into()))
    }
    pub fn y(self, o: impl Into<Expr>) -> Expr {
        Expr::Y(Box::new(self), Box::new(o.into()))
    }
    pub fn o(self, o: impl Into<Expr>) -> Expr {
        Expr::O(Box::new(self), Box::new(o.into()))
    }
    pub fn no(self) -> Expr {
        Expr::No(Box::new(self))
    }
    pub fn min(self, o: impl Into<Expr>) -> Expr {
        Expr::Min(Box::new(self), Box::new(o.into()))
    }
    pub fn max(self, o: impl Into<Expr>) -> Expr {
        Expr::Max(Box::new(self), Box::new(o.into()))
    }
    pub fn acotar(self, a: f32, b: f32) -> Expr {
        self.max(a).min(b)
    }
    pub fn abs(self) -> Expr {
        Expr::Abs(Box::new(self))
    }
    pub fn suave(self, a: f32, b: f32) -> Expr {
        Expr::Suave(a, b, Box::new(self))
    }
}

/// a·(1−t) + b·t
pub fn mezcla(a: f32, b: f32, t: impl Into<Expr>) -> Expr {
    t.into() * (b - a) + a
}

impl PropId {
    pub fn e(self) -> Expr {
        Expr::P(self)
    }
    pub fn vel(self) -> Expr {
        Expr::Vel(self)
    }
}
impl HechoId {
    pub fn e(self) -> Expr {
        Expr::H(self)
    }
}
impl From<HechoId> for Expr {
    fn from(h: HechoId) -> Expr {
        Expr::H(h)
    }
}
impl From<f32> for Expr {
    fn from(v: f32) -> Expr {
        Expr::K(v)
    }
}
impl From<PropId> for Expr {
    fn from(p: PropId) -> Expr {
        Expr::P(p)
    }
}

macro_rules! operador {
    ($rasgo:ident, $metodo:ident, $variante:ident) => {
        impl<T: Into<Expr>> $rasgo<T> for Expr {
            type Output = Expr;
            fn $metodo(self, o: T) -> Expr {
                Expr::$variante(Box::new(self), Box::new(o.into()))
            }
        }
        impl<T: Into<Expr>> $rasgo<T> for PropId {
            type Output = Expr;
            fn $metodo(self, o: T) -> Expr {
                Expr::$variante(Box::new(self.into()), Box::new(o.into()))
            }
        }
        impl $rasgo<Expr> for f32 {
            type Output = Expr;
            fn $metodo(self, o: Expr) -> Expr {
                Expr::$variante(Box::new(self.into()), Box::new(o))
            }
        }
        impl $rasgo<PropId> for f32 {
            type Output = Expr;
            fn $metodo(self, o: PropId) -> Expr {
                Expr::$variante(Box::new(self.into()), Box::new(o.into()))
            }
        }
    };
}
operador!(Add, add, Suma);
operador!(Sub, sub, Resta);
operador!(Mul, mul, Por);
operador!(Div, div, Entre);

// ── lo que se pinta ─────────────────────────────────────────────

pub type Punto = (Expr, Expr);

#[derive(Clone, Debug)]
pub enum Forma {
    Elipse { centro: Punto, radio: Expr, escala: Punto },
    /// Caja redondeada. Con media anchura o altura por debajo de medio píxel,
    /// no existe: ni se pinta ni se funde con nada.
    Caja { centro: Punto, mitad: Punto, radio: Expr },
}

impl Forma {
    pub fn circulo(centro: Punto, radio: impl Into<Expr>) -> Forma {
        Forma::Elipse { centro, radio: radio.into(), escala: (1.0.into(), 1.0.into()) }
    }

    /// La misma distancia que calcula el shader, para saber si el ratón está
    /// dentro sin preguntarle a la GPU.
    pub fn distancia(&self, c: Ctx, x: f32, y: f32) -> f32 {
        match self {
            Forma::Elipse { centro, radio, escala } => {
                let (ex, ey) = (escala.0.evaluar(c), escala.1.evaluar(c));
                let (qx, qy) = ((x - centro.0.evaluar(c)) / ex, (y - centro.1.evaluar(c)) / ey);
                (qx.hypot(qy) - radio.evaluar(c)) * ex.min(ey)
            }
            Forma::Caja { centro, mitad, radio } => {
                let (mx, my) = (mitad.0.evaluar(c), mitad.1.evaluar(c));
                if mx < 0.5 || my < 0.5 {
                    return f32::MAX;
                }
                let r = radio.evaluar(c);
                let qx = (x - centro.0.evaluar(c)).abs() - mx + r;
                let qy = (y - centro.1.evaluar(c)).abs() - my + r;
                qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Sombra {
    pub desplazada: (f32, f32),
    pub difusa: f32,
    pub alfa: f32,
}

/// Un degradado vertical de claridad, para que el cuerpo no sea plano.
#[derive(Clone, Debug)]
pub struct Luz {
    pub cantidad: f32,
    pub desde_y: Expr,
    pub alto: f32,
}

pub type Color = [Expr; 3];

pub fn color(r: f32, g: f32, b: f32) -> Color {
    [r.into(), g.into(), b.into()]
}

/// La lista de dibujo. Se recorre en orden, por píxel, en la GPU.
#[derive(Clone, Debug)]
pub enum Instr {
    /// Empieza un cuerpo: a partir de aquí las formas se acumulan.
    Grupo { sombra: Option<Sombra> },
    /// Añade una forma al cuerpo, fundida con lo que lleve (`fusion` es el
    /// radio del mínimo suave; 0 es una unión seca).
    Forma { forma: Forma, fusion: Expr },
    /// Pinta el cuerpo acumulado: sombra, relleno, luz y filo.
    Relleno { color: Color, alfa: Expr, filo: f32, luz: Option<Luz> },
    /// Todo lo que venga después se recorta a esta forma. `None` lo quita.
    Recorte(Option<(Forma, f32)>),
    /// Una forma suelta, de color plano.
    Plano { forma: Forma, color: Color, alfa: Expr },
    /// Un trozo del atlas de la escena, colocado en pantalla.
    Textura { destino: (Expr, Expr, Expr, Expr), uv: [f32; 4], alfa: Expr },
}

// ── lo que el render hace solo ──────────────────────────────────

#[derive(Clone, Debug)]
pub enum Comportamiento {
    /// Lleva la propiedad de 1 a 0 y de vuelta, de vez en cuando.
    Parpadeo { prop: PropId, cada: (f32, f32), dura: f32 },
    /// prop = amplitud · sin(frecuencia · t). Con amplitud 0 no cuesta nada.
    Onda { prop: PropId, frecuencia: f32, amplitud: Expr },
    /// Dos propiedades que tiran hacia el puntero, con su propio muelle.
    Mirada { x: PropId, y: PropId, centro: Punto, alcance: (f32, f32), distancia: f32, reposo: Punto },
}

#[derive(Clone, Debug)]
pub struct Transicion {
    pub prop: PropId,
    pub a: f32,
    pub muelle: Muelle,
    pub retraso: Duration,
}

pub fn ir(prop: PropId, a: f32, muelle: Muelle, retraso_ms: u64) -> Transicion {
    Transicion { prop, a, muelle, retraso: Duration::from_millis(retraso_ms) }
}

/// Una región sensible: una forma con nombre. El render hace el hit-test con
/// la misma fórmula con la que pinta.
#[derive(Clone, Debug)]
pub struct Zona {
    pub id: &'static str,
    pub forma: Forma,
    pub activa: Expr,
}

// ── capas: quién gana ───────────────────────────────────────────

/// Cuándo se cumple una reclamación.
#[derive(Clone, Debug)]
pub enum Cuando {
    Siempre,
    Mientras(Expr),
    /// Durante un rato después de cualquiera de estos sucesos.
    Tras { sucesos: Vec<SucesoId>, dura: Duration },
    /// Desde uno de estos sucesos hasta uno de aquellos.
    DesdeHasta { desde: Vec<SucesoId>, hasta: Vec<SucesoId> },
}

#[derive(Clone, Debug)]
pub struct Reclamacion {
    pub nombre: &'static str,
    pub cuando: Cuando,
    /// Lo que fija al ganar: es lo que el boceto B llamaba «estado».
    pub fija: Vec<Transicion>,
}

impl Reclamacion {
    pub fn mientras(nombre: &'static str, c: impl Into<Expr>) -> Self {
        Reclamacion { nombre, cuando: Cuando::Mientras(c.into()), fija: vec![] }
    }
    pub fn tras(nombre: &'static str, sucesos: &[SucesoId], ms: u64) -> Self {
        Reclamacion { nombre, cuando: Cuando::Tras { sucesos: sucesos.to_vec(), dura: Duration::from_millis(ms) }, fija: vec![] }
    }
    pub fn desde_hasta(nombre: &'static str, desde: &[SucesoId], hasta: &[SucesoId]) -> Self {
        Reclamacion { nombre, cuando: Cuando::DesdeHasta { desde: desde.to_vec(), hasta: hasta.to_vec() }, fija: vec![] }
    }
    pub fn por_defecto(nombre: &'static str) -> Self {
        Reclamacion { nombre, cuando: Cuando::Siempre, fija: vec![] }
    }
    pub fn fija(mut self, t: Vec<Transicion>) -> Self {
        self.fija = t;
        self
    }
}

/// Un hueco que muchos reclaman. Gana la primera reclamación que se cumple;
/// cuando deja de cumplirse se ve la siguiente, sola. Nadie «apaga» nada.
#[derive(Clone, Debug)]
pub struct Capa {
    pub nombre: &'static str,
    pub muelle: Muelle,
    pub reclamaciones: Vec<Reclamacion>,
    /// Una propiedad por reclamación, que va a 1 cuando gana y a 0 cuando no:
    /// con ella se funde un dibujo en otro.
    pub presencias: Vec<PropId>,
}

/// Lo que devuelve `Escena::capa`, para pintar según quién gane.
pub struct CapaRef {
    pub presencias: Vec<PropId>,
}
impl CapaRef {
    pub fn presencia(&self, i: usize) -> PropId {
        self.presencias[i]
    }
}

// ── gestos: líneas de tiempo ────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub enum Curva {
    Lineal,
    InQuad,
    OutQuad,
    InCubic,
    OutCubic,
    InOutSine,
    OutBack,
}

impl Curva {
    pub fn aplicar(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Curva::Lineal => t,
            Curva::InQuad => t * t,
            Curva::OutQuad => 1.0 - (1.0 - t) * (1.0 - t),
            Curva::InCubic => t * t * t,
            Curva::OutCubic => 1.0 - (1.0 - t).powi(3),
            Curva::InOutSine => 0.5 - 0.5 * (std::f32::consts::PI * t).cos(),
            Curva::OutBack => {
                let (c1, u) = (1.70158, t - 1.0);
                1.0 + (c1 + 1.0) * u * u * u + c1 * u * u
            }
        }
    }
}

/// Quién puede interrumpir a quién: un gesto solo corta a otro de su clase o
/// inferior. Un reflejo no le quita la cara a algo que se pidió.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Clase {
    Ambiente,
    Postura,
    Reflejo,
    Pedido,
    Estado,
}

#[derive(Clone, Debug)]
pub struct Fotograma {
    pub ms: u32,
    pub aguanta: u32,
    pub curva: Curva,
    /// Lo que no se nombra vuelve a su pose base.
    pub valores: Vec<(PropId, f32)>,
    pub emite: Option<SucesoId>,
}

pub fn foto(ms: u32, curva: Curva) -> Fotograma {
    Fotograma { ms, aguanta: 0, curva, valores: vec![], emite: None }
}
impl Fotograma {
    pub fn con(mut self, p: PropId, v: f32) -> Self {
        self.valores.push((p, v));
        self
    }
    pub fn aguanta(mut self, ms: u32) -> Self {
        self.aguanta = ms;
        self
    }
    pub fn emite(mut self, s: SucesoId) -> Self {
        self.emite = Some(s);
        self
    }
}

#[derive(Clone, Debug)]
pub struct Gesto {
    pub nombre: &'static str,
    pub clase: Clase,
    pub fotogramas: Vec<Fotograma>,
}

// ── reglas: qué hace cambiar las cosas ──────────────────────────

#[derive(Clone, Debug)]
pub enum Disparador {
    Entra(ZonaId),
    Sale(ZonaId),
    Pulsa(ZonaId),
    /// El ratón lleva este rato encima.
    Encima { zona: ZonaId, durante: Duration },
    /// Ha estado encima y lleva este rato fuera.
    Fuera { zona: ZonaId, durante: Duration },
    /// Nadie ha tocado el ratón en este rato, mientras se cumpla la condición.
    Quieto { durante: Duration, mientras: Expr },
    /// De vez en cuando, con azar, mientras se cumpla la condición.
    Cada { entre: (f32, f32), mientras: Expr },
    Al(SucesoId),
}

#[derive(Clone, Debug)]
pub enum Efecto {
    Animar(Transicion),
    Hecho(HechoId, f32),
    /// De sí a no y de no a sí.
    Alternar(HechoId),
    Suceso(SucesoId),
    Impulso(PropId, f32),
    Gesto(GestoId),
}

/// Todo lo que hay aquí lo ejecuta el render, esté la lógica como esté.
#[derive(Clone, Debug)]
pub struct Regla {
    pub cuando: Disparador,
    pub efectos: Vec<Efecto>,
}

pub fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

pub struct Lienzo {
    pub ancho: usize,
    pub alto: usize,
    pub rgba: Vec<u8>,
}

#[derive(Default)]
pub struct Escena {
    pub props: Vec<(&'static str, f32, Muelle)>,
    pub instrs: Vec<Instr>,
    pub comportamientos: Vec<Comportamiento>,
    pub zonas: Vec<Zona>,
    pub atlas: Option<Lienzo>,
    pub hechos: Vec<(&'static str, f32)>,
    /// Nombre, y si además de a la escena le llega a la lógica.
    pub sucesos: Vec<(&'static str, bool)>,
    pub capas: Vec<Capa>,
    pub gestos: Vec<Gesto>,
    /// Las propiedades que forman la pose: las que un gesto lleva de la mano.
    pub pose: Vec<PropId>,
    /// Gestos que se repiten solos mientras algo sea verdad.
    pub posturas: Vec<(GestoId, Expr)>,
    pub reglas: Vec<Regla>,
}

impl Escena {
    /// Las propiedades van por nombre: si la escena se sustituye por otra, las
    /// que se llamen igual conservan valor y velocidad.
    pub fn prop(&mut self, nombre: &'static str, inicial: f32) -> PropId {
        self.prop_con(nombre, inicial, Muelle::VIVO)
    }
    pub fn prop_con(&mut self, nombre: &'static str, inicial: f32, muelle: Muelle) -> PropId {
        self.props.push((nombre, inicial, muelle));
        PropId(self.props.len() as u16 - 1)
    }
    pub fn pintar(&mut self, i: Instr) {
        self.instrs.push(i);
    }
    /// Una propiedad de la pose, con su valor de reposo.
    pub fn prop_de_pose(&mut self, nombre: &'static str, reposo: f32) -> PropId {
        let p = self.prop_con(nombre, reposo, Muelle::POSE);
        self.pose.push(p);
        p
    }
    pub fn hecho(&mut self, nombre: &'static str, inicial: f32) -> HechoId {
        self.hechos.push((nombre, inicial));
        HechoId(self.hechos.len() as u16 - 1)
    }
    /// Un suceso interno: lo oyen las capas y las reglas.
    pub fn suceso(&mut self, nombre: &'static str) -> SucesoId {
        self.sucesos.push((nombre, false));
        SucesoId(self.sucesos.len() as u16 - 1)
    }
    /// Un suceso que además sale hacia la lógica.
    pub fn suceso_que_sale(&mut self, nombre: &'static str) -> SucesoId {
        self.sucesos.push((nombre, true));
        SucesoId(self.sucesos.len() as u16 - 1)
    }
    pub fn zona(&mut self, id: &'static str, forma: Forma, activa: impl Into<Expr>) -> ZonaId {
        self.zonas.push(Zona { id, forma, activa: activa.into() });
        ZonaId(self.zonas.len() as u16 - 1)
    }
    /// Las reclamaciones van de más a menos prioridad; la última debería ser
    /// `por_defecto`.
    pub fn capa(&mut self, nombre: &'static str, muelle: Muelle, reclamaciones: Vec<Reclamacion>) -> CapaRef {
        let presencias: Vec<PropId> = reclamaciones
            .iter()
            .map(|r| {
                // El nombre vive lo que el programa: una fuga pequeña y una sola vez por escena.
                let n: &'static str = Box::leak(format!("capa.{nombre}.{}", r.nombre).into_boxed_str());
                self.props.push((n, 0.0, muelle));
                PropId(self.props.len() as u16 - 1)
            })
            .collect();
        self.capas.push(Capa { nombre, muelle, reclamaciones, presencias: presencias.clone() });
        CapaRef { presencias }
    }
    pub fn gesto(&mut self, nombre: &'static str, clase: Clase, fotogramas: Vec<Fotograma>) -> GestoId {
        self.gestos.push(Gesto { nombre, clase, fotogramas });
        GestoId(self.gestos.len() as u16 - 1)
    }
    pub fn postura(&mut self, gesto: GestoId, mientras: impl Into<Expr>) {
        self.posturas.push((gesto, mientras.into()));
    }
    pub fn regla(&mut self, cuando: Disparador, efectos: Vec<Efecto>) {
        self.reglas.push(Regla { cuando, efectos });
    }
}

// ── mensajes ────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct Muelle {
    pub rigidez: f32,
    pub freno: f32,
}

impl Muelle {
    pub const VIVO: Muelle = Muelle { rigidez: 170.0, freno: 19.0 };
    pub const SERENO: Muelle = Muelle { rigidez: 150.0, freno: 23.0 };
    pub const RAPIDO: Muelle = Muelle { rigidez: 420.0, freno: 40.0 };
    pub const LENTO: Muelle = Muelle { rigidez: 28.0, freno: 11.0 };
    pub const OJOS: Muelle = Muelle { rigidez: 190.0, freno: 24.0 };
    pub const POSE: Muelle = Muelle { rigidez: 260.0, freno: 28.0 };
}

pub enum Orden {
    Animar(Transicion),
    Impulso { prop: PropId, velocidad: f32 },
    /// Solo en el modo ingenuo: la lógica vive en el hilo que pinta, como en
    /// QtQuick, y su trabajo le para el reloj a todo.
    Bloquear(Duration),
}

pub enum ARender {
    Escena(Escena),
    Orden(Orden),
    /// La frontera: la lógica cuenta lo que pasa, y nada más.
    Hecho(&'static str, f32),
    Suceso(&'static str),
    Gesto(&'static str),
    Puntero(Option<(f32, f32)>),
    Pulsar,
    Salir,
}

#[derive(Debug)]
pub enum Evento {
    Entra(&'static str),
    Sale(&'static str),
    Pulsa(&'static str),
    Alarma(&'static str),
    Suceso(&'static str),
    /// Una capa ha cambiado de manos: (capa, quién gana ahora).
    Capa(&'static str, &'static str),
    /// Se pidió un gesto y había uno de más clase puesto.
    GestoRechazado(&'static str),
    /// El modo sin ratón: «haz lo siguiente que harías».
    Demo,
}

/// Una propiedad animada: posición, velocidad y adónde quiere ir.
#[derive(Clone, Copy)]
pub struct Animada {
    pub x: f32,
    pub v: f32,
    pub objetivo: f32,
    pub muelle: Muelle,
}

impl Animada {
    pub fn en(x: f32, muelle: Muelle) -> Self {
        Animada { x, v: 0.0, objetivo: x, muelle }
    }

    /// Euler semi-implícito en pasos de 2 ms como mucho: estable aunque un
    /// frame llegue tarde.
    pub fn avanzar(&mut self, dt: f32) {
        let pasos = (dt / 0.002).ceil().max(1.0);
        let h = dt / pasos;
        for _ in 0..pasos as u32 {
            let a = -self.muelle.rigidez * (self.x - self.objetivo) - self.muelle.freno * self.v;
            self.v += a * h;
            self.x += self.v * h;
        }
    }

    pub fn quieta(&self) -> bool {
        (self.x - self.objetivo).abs() < 0.02 && self.v.abs() < 0.08
    }

    pub fn posar(&mut self) {
        self.x = self.objetivo;
        self.v = 0.0;
    }

    /// Para las que escribe un comportamiento directamente.
    pub fn fijar(&mut self, x: f32) {
        self.x = x;
        self.objetivo = x;
        self.v = 0.0;
    }
}
