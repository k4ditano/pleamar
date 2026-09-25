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

/// Los nombres de una escena viven lo que el programa, pero cada uno una sola
/// vez: recargar el mismo fichero cien veces no gasta más que la primera.
pub fn internar(s: &str) -> &'static str {
    use std::collections::HashSet;
    use std::sync::Mutex;
    static TABLA: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut t = TABLA.lock().unwrap();
    let tabla = t.get_or_insert_with(HashSet::new);
    if let Some(ya) = tabla.get(s) {
        return ya;
    }
    let nuevo: &'static str = Box::leak(s.to_owned().into_boxed_str());
    tabla.insert(nuevo);
    nuevo
}

// ── dónde vive la escena ────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ancla {
    Arriba,
    Abajo,
    Izquierda,
    Derecha,
    ArribaIzquierda,
    ArribaDerecha,
    AbajoIzquierda,
    AbajoDerecha,
    Centro,
}

impl Ancla {
    /// A qué bordes está pegada, en el orden de siempre: izquierda, arriba,
    /// derecha, abajo. Contra un borde pegado no hay sitio que pedir —una barra
    /// de arriba se sale por arriba porque quiere—, así que es donde no se
    /// avisa de que algo se corta.
    pub fn pegada(&self) -> [bool; 4] {
        let (i, a, d, b) = match self {
            Ancla::Arriba => (false, true, false, false),
            Ancla::Abajo => (false, false, false, true),
            Ancla::Izquierda => (true, false, false, false),
            Ancla::Derecha => (false, false, true, false),
            Ancla::ArribaIzquierda => (true, true, false, false),
            Ancla::ArribaDerecha => (false, true, true, false),
            Ancla::AbajoIzquierda => (true, false, false, true),
            Ancla::AbajoDerecha => (false, false, true, true),
            Ancla::Centro => (false, false, false, false),
        };
        [i, a, d, b]
    }
    pub fn de_palabra(p: &str) -> Option<Ancla> {
        Some(match p {
            "top" => Ancla::Arriba,
            "bottom" => Ancla::Abajo,
            "left" => Ancla::Izquierda,
            "right" => Ancla::Derecha,
            "top_left" => Ancla::ArribaIzquierda,
            "top_right" => Ancla::ArribaDerecha,
            "bottom_left" => Ancla::AbajoIzquierda,
            "bottom_right" => Ancla::AbajoDerecha,
            "center" => Ancla::Centro,
            _ => return None,
        })
    }
}

/// En qué capa del escritorio: detrás de las ventanas o delante.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Nivel {
    Fondo,
    Debajo,
    Encima,
    SobreTodo,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pantallas {
    /// Una superficie en cada monitor, y en los que se enchufen después.
    Todas,
    /// Solo en estos. Un nombre repetido da dos superficies en el mismo.
    Estas(Vec<String>),
    /// El monitor número k de los que haya, por orden. Es lo que hace `screens: each`:
    /// una superficie por monitor, cada una con su propio estado.
    Numero(usize),
}

/// La superficie que pide una escena. El tamaño es en píxeles lógicos: en un
/// monitor a escala 2 se pinta con el doble de píxeles de verdad.
#[derive(Clone, Debug)]
pub struct Superficie {
    /// Cómo se llama. La principal, «».
    pub nombre: String,
    /// Si sale de un `screens: each`: cuál de las copias es. La lógica y el dibujo la
    /// distinguen por ahí (`screen.name`, `$screen`).
    pub instancia: usize,
    /// Dónde se dibuja lo suyo, dentro del espacio de la escena: cada superficie mira a
    /// un trozo distinto del mismo plano, como las emergentes. Así todas comparten
    /// propiedades, hechos y reglas sin saber unas de otras.
    pub origen: (f32, f32),
    /// Está mientras este hecho sea verdad. Sin él, siempre.
    pub abierta: Option<Expr>,
    /// 0 es «todo el ancho del monitor».
    pub ancho: u32,
    pub alto: u32,
    pub ancla: Ancla,
    /// Si el borde al que se pega lo decide un hecho: cuál, y a qué ancla
    /// corresponde cada uno de sus valores. Layer-shell deja cambiarla en
    /// marcha, así que algo que tiene que elegir esquina —la cara de grabar de
    /// Marea escoge la que caiga fuera de lo grabado— no necesita cuatro
    /// superficies, una por esquina.
    pub ancla_de: Option<(HechoId, Vec<Ancla>)>,
    /// Arriba, derecha, abajo, izquierda.
    pub margen: [i32; 4],
    pub nivel: Nivel,
    /// Si es una ventana normal —de las que el compositor decora y coloca— en vez de
    /// un panel pegado al borde: cómo se titula. Sin esto, es un panel.
    pub ventana: Option<String>,
    /// Si es la pantalla de bloqueo: una superficie que NO existe hasta que su
    /// `open:` se hace verdad, y que al existir tiene la sesión bloqueada de
    /// verdad —lo garantiza el compositor, no el dibujo—, en todos los
    /// monitores a la vez. Cuando `open:` deja de ser verdad, se desbloquea.
    pub cerrojo: bool,
    /// Cuánto sitio le reserva el compositor: las ventanas no lo pisan.
    pub reserva: i32,
    /// Cuántos frames por segundo como MUCHO, decidido por quien escribe la
    /// escena y no por el monitor que toque. 0 es «los del monitor». Una barra
    /// que respira no necesita 165 frames por segundo, y pintarlos es lo que
    /// cuesta: con esto lo que consume es lo mismo en cualquier pantalla.
    pub ritmo: u32,
    pub pantallas: Pantallas,
    pub teclado: Teclado,
    /// El teclado solo se pide mientras se cumpla `Escena::teclado_mientras`.
    pub teclado_mientras: bool,
    /// Mientras ninguna regla use el botón derecho, cierra el programa.
    pub derecho_cierra: bool,
}

/// Si la superficie quiere el teclado. `AlPulsar` es lo normal en un panel con
/// algo que escribir; `Siempre` se lo queda entero, como un lanzador.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Teclado {
    #[default]
    Nunca,
    AlPulsar,
    Siempre,
}

