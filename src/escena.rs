//! El contrato entre quien piensa y quien pinta.
//!
//! Una escena es *datos*: propiedades con nombre, una lista de instrucciones de
//! dibujo cuyos números son expresiones sobre esas propiedades, comportamientos
//! que el render lleva solo (parpadear, mirar, respirar) y zonas sensibles al
//! ratón. El render no sabe qué es una bolita: evalúa, pinta y avisa.
//!
//! La lógica nunca manda valores: manda *intenciones* («esta propiedad va a 406
//! con este muelle, dentro de 60 ms»).

use std::ops::{Add, Div, Mul, Sub};
use std::time::Duration;

pub const ANCHO: u32 = 720;
pub const ALTO: u32 = 300;
pub const MAX_INSTR: usize = 64;

// ── propiedades y expresiones ───────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropId(pub u16);

/// Una expresión pura sobre las propiedades. Como no tiene efectos, el render
/// puede evaluarla cuando quiera y donde quiera: es lo que sería un binding.
#[derive(Clone, Debug)]
pub enum Expr {
    K(f32),
    P(PropId),
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
}

impl Expr {
    pub fn evaluar(&self, props: &[Animada]) -> f32 {
        use Expr::*;
        match self {
            K(v) => *v,
            P(p) => props[p.0 as usize].x,
            Vel(p) => props[p.0 as usize].v,
            Suma(a, b) => a.evaluar(props) + b.evaluar(props),
            Resta(a, b) => a.evaluar(props) - b.evaluar(props),
            Por(a, b) => a.evaluar(props) * b.evaluar(props),
            Entre(a, b) => a.evaluar(props) / b.evaluar(props),
            Min(a, b) => a.evaluar(props).min(b.evaluar(props)),
            Max(a, b) => a.evaluar(props).max(b.evaluar(props)),
            Abs(a) => a.evaluar(props).abs(),
            Suave(a, b, x) => {
                let t = ((x.evaluar(props) - a) / (b - a)).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            }
        }
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
    pub fn distancia(&self, props: &[Animada], x: f32, y: f32) -> f32 {
        match self {
            Forma::Elipse { centro, radio, escala } => {
                let (ex, ey) = (escala.0.evaluar(props), escala.1.evaluar(props));
                let (qx, qy) = ((x - centro.0.evaluar(props)) / ex, (y - centro.1.evaluar(props)) / ey);
                (qx.hypot(qy) - radio.evaluar(props)) * ex.min(ey)
            }
            Forma::Caja { centro, mitad, radio } => {
                let (mx, my) = (mitad.0.evaluar(props), mitad.1.evaluar(props));
                if mx < 0.5 || my < 0.5 {
                    return f32::MAX;
                }
                let r = radio.evaluar(props);
                let qx = (x - centro.0.evaluar(props)).abs() - mx + r;
                let qy = (y - centro.1.evaluar(props)).abs() - my + r;
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

/// Una región sensible. Lo declarado en `al_entrar` y `al_salir` lo ejecuta el
/// render en el acto, esté la lógica como esté —es el `:hover` de CSS—; además
/// avisa a la lógica por nombre, que ya no sabe de coordenadas.
#[derive(Clone, Debug)]
pub struct Zona {
    pub id: &'static str,
    pub forma: Forma,
    pub activa: Expr,
    pub al_entrar: Vec<Transicion>,
    pub al_salir: Vec<Transicion>,
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