impl Default for Superficie {
    fn default() -> Self {
        // Neutra: todo el ancho, arriba, en todos los monitores. Lo que pida la escena manda.
        Superficie { nombre: String::new(), instancia: 0, origen: (0.0, 0.0), abierta: None, ventana: None, cerrojo: false, ancho: 0, alto: 40, ancla: Ancla::Arriba, ancla_de: None, margen: [0; 4], nivel: Nivel::Encima, reserva: 0, ritmo: 0, pantallas: Pantallas::Todas, teclado: Teclado::Nunca, teclado_mientras: false, derecho_cierra: true }
    }
}

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
/// Un texto que la lógica puede cambiar: un hecho, pero de letras.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextoId(pub u16);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImagenId(pub u16);

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
    /// Redondeo hacia abajo o hacia arriba: hace falta para contar fichas.
    Suelo(Box<Expr>),
    /// Seno y coseno, en grados: lo que hace falta para poner algo en un arco.
    Seno(Box<Expr>),
    Coseno(Box<Expr>),
    Techo(Box<Expr>),
    /// smoothstep(a, b, x)
    Suave(f32, f32, Box<Expr>),
    /// `mix(a, b, t)` y también `if(c, x, y)`, que es mezclar con 0 o 1. Cada
    /// lado aparece una sola vez: escrito `a + (b - a) * t`, `a` salía dos, y
    /// una cadena de `if` doblaba el árbol en cada eslabón. Y con `t` justo en
    /// 0 o en 1, que es lo que da una condición, solo se evalúa un lado.
    Mezcla(Box<Expr>, Box<Expr>, Box<Expr>),
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
            Suelo(a) => a.evaluar(c).floor(),
            Seno(a) => a.evaluar(c).to_radians().sin(),
            Coseno(a) => a.evaluar(c).to_radians().cos(),
            Techo(a) => a.evaluar(c).ceil(),
            Suave(a, b, x) => {
                let t = ((x.evaluar(c) - a) / (b - a)).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            }
            Mezcla(a, b, t) => match t.evaluar(c) {
                0.0 => a.evaluar(c),
                1.0 => b.evaluar(c),
                t => {
                    let a = a.evaluar(c);
                    a + (b.evaluar(c) - a) * t
                }
            },
            Mayor(a, b) => (a.evaluar(c) > b.evaluar(c)) as u8 as f32,
            Y(a, b) => (a.evaluar(c) > 0.5 && b.evaluar(c) > 0.5) as u8 as f32,
            O(a, b) => (a.evaluar(c) > 0.5 || b.evaluar(c) > 0.5) as u8 as f32,
            No(a) => (a.evaluar(c) <= 0.5) as u8 as f32,
        }
    }
    pub fn es_verdad(&self, c: Ctx) -> bool {
        self.evaluar(c) > 0.5
    }
    /// Cuántos nodos tiene. Un `let` se sustituye donde se usa, así que esto es
    /// lo que cuesta cada vez que se nombra, y lo que decide si vale la pena
    /// calcularlo una sola vez.
    /// Si algún hecho de los que lee cumple la condición.
    pub fn lee(&self, f: impl Fn(HechoId) -> bool + Copy) -> bool {
        use Expr::*;
        match self {
            K(_) | P(_) | Vel(_) => false,
            H(h) => f(*h),
            Abs(a) | Suelo(a) | Seno(a) | Coseno(a) | Techo(a) | No(a) | Suave(_, _, a) => a.lee(f),
            Suma(a, b) | Resta(a, b) | Por(a, b) | Entre(a, b) | Min(a, b) | Max(a, b) | Mayor(a, b) | Y(a, b) | O(a, b) => a.lee(f) || b.lee(f),
            Mezcla(a, b, t) => a.lee(f) || b.lee(f) || t.lee(f),
        }
    }
    pub fn nodos(&self) -> usize {
        use Expr::*;
        match self {
            K(_) | P(_) | H(_) | Vel(_) => 1,
            Abs(a) | Suelo(a) | Seno(a) | Coseno(a) | Techo(a) | No(a) | Suave(_, _, a) => 1 + a.nodos(),
            Suma(a, b) | Resta(a, b) | Por(a, b) | Entre(a, b) | Min(a, b) | Max(a, b) | Mayor(a, b) | Y(a, b) | O(a, b) => 1 + a.nodos() + b.nodos(),
            Mezcla(a, b, t) => 1 + a.nodos() + b.nodos() + t.nodos(),
        }
    }
    /// Lo que vale, si no depende de nada.
    pub fn constante(&self) -> Option<f32> {
        match self {
            Expr::K(v) => Some(*v),
            _ => None,
        }
    }
    /// Lo que se puede saber al leer la escena se sabe una vez, no cada frame:
    /// `2 * 3` es `6`, `-4` es `-4` (y no `0 - 4`), `x * 1` es `x`. Los hijos
    /// ya vienen plegados, porque se construye de abajo arriba.
    fn plegada(self) -> Expr {
        use Expr::*;
        let k = |e: &Expr| e.constante();
        let hijos_constantes = match &self {
            K(_) | P(_) | H(_) | Vel(_) => return self,
            Abs(a) | Suelo(a) | Seno(a) | Coseno(a) | Techo(a) | No(a) | Suave(_, _, a) => k(a).is_some(),
            Suma(a, b) | Resta(a, b) | Por(a, b) | Entre(a, b) | Min(a, b) | Max(a, b) | Mayor(a, b) | Y(a, b) | O(a, b) => k(a).is_some() && k(b).is_some(),
            Mezcla(a, b, t) => k(a).is_some() && k(b).is_some() && k(t).is_some(),
        };
        if hijos_constantes {
            return K(self.evaluar(Ctx { props: &[], hechos: &[] }));
        }
        match self {
            Suma(a, b) if k(&b) == Some(0.0) => *a,
            Suma(a, b) if k(&a) == Some(0.0) => *b,
            Resta(a, b) if k(&b) == Some(0.0) => *a,
            Por(a, b) if k(&b) == Some(1.0) => *a,
            Por(a, b) if k(&a) == Some(1.0) => *b,
            Entre(a, b) if k(&b) == Some(1.0) => *a,
            Mezcla(a, b, t) => match k(&t) {
                Some(0.0) => *a,
                Some(1.0) => *b,
                _ => Mezcla(a, b, t),
            },
            otra => otra,
        }
    }
    pub fn mezcla(self, b: impl Into<Expr>, t: impl Into<Expr>) -> Expr {
        Expr::Mezcla(Box::new(self), Box::new(b.into()), Box::new(t.into())).plegada()
    }
    pub fn mayor(self, o: impl Into<Expr>) -> Expr {
        Expr::Mayor(Box::new(self), Box::new(o.into())).plegada()
    }
    pub fn y(self, o: impl Into<Expr>) -> Expr {
        Expr::Y(Box::new(self), Box::new(o.into())).plegada()
    }
    pub fn o(self, o: impl Into<Expr>) -> Expr {
        Expr::O(Box::new(self), Box::new(o.into())).plegada()
    }
    pub fn no(self) -> Expr {
        Expr::No(Box::new(self)).plegada()
    }
    pub fn min(self, o: impl Into<Expr>) -> Expr {
        Expr::Min(Box::new(self), Box::new(o.into())).plegada()
    }
    pub fn max(self, o: impl Into<Expr>) -> Expr {
        Expr::Max(Box::new(self), Box::new(o.into())).plegada()
    }
    pub fn acotar(self, a: f32, b: f32) -> Expr {
        self.max(a).min(b)
    }
    pub fn abs(self) -> Expr {
        Expr::Abs(Box::new(self)).plegada()
    }
    pub fn seno(self) -> Expr {
        Expr::Seno(Box::new(self)).plegada()
    }
    pub fn coseno(self) -> Expr {
        Expr::Coseno(Box::new(self)).plegada()
    }
    pub fn suelo(self) -> Expr {
        Expr::Suelo(Box::new(self)).plegada()
    }
    pub fn techo(self) -> Expr {
        Expr::Techo(Box::new(self)).plegada()
    }
    pub fn suave(self, a: f32, b: f32) -> Expr {
        Expr::Suave(a, b, Box::new(self)).plegada()
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
                Expr::$variante(Box::new(self), Box::new(o.into())).plegada()
            }
        }
        impl<T: Into<Expr>> $rasgo<T> for PropId {
            type Output = Expr;
            fn $metodo(self, o: T) -> Expr {
                Expr::$variante(Box::new(self.into()), Box::new(o.into())).plegada()
            }
        }
        impl $rasgo<Expr> for f32 {
            type Output = Expr;
            fn $metodo(self, o: Expr) -> Expr {
                Expr::$variante(Box::new(self.into()), Box::new(o)).plegada()
            }
        }
        impl $rasgo<PropId> for f32 {
            type Output = Expr;
            fn $metodo(self, o: PropId) -> Expr {
                Expr::$variante(Box::new(self.into()), Box::new(o.into())).plegada()
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

pub use crate::formas::{Afin, Forma, Paso};

#[derive(Clone, Debug)]
pub struct Sombra {
    /// Todo lo suyo son expresiones, como en el resto de la escena: una sombra
    /// que no puede cambiar obliga a elegir entre el halo que quiere una bolita
    /// y la sombra que quiere un panel, cuando son el mismo cuerpo.
    pub desplazada: (Expr, Expr),
    pub difusa: Expr,
    pub alfa: Expr,
    /// De qué color. Negra si no se dice, que es lo que una sombra es sobre
    /// papel; sobre un escritorio de ventanas oscuras, una sombra negra no
    /// tiene nada que oscurecer y lo que separa una cosa del fondo es un halo
    /// claro. Por eso se puede decir, y por eso es una expresión: puede ir
    /// cambiando de halo a sombra según lo que haya debajo.
    pub color: Option<Color>,
}

/// Un cristal: cuánto (`glass`, de 0 a 1) y si dobla lo de detrás como una
/// lente (`lens`, verdad o no) o solo lo deja ver desenfocado.
#[derive(Clone, Debug)]
pub struct Vidrio {
    pub cuanto: Expr,
    pub lente: Expr,
}

/// Un degradado vertical de claridad, para que el cuerpo no sea plano.
#[derive(Clone, Debug)]
pub struct Luz {
    pub cantidad: f32,
    pub desde_y: Expr,
    pub alto: f32,
}

pub type Color = [Expr; 3];

#[derive(Clone, Debug)]
pub enum Pintura {
    Color(Color),
    /// Un degradado de un punto a otro, o desde un centro hacia fuera. Las paradas
    /// van en orden, cada una con dónde cae (de 0 a 1) y de qué color es.
    Degradado { radial: bool, de: Punto, a: Punto, paradas: Vec<(Expr, Color)> },
}
impl From<Color> for Pintura {
    fn from(c: Color) -> Pintura {
        Pintura::Color(c)
    }
}

/// Girar, escalar y mover todo lo que venga después, alrededor de un punto. Se
/// compone con las que ya hubiera: la cabeza gira, y dentro de ella un ojo
/// puede girar por su cuenta.
#[derive(Clone, Debug)]
pub struct Transformacion {
    pub pivote: Punto,
    pub giro: Expr,
    pub escala: Punto,
    pub mueve: Punto,
}

impl Transformacion {
    pub fn en(pivote: Punto) -> Self {
        Transformacion { pivote, giro: 0.0.into(), escala: (1.0.into(), 1.0.into()), mueve: (0.0.into(), 0.0.into()) }
    }
    pub fn giro(mut self, radianes: impl Into<Expr>) -> Self {
        self.giro = radianes.into();
        self
    }
    pub fn escala(mut self, x: impl Into<Expr>, y: impl Into<Expr>) -> Self {
        self.escala = (x.into(), y.into());
        self
    }
    pub fn mueve(mut self, x: impl Into<Expr>, y: impl Into<Expr>) -> Self {
        self.mueve = (x.into(), y.into());
        self
    }
    pub fn afin(&self, c: Ctx) -> Afin {
        Afin::nueva(
            (self.pivote.0.evaluar(c), self.pivote.1.evaluar(c)),
            self.giro.evaluar(c),
            (self.escala.0.evaluar(c), self.escala.1.evaluar(c)),
            (self.mueve.0.evaluar(c), self.mueve.1.evaluar(c)),
        )
    }
}

// ── texto e imágenes ────────────────────────────────────────────

#[derive(Clone, Debug)]
pub enum Contenido {
    Fijo(String),
    /// Un número que sale de una expresión, con tantos decimales y lo que lleve detrás: `54 %`.
    Numero(Expr, u8, String),
    /// Lo que valga ahora mismo un texto vivo: lo cambia la lógica.
    Vivo(TextoId),
    /// Un texto con huecos: `"{r.title} · {volume * 100} %"`. Se monta en el render,
    /// cada vez que cambie cualquiera de sus partes.
    Plantilla(Vec<Trozo>),
}

#[derive(Clone, Debug)]
pub enum Trozo {
    Fijo(String),
    /// Un texto vivo, tal cual o pasado a mayúsculas o minúsculas.
    Vivo(TextoId, Letras),
    /// Una expresión, con tantos decimales.
    Numero(Expr, u8),
    /// Unos segundos, escritos como los escribe un reloj: `1:07`, y `1:02:07`
    /// cuando pasan de la hora.
    Duracion(Expr),
    /// El nombre de lo que valga un hecho enumerado: `{mode}` → `critical`.
    Nombre(Expr, Vec<String>),
    /// `{? · {r.body}}`: un tramo que solo está si ninguno de sus textos está vacío.
    Opcional(Vec<Trozo>),
    /// Un tramo que viene de fuera ya montado: el texto que se le pasó a un componente.
    Tramo(Vec<Trozo>),
    /// Un texto que se pasó vacío: no escribe nada, pero cuenta como vacío para un `{? …}`.
    Vacio,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Letras {
    Igual,
    Mayusculas,
    Minusculas,
}

impl Trozo {
    /// Escribe el tramo. Devuelve si estaba entero: si ningún texto vivo salió vacío.
    pub fn escribir(trozos: &[Trozo], c: Ctx, textos: &[String], en: &mut String) -> bool {
        use std::fmt::Write;
        let mut entero = true;
        for t in trozos {
            match t {
                Trozo::Fijo(s) => en.push_str(s),
                Trozo::Vivo(id, letras) => {
                    let s = textos.get(id.0 as usize).map_or("", String::as_str);
                    entero &= !s.is_empty();
                    match letras {
                        Letras::Igual => en.push_str(s),
                        Letras::Mayusculas => en.push_str(&s.to_uppercase()),
                        Letras::Minusculas => en.push_str(&s.to_lowercase()),
                    }
                }
                Trozo::Numero(e, decimales) => {
                    let _ = write!(en, "{:.*}", *decimales as usize, e.evaluar(c));
                }
                Trozo::Duracion(e) => {
                    let t = e.evaluar(c).max(0.0) as u64;
                    let _ = if t >= 3600 {
                        write!(en, "{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60)
                    } else {
                        write!(en, "{}:{:02}", t / 60, t % 60)
                    };
                }
                Trozo::Nombre(e, nombres) => {
                    if let Some(n) = nombres.get(e.evaluar(c).round().max(0.0) as usize) {
                        en.push_str(n);
                    }
                }
                Trozo::Tramo(dentro) => entero &= Trozo::escribir(dentro, c, textos, en),
                Trozo::Vacio => entero = false,
                Trozo::Opcional(dentro) => {
                    let desde = en.len();
                    if !Trozo::escribir(dentro, c, textos, en) {
                        en.truncate(desde);
                    }
                }
            }
        }
        entero
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Alineado {
    Izquierda,
    Centro,
    Derecha,
}

#[derive(Clone, Debug)]
pub struct Estilo {
    /// `None` es la sans del sistema, sea cual sea el sistema.
    pub familia: Option<&'static str>,
    pub px: f32,
    /// 400 normal, 500 medio, 700 negrita.
    pub peso: u16,
    pub color: Color,
    /// Alto de línea, en veces el tamaño.
    pub interlinea: f32,
    pub alineado: Alineado,
    /// A partir de estas líneas, puntos suspensivos.
    pub max_lineas: Option<usize>,
}

impl Estilo {
    pub fn de(px: f32, color: Color) -> Self {
        Estilo { familia: None, px, peso: 400, color, interlinea: 1.3, alineado: Alineado::Izquierda, max_lineas: None }
    }
    pub fn peso(mut self, p: u16) -> Self {
        self.peso = p;
        self
    }
    pub fn alineado(mut self, a: Alineado) -> Self {
        self.alineado = a;
        self
    }
    pub fn lineas(mut self, n: usize) -> Self {
        self.max_lineas = Some(n);
        self
    }
    pub fn familia(mut self, f: &'static str) -> Self {
        self.familia = Some(f);
        self
    }
}

/// De dónde sale una imagen.
#[derive(Clone, Debug)]
pub enum Fuente {
    Ruta(std::path::PathBuf),
    /// Un icono por su nombre («firefox»). Dónde encontrarlo lo sabe la plataforma.
    Icono(String),
    /// Lo que diga un texto vivo: el nombre de un icono, o una ruta si empieza
    /// por `/`. Es como la lógica elige una imagen: cambiando ese texto.
    Viva(TextoId),
}

pub fn color(r: f32, g: f32, b: f32) -> Color {
    [r.into(), g.into(), b.into()]
}

/// La lista de dibujo. El render la convierte en elementos —un quad cada uno—
/// y los pinta en orden.
#[derive(Clone, Debug)]
pub enum Instr {
    /// Empieza un cuerpo: a partir de aquí las formas se acumulan.
    Grupo { sombra: Option<Sombra> },
    /// Añade una forma al cuerpo, fundida con lo que lleve (`fusion` es el
    /// radio del mínimo suave; 0 es una unión seca).
    Forma { forma: Forma, fusion: Expr },
    /// Pinta el cuerpo acumulado: sombra, relleno, luz y filo. Con `vidrio`, de
    /// 0 a 1, el relleno se vuelve cristal: translúcido, y con la luz en los cantos.
    Relleno { pintura: Pintura, alfa: Expr, filo: f32, luz: Option<Luz>, borde: Option<(Expr, Color)>, vidrio: Option<Vidrio> },
    /// Todo lo que venga después se recorta a esta forma, además de a las que
    /// ya hubiera (hasta cuatro). `None` quita la última.
    Recorte(Option<(Forma, f32)>),
    /// Todo lo que venga después se transforma con esto, además de con lo que
    /// ya hubiera. `None` quita la última.
    Transformar(Option<Transformacion>),
    /// Todo lo que venga después se pinta aparte y se funde como UNA cosa con
    /// esta opacidad: lo de delante no deja ver lo de detrás a medio fundido.
    /// `None` cierra el grupo.
    Opacidad(Option<Expr>),
    /// Una forma suelta, de color plano.
    Plano { forma: Forma, color: Color, alfa: Expr, vidrio: Option<Vidrio> },
    /// Texto. `en` es el punto de referencia y `ancla` qué parte del texto cae
    /// sobre él: (0, 0) la esquina de arriba a la izquierda, (0.5, 0.5) el
    /// centro. Con `ancho` se parte en líneas; sin él, es una sola.
    /// `mide`, si se da, son dos propiedades donde el render deja lo que ocupa
    /// el texto: con ellas una caja puede crecer con su rótulo.
    Texto { contenido: Contenido, en: Punto, ancla: (f32, f32), ancho: Option<Expr>, estilo: Estilo, alfa: Expr, mide: Option<(PropId, PropId)> },
    /// Un campo donde escribir. Edita un texto vivo; el cursor, la selección y el
    /// eco de cada tecla los lleva el render, sin esperar a la lógica. `zona` es
    /// el nombre de la zona que lo enfoca al pulsarla.
    /// `secreto`: lo escrito se enseña como puntos, uno por letra. El texto de
    /// verdad sigue siendo el texto vivo; lo que cambia es lo que se pinta.
    Campo { texto: TextoId, zona: &'static str, en: Punto, ancho: Expr, estilo: Estilo, alfa: Expr, marcador: String, seleccion: Color, secreto: bool },
    /// Una imagen o un icono. Con `tinte`, su forma se pinta de ese color: lo
    /// que quiere un icono simbólico.
    Imagen { imagen: ImagenId, destino: (Expr, Expr, Expr, Expr), alfa: Expr, tinte: Option<Color> },
}

// ── lo que el render hace solo ──────────────────────────────────

#[derive(Clone, Debug)]
pub enum Comportamiento {
    /// Lleva la propiedad de 1 a 0 y de vuelta, de vez en cuando.
    Parpadeo { prop: PropId, cada: (f32, f32), dura: f32 },
    /// prop = amplitud · sin(frecuencia · t). Con amplitud 0 no cuesta nada.
    Onda { prop: PropId, frecuencia: f32, amplitud: Expr },
    /// La propiedad persigue a una expresión con su muelle. Es lo que en el
    /// lenguaje será `width: label.width + 24 ~lively`.
    Sigue { prop: PropId, a: Expr },
    /// prop = expresión, sin muelle. Para que algo que se calcula al final del
    /// dibujo —lo que ocupa un reparto— se pueda leer desde el principio.
    Es { prop: PropId, a: Expr },
    /// prop += por_segundo · dt, sin fin: una aguja que da vueltas.
    Avance { prop: PropId, por_segundo: Expr },
    /// Dos propiedades que tiran hacia el puntero, con su propio muelle.
    Mirada { x: PropId, y: PropId, centro: Punto, alcance: (f32, f32), distancia: f32, reposo: Punto },
}

#[derive(Clone, Debug)]
pub struct Transicion {
    pub prop: PropId,
    /// Adónde. Una expresión, que se evalúa cuando le llega la hora: `center - 220`.
    pub a: Expr,
    pub muelle: Muelle,
    pub retraso: Duration,
}

pub fn ir(prop: PropId, a: impl Into<Expr>, muelle: Muelle, retraso_ms: u64) -> Transicion {
    Transicion { prop, a: a.into(), muelle, retraso: Duration::from_millis(retraso_ms) }
}

/// Las teclas que acompañan a otra.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub mayus: bool,
    pub logo: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Cursor {
    #[default]
    Normal,
    Mano,
    Texto,
    Agarrar,
    Agarrando,
}

/// Una región sensible: una forma con nombre. El render hace el hit-test con
/// la misma fórmula con la que pinta.
#[derive(Clone, Debug)]
pub struct Zona {
    pub id: &'static str,
    pub forma: Forma,
    pub activa: Expr,
    /// Qué cursor se pone al pasar por encima.
    pub cursor: Cursor,
    /// Las transformaciones bajo las que vive, de fuera adentro: lo que se ve
    /// girado se pulsa girado.
    pub bajo: Vec<Transformacion>,
}

impl Zona {
    /// La caja que la contiene, para decirle al compositor por dónde entra el ratón.
    pub fn caja(&self, c: Ctx) -> Option<[f32; 4]> {
        let mut p = self.forma.aplanar_en(c, &mut Vec::new());
        p.afin = self.bajo.iter().fold(Afin::IDENTIDAD, |a, t| a.por(t.afin(c)));
        p.caja()
    }

    /// Un punto de la pantalla, visto desde dentro: en un reparto, (0, 0) es la
    /// esquina del hueco. Es lo que una regla lee en `local.x`.
    pub fn a_local(&self, c: Ctx, x: f32, y: f32) -> (f32, f32) {
        self.bajo.iter().fold(Afin::IDENTIDAD, |a, t| a.por(t.afin(c))).inversa().aplicar(x, y)
    }

    /// Un camino se mide contra sus puntos, como en la GPU: una zona con forma
    /// de flecha se pulsa donde se ve la flecha, no en su caja.
    pub fn contiene(&self, c: Ctx, x: f32, y: f32) -> bool {
        let mut pts = Vec::new();
        let mut p = self.forma.aplanar_en(c, &mut pts);
        p.afin = self.bajo.iter().fold(Afin::IDENTIDAD, |a, t| a.por(t.afin(c)));
        p.distancia_con(x, y, &pts) < 0.0
    }
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
    /// Expresiones, que se evalúan al empezar el gesto: así un mismo gesto
    /// puede señalar a un lado u otro según un hecho.
    pub valores: Vec<(PropId, Expr)>,
    pub emite: Option<SucesoId>,
}

pub fn foto(ms: u32, curva: Curva) -> Fotograma {
    Fotograma { ms, aguanta: 0, curva, valores: vec![], emite: None }
}
impl Fotograma {
    pub fn con(mut self, p: PropId, v: impl Into<Expr>) -> Self {
        self.valores.push((p, v.into()));
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
    /// El botón izquierdo.
    Pulsa(ZonaId),
    /// Otro botón: 1 es el derecho y 2 el del medio.
    PulsaCon(ZonaId, u8),
    /// Se suelta lo que se pulsó aquí, esté donde esté ya el ratón.
    Suelta(ZonaId),
    /// La rueda, encima. Cuánto, en `wheel`: +1 por muesca hacia arriba.
    Rueda(ZonaId),
    /// El ratón se mueve con el botón puesto desde que se pulsó aquí. Dónde, en
    /// `local.x` y `local.y`; cuánto desde que se pulsó, en `drag.dx` y `drag.dy`.
    Arrastra(ZonaId),
    /// Lleva este rato pulsada.
    Mantiene { zona: ZonaId, durante: Duration },
    /// Una tecla, por su nombre: `Escape`, `Return`, `a`, `Ctrl+k`.
    Tecla(String),
    /// Se ha pulsado Intro en un campo.
    Envia(TextoId),
    /// La superficie gana o pierde el teclado: perderlo es que han pulsado fuera.
    GanaFoco,
    PierdeFoco,
    /// Han soltado encima algo arrastrado desde otra aplicación.
    Recibe(ZonaId),
    /// El ratón lleva este rato encima.
    Encima { zona: ZonaId, durante: Duration },
    /// Ha estado encima y lleva este rato fuera.
    Fuera { zona: ZonaId, durante: Duration },
    /// Nadie ha tocado el ratón en este rato, mientras se cumpla la condición.
    Quieto { durante: Duration, mientras: Expr },
    /// De vez en cuando, con azar, mientras se cumpla la condición.
    Cada { entre: (f32, f32), mientras: Expr },
    /// Cuando esa cuenta deje de valer lo que valía. La primera vez no cuenta: se
    /// dispara al cambiar, no al nacer.
    Cambia(Expr),
    /// Cuando esa cuenta lleve este rato valiendo lo mismo. Es el reverso de
    /// `Cambia`, y como ella, nacer no cuenta: hace falta un cambio antes, o una
    /// escena que arranca quieta se dispararía sola al abrirse. Cada cambio
    /// vuelve a poner el reloj a cero, así que una ráfaga —la tecla de volumen
    /// pulsada seis veces— es una sola espera y no seis.
    Quieta { que: Expr, durante: Duration },
    Al(SucesoId),
}

#[derive(Clone, Debug)]
pub enum Efecto {
    Animar(Transicion),
    /// Un hecho pasa a valer lo que valga la expresión en ese momento. Puede
    /// leer lo del ratón: `level = clamp(local.x / 64, 0, 1)`.
    Hecho(HechoId, Expr),
    /// De sí a no y de no a sí.
    Alternar(HechoId),
    /// Un suceso, con una carga si se quiere: `emit opened(i)`. Se evalúa al dispararse.
    Suceso(SucesoId, Option<Expr>),
    /// Un empujón a la velocidad del muelle. La cantidad se evalúa al
    /// dispararse, así que puede depender de lo que esté pasando: el rebote de
    /// una cuenta atrás decrece con el número que queda.
    Impulso(PropId, Expr),
    Gesto(GestoId),
    /// Poner el cursor de texto en un campo, o quitarlo.
    Enfocar(Option<TextoId>),
}

/// Todo lo que hay aquí lo ejecuta el render, esté la lógica como esté.
#[derive(Clone, Debug)]
pub struct Regla {
    pub cuando: Disparador,
    /// `on press x while open`: solo si esto es verdad en el momento de dispararse.
    pub si: Option<Expr>,
    pub efectos: Vec<Efecto>,
}

pub fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[derive(Default)]
pub struct Escena {
    pub props: Vec<(&'static str, f32, Muelle)>,
    pub instrs: Vec<Instr>,
    pub comportamientos: Vec<Comportamiento>,
    pub zonas: Vec<Zona>,
    /// Nombre y valor inicial de cada texto vivo.
    pub textos: Vec<(&'static str, String)>,
    /// Cada imagen, y a qué tamaño lógico se pinta como mucho.
    pub imagenes: Vec<(Fuente, (u32, u32))>,
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
    /// Las ventanas que pide. La primera es la principal: la que manda si algo es de una sola.
    pub superficies: Vec<Superficie>,
    /// Ficheros que no son `.plm` y de los que también está hecha —los SVG de
    /// sus figuras—: tocarlos también la recarga.
    pub adjuntos: Vec<std::path::PathBuf>,
    /// `keyboard: exclusive while open`: cuándo quiere el teclado.
    pub teclado_mientras: Option<Expr>,
    pub permisos: Permisos,
    pub emergentes: Vec<Emergente>,
    pub modelos: Vec<Modelo>,
    /// De los hechos que no son números a secas, qué son. Por nombre.
    pub tipos: Vec<(String, TipoDeHecho)>,
    pub plugins: Vec<Plugin>,
    /// Los servicios que la escena pide por su nombre, y qué campos quiere de cada uno.
    pub servicios: Vec<Servicio>,
    /// Con `screens: each`, qué tramo de instrucciones, comportamientos y reglas
    /// es de cada copia (por el índice de su superficie). Una copia cuya
    /// superficie está cerrada no tiene nada que enseñar ni nada que mover:
    /// el render se salta lo suyo entero. Sin esto, dos copias eran el doble
    /// de escena por frame aunque una no se viera.
    pub tramos: Vec<Tramo>,
}

#[derive(Clone, Debug, Default)]
pub struct Tramo {
    pub superficie: usize,
    pub instrs: std::ops::Range<usize>,
    pub comportamientos: std::ops::Range<usize>,
    pub reglas: std::ops::Range<usize>,
}

/// Una biblioteca con lógica propia. Su frontera —hechos, textos, modelos, sucesos—
/// vive bajo su nombre (`Clock.now`), su lógica corre en su propio estado de Luau, solo
/// puede nombrar lo suyo, y lo que toque del sistema lo dicen **sus** permisos.
#[derive(Clone, Debug, PartialEq)]
pub struct Plugin {
    pub nombre: String,
    pub logica: std::path::PathBuf,
    pub permisos: Permisos,
}

/// Datos con forma que cruzan la frontera: una lista de fichas, todas con los
/// mismos campos. La lógica la entrega entera (`model.rows = lista`) y la escena
/// la recorre (`for r in rows`). Por dentro, cada campo de cada ficha es un texto
/// vivo o un hecho con nombre —`rows.3.label`—: el render no sabe que hay listas.
#[derive(Clone, Debug, PartialEq)]
pub struct Modelo {
    pub nombre: String,
    /// Cuántas fichas caben. Lo que pase de ahí no se ve, pero se cuenta: `rows.total`.
    pub caben: usize,
    pub campos: Vec<Campo>,
}

/// Un servicio del sistema pedido desde la escena: lo que llegue rellena
/// `alias.campo` sin que nadie escriba lógica. `service clock as now { time: text }`.
#[derive(Clone, Debug, PartialEq)]
pub struct Servicio {
    /// Como lo conoce la plataforma: `clock`, `audio`, `battery`, `network`, `media`.
    pub nombre: String,
    /// Delante de cada campo: `now.time`.
    pub alias: String,
    pub campos: Vec<Campo>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Campo {
    pub nombre: String,
    pub tipo: TipoDeCampo,
    /// Lo que vale si la ficha no lo trae.
    pub por_defecto: ValorDeCampo,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TipoDeCampo {
    Texto,
    Numero,
    /// Como `Numero`, pero solo 0 o 1, y la lógica lo escribe con `true` y `false`.
    Bool,
    /// Uno de estos nombres: `low | normal | critical`. Por dentro, su posición.
    Enum(Vec<String>),
    /// El nombre de un icono o una ruta, y la imagen que eso diga, a este tamaño.
    Imagen(u32, u32),
    /// Una lista de fichas dentro de la ficha: un menú con sus submenús.
    Lista(Box<Modelo>),
}

/// Lo que es un hecho que no es un número a secas. La escena solo ve números; esto
/// es para quien los pone y los lee desde fuera —la lógica, `--decir`, un hueco de un
/// texto—, que habla de `true` y de `critical`, no de 1 y de 2.
#[derive(Clone, Debug, PartialEq)]
pub enum TipoDeHecho {
    Bool,
    Enum(Vec<String>),
}

impl TipoDeHecho {
    /// Como se le enseña a alguien: `true`, `critical`.
    pub fn como_texto(&self, v: f32) -> String {
        match self {
            TipoDeHecho::Bool => (v > 0.5).to_string(),
            TipoDeHecho::Enum(nombres) => nombres.get(v.round().max(0.0) as usize).cloned().unwrap_or_else(|| v.to_string()),
        }
    }
    /// Y al revés: lo que alguien escribió, como número.
    pub fn de_texto(&self, t: &str) -> Option<f32> {
        match self {
            TipoDeHecho::Bool => match t { "true" => Some(1.0), "false" => Some(0.0), _ => None },
            TipoDeHecho::Enum(nombres) => nombres.iter().position(|n| n == t).map(|k| k as f32),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ValorDeCampo {
    Texto(String),
    Numero(f32),
}

/// Una superficie que sale de la principal: un menú, una ficha. Lo que pinta
/// es un trozo más de la escena —mismos muelles, mismas zonas, mismas reglas—,
/// dibujado lejos, en `origen`; la superficie emergente es una ventana a ese trozo.
#[derive(Clone, Debug)]
pub struct Emergente {
    pub nombre: &'static str,
    /// Está abierta mientras este hecho sea verdad. Si el sistema la cierra
    /// (han pulsado fuera), el render lo pone a falso.
    pub abierta: HechoId,
    /// Dónde sale, dentro de la superficie principal; y cuánto mide.
    pub en: (Expr, Expr),
    pub tam: (Expr, Expr),
    pub origen: (f32, f32),
}

/// Lo que la lógica de esta escena puede tocar del sistema. **Sin declarar,
/// nada**: ni una orden, ni un servicio. Está en la escena y no en el script
/// para que se lea de un vistazo, antes de ejecutar nada, qué va a poder hacer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Permisos {
    /// Las órdenes que `run` y `spawn` pueden lanzar, por su nombre exacto.
    pub ordenes: Vec<String>,
    /// Los servicios que `sys.watch` y `sys.call` pueden usar: `audio`, `apps`…
    pub servicios: Vec<String>,
}

impl Escena {
    /// La principal: la que se declara sin nombre, o la primera.
    pub fn superficie(&self) -> &Superficie {
        self.superficies.first().expect("every scene has at least one surface")
    }
    pub fn superficie_mut(&mut self) -> &mut Superficie {
        if self.superficies.is_empty() {
            self.superficies.push(Superficie::default());
        }
        &mut self.superficies[0]
    }

    /// Las propiedades van por nombre: si la escena se sustituye por otra, las
    /// que se llamen igual conservan valor y velocidad.
    pub fn prop(&mut self, nombre: &'static str, inicial: f32) -> PropId {
        self.prop_con(nombre, inicial, Muelle::VIVO)
    }
    pub fn prop_con(&mut self, nombre: &'static str, inicial: f32, muelle: Muelle) -> PropId {
        self.props.push((nombre, inicial, muelle));
        PropId(self.props.len() as u16 - 1)
    }
    /// El muelle con el que se declaró una propiedad. Es el suyo: una regla que
    /// la manda a un sitio sin decir con qué muelle va con este, que es lo que
    /// dice `prop x = 0 ~620ms` al escribirse.
    pub fn muelle_de(&self, p: PropId) -> Muelle {
        self.props[p.0 as usize].2
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
    /// Dos propiedades de solo lectura —ancho y alto— que el render rellena con
    /// lo que mida un texto.
    pub fn medida(&mut self, nombre: &'static str) -> (PropId, PropId) {
        let n = |sufijo: &str| internar(&format!("{nombre}.{sufijo}"));
        (self.prop(n("ancho"), 0.0), self.prop(n("alto"), 0.0))
    }
    pub fn texto_vivo(&mut self, nombre: &'static str, inicial: &str) -> TextoId {
        self.textos.push((nombre, inicial.to_owned()));
        TextoId(self.textos.len() as u16 - 1)
    }
    pub fn imagen(&mut self, fuente: Fuente, ancho: u32, alto: u32) -> ImagenId {
        self.imagenes.push((fuente, (ancho, alto)));
        ImagenId(self.imagenes.len() as u16 - 1)
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
        self.zona_bajo(id, forma, activa, vec![])
    }
    pub fn zona_bajo(&mut self, id: &'static str, forma: Forma, activa: impl Into<Expr>, bajo: Vec<Transformacion>) -> ZonaId {
        self.zonas.push(Zona { id, forma, activa: activa.into(), cursor: Cursor::Normal, bajo });
        ZonaId(self.zonas.len() as u16 - 1)
    }
    /// Las reclamaciones van de más a menos prioridad; la última debería ser
    /// `por_defecto`.
    pub fn capa(&mut self, nombre: &'static str, muelle: Muelle, reclamaciones: Vec<Reclamacion>) -> CapaRef {
        let presencias: Vec<PropId> = reclamaciones
            .iter()
            .map(|r| {
                let n = internar(&format!("capa.{nombre}.{}", r.nombre));
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
        self.reglas.push(Regla { cuando, si: None, efectos });
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
    pub const SUAVE: Muelle = Muelle { rigidez: 190.0, freno: 24.0 };
    pub const POSE: Muelle = Muelle { rigidez: 260.0, freno: 28.0 };

    /// El muelle que llega en ese rato sin pasarse. Críticamente amortiguado
    /// (`freno = 2 · √rigidez`), que es el que no rebota; con eso, llegar al 99 %
    /// tarda unas 6,64 constantes de tiempo, y de ahí sale la rigidez.
    pub fn en(segundos: f32) -> Muelle {
        let w = 6.64 / segundos.max(0.016);
        Muelle { rigidez: w * w, freno: 2.0 * w }
    }
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
    /// Una recarga que no cuela. Un runtime que se recarga al guardar no puede
    /// contarlo solo por una consola que a lo mejor nadie mira: se enseña en la
    /// propia superficie, que es donde ya está mirando quien lo escribe. `None`
    /// la quita: la escena ha vuelto a estar bien.
    FalloDeRecarga(Option<String>),
    /// Una superficie más donde pintar: un monitor que ya estaba o que acaban
    /// de enchufar.
    Lamina(Box<crate::gpu::NuevaLamina>),
    LaminaFuera(u32),
    /// El compositor ya enseñó el último frame de esa lámina y quiere otro.
    Frame(u32),
    /// Lo que se veía en pantalla detrás de un cristal, con él encima.
    Detras(Box<crate::plataforma::Detras>),
    /// El taller ha terminado algo: una maqueta, unas imágenes.
    Taller(Box<crate::texto::Paquete>),
    Escala(u32, f32),
    /// Una superficie ha cambiado de tamaño: una ventana que alguien estira.
    TamLamina(u32, (f32, f32)),
    Orden(Orden),
    /// La frontera: la lógica cuenta lo que pasa, y nada más.
    Hecho(&'static str, f32),
    Texto(&'static str, String),
    Suceso(&'static str),
    /// Un suceso que viene de fuera del programa: lo oyen la escena y la lógica.
    SucesoDeFuera(&'static str, Option<f32>),
    Gesto(&'static str),
    Puntero(Option<(f32, f32)>),
    /// 0 es el izquierdo, 1 el derecho, 2 el del medio.
    Boton(u8, bool),
    /// Muescas de rueda: positivo, hacia arriba.
    Rueda(f32),
    /// El nombre de la tecla, lo que escribe si escribe algo, y con qué iba pulsada.
    Tecla(String, Option<String>, Mods),
    TeclaSuelta(String),
    /// La superficie ha ganado o perdido el teclado.
    FocoTeclado(bool),
    /// Poner el cursor de texto en un campo, o quitarlo de donde esté.
    Enfocar(Option<&'static str>),
    /// Cómo repite las teclas este usuario: a los cuántos ms empieza y cada cuántos
    /// sigue. `None`: las tiene sin repetición. Lo dice el sistema; si calla, 400 y 33.
    Repeticion(Option<(u32, u32)>),
    /// Un hecho puesto desde fuera, como se escribió: `true`, `critical`, `3`. El render sabe de qué tipo es.
    HechoDeFuera(&'static str, String),
    /// El sistema ha cerrado una emergente: han pulsado fuera de ella.
    EmergenteCerrada(usize),
    /// Lo que dice el compositor del bloqueo: `true`, la sesión ESTÁ bloqueada
    /// (y no antes); `false`, no lo ha concedido o lo ha dado por terminado.
    Cerrojo(bool),
    /// Desde fuera preguntan cuánto vale un hecho, un texto o una propiedad.
    Pregunta(&'static str, std::sync::mpsc::Sender<String>),
    /// Han soltado algo encima, arrastrado desde otra aplicación: (tipo, contenido).
    Soltado(String, String),
    Salir,
}

/// Los textos que edita un campo `secret: true`. Lo que lleven no se escribe
/// en ningún log, ni en el eco de los ensayos: es una contraseña. Es de todo el
/// proceso porque quien hace el eco —la lógica— no tiene la escena delante.
pub static SECRETOS: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());

#[derive(Clone, Debug)]
pub enum Evento {
    Entra(&'static str),
    Sale(&'static str),
    Pulsa(&'static str),
    Suelta(&'static str),
    Rueda(&'static str, f32),
    Tecla(String, Option<String>),
    /// Lo que pone ahora un campo, tecla a tecla.
    Texto(&'static str, String),
    /// Intro en un campo.
    Envia(&'static str, String),
    Foco(bool),
    /// Algo soltado sobre una zona: (zona, tipo, contenido).
    Recibido(&'static str, String, String),
    Alarma(&'static str),
    /// Un suceso de la escena que sale hacia la lógica, con su carga si la trae.
    Suceso(&'static str, Option<f32>),
    /// Una regla ha cambiado un hecho: la lógica lleva la cuenta de lo que es verdad.
    Hecho(&'static str, f32),
    /// Una orden que se lanzó ha terminado: (cuál, lo que escribió, con qué código).
    Proceso(u32, String, i32),
    /// Una orden que sigue en marcha ha escrito una línea.
    Linea(u32, String),
    /// Un servicio del sistema tiene algo nuevo que contar.
    Dato(String, crate::plataforma::Valor),
    /// El fichero de la lógica ha cambiado.
    RecargarLogica,
    /// La escena se ha recargado: estos son ahora sus hechos y sus textos.
    EscenaNueva(Vec<(&'static str, f32)>, Vec<(&'static str, String)>, Permisos, Vec<Modelo>, Vec<(String, TipoDeHecho)>, Vec<Plugin>, Vec<&'static str>, Vec<Servicio>),
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

#[cfg(test)]
mod pruebas {
    use super::*;

    fn con(props: &[f32]) -> Vec<Animada> {
        props.iter().map(|&x| Animada { x, v: 0.0, objetivo: x, muelle: Muelle::VIVO }).collect()
    }

    #[test]
    fn lo_constante_se_pliega_al_leer() {
        assert!(matches!((Expr::K(2.0) * 3.0 + 1.0), Expr::K(7.0)));
        assert!(matches!(Expr::K(0.0) - Expr::K(4.0), Expr::K(-4.0)));
        assert!(matches!(PropId(0) * 1.0, Expr::P(_)));
        assert!(matches!(PropId(0) + 0.0, Expr::P(_)));
        assert!(matches!(Expr::K(3.0).mezcla(PropId(0), 0.0), Expr::K(3.0)));
        assert!(matches!(Expr::K(3.0).mezcla(PropId(0), 1.0), Expr::P(_)));
    }

    #[test]
    fn mezclar_da_lo_mismo_que_antes() {
        let props = con(&[0.25, 1.0, 0.0]);
        let c = Ctx { props: &props, hechos: &[] };
        let (a, b) = (Expr::K(10.0), Expr::K(20.0));
        for p in 0..3 {
            let t = props[p].x;
            assert_eq!(a.clone().mezcla(b.clone() + PropId(2), PropId(p as u16)).evaluar(c), 10.0 + 10.0 * t);
        }
    }

    #[test]
    fn una_cadena_de_if_crece_en_linea() {
        let mut e = Expr::K(1.0);
        for k in 0..30 {
            e = e.mezcla(Expr::K(k as f32), PropId(0).e().mayor(k as f32));
        }
        assert!(e.nodos() < 200, "{} nodos", e.nodos());
    }
}
