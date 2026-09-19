//! Las formas: lo que una escena declara (con expresiones) y lo que queda al
//! evaluarlas (números). La misma geometría sirve para pintar en la GPU, para
//! saber qué hay bajo el ratón y para calcular la caja de cada elemento.

use crate::escena::{Ctx, Expr, Punto};

/// Una transformación afín: p' = M·p + t. Las de un grupo y las de sus padres
/// se multiplican, así que girar algo dentro de algo que escala hace lo que se
/// espera.
#[derive(Clone, Copy, Debug)]
pub struct Afin {
    pub m: [f32; 4],
    pub t: [f32; 2],
}

impl Afin {
    pub const IDENTIDAD: Afin = Afin { m: [1.0, 0.0, 0.0, 1.0], t: [0.0, 0.0] };

    /// Alrededor de un pivote: primero escala, luego gira, luego mueve.
    /// Radianes, y positivo es en el sentido del reloj (la y crece hacia abajo).
    pub fn nueva(pivote: (f32, f32), giro: f32, escala: (f32, f32), mueve: (f32, f32)) -> Afin {
        let (s, c) = giro.sin_cos();
        let m = [c * escala.0, -s * escala.1, s * escala.0, c * escala.1];
        let t = [
            pivote.0 + mueve.0 - (m[0] * pivote.0 + m[1] * pivote.1),
            pivote.1 + mueve.1 - (m[2] * pivote.0 + m[3] * pivote.1),
        ];
        Afin { m, t }
    }

    /// `self ∘ o`: se aplica `o` y después `self`.
    pub fn por(self, o: Afin) -> Afin {
        let (a, b) = (self.m, o.m);
        Afin {
            m: [a[0] * b[0] + a[1] * b[2], a[0] * b[1] + a[1] * b[3], a[2] * b[0] + a[3] * b[2], a[2] * b[1] + a[3] * b[3]],
            t: [a[0] * o.t[0] + a[1] * o.t[1] + self.t[0], a[2] * o.t[0] + a[3] * o.t[1] + self.t[1]],
        }
    }

    pub fn inversa(self) -> Afin {
        let m = self.m;
        let det = m[0] * m[3] - m[1] * m[2];
        let det = if det.abs() < 1e-6 { 1e-6 } else { det };
        let i = [m[3] / det, -m[1] / det, -m[2] / det, m[0] / det];
        Afin { m: i, t: [-(i[0] * self.t[0] + i[1] * self.t[1]), -(i[2] * self.t[0] + i[3] * self.t[1])] }
    }

    pub fn aplicar(self, x: f32, y: f32) -> (f32, f32) {
        (self.m[0] * x + self.m[1] * y + self.t[0], self.m[2] * x + self.m[3] * y + self.t[1])
    }

    /// Cuánto estira las distancias. Exacto si la escala es igual en los dos
    /// ejes; si no, una media que basta para el suavizado del borde.
    pub fn factor(self) -> f32 {
        (self.m[0] * self.m[3] - self.m[1] * self.m[2]).abs().sqrt()
    }

    pub fn es_identidad(self) -> bool {
        self.m == Afin::IDENTIDAD.m && self.t == Afin::IDENTIDAD.t
    }

    /// La caja que contiene a otra una vez transformada.
    pub fn caja(self, c: [f32; 4]) -> [f32; 4] {
        if self.es_identidad() {
            return c;
        }
        let e = [self.aplicar(c[0], c[1]), self.aplicar(c[2], c[1]), self.aplicar(c[0], c[3]), self.aplicar(c[2], c[3])];
        let (mut r0, mut r1) = (e[0], e[0]);
        for p in e {
            r0 = (r0.0.min(p.0), r0.1.min(p.1));
            r1 = (r1.0.max(p.0), r1.1.max(p.1));
        }
        [r0.0, r0.1, r1.0, r1.1]
    }

    pub fn codificar(self, s: &mut [f32]) {
        let i = self.inversa();
        s[..8].copy_from_slice(&[i.m[0], i.m[1], i.m[2], i.m[3], i.t[0], i.t[1], self.factor(), 0.0]);
    }
}

#[derive(Clone, Debug)]
pub enum Forma {
    Elipse { centro: Punto, radio: Expr, escala: Punto },
    /// Caja redondeada. Con media anchura o altura por debajo de medio píxel,
    /// no existe: ni se pinta ni se funde con nada.
    Caja { centro: Punto, mitad: Punto, radio: Expr },
    /// Un arco como «∩». `apertura` es la mitad del ángulo que abarca, en
    /// radianes: π/2 es media circunferencia.
    Arco { centro: Punto, radio: Expr, apertura: Expr, grosor: Expr },
    /// Una línea de extremos redondos.
    Segmento { de: Punto, a: Punto, grosor: Expr },
    /// Girada sobre su centro. Radianes, y positivo es en el sentido del reloj.
    Girada(Box<Forma>, Expr),
    /// Solo el contorno: un círculo se vuelve un aro.
    Trazo(Box<Forma>, Expr),
}

impl Forma {
    pub fn circulo(centro: Punto, radio: impl Into<Expr>) -> Forma {
        Forma::Elipse { centro, radio: radio.into(), escala: (1.0.into(), 1.0.into()) }
    }
    pub fn aro(centro: Punto, radio: impl Into<Expr>, grosor: impl Into<Expr>) -> Forma {
        Forma::circulo(centro, radio).trazo(grosor)
    }
    pub fn girada(self, angulo: impl Into<Expr>) -> Forma {
        Forma::Girada(Box::new(self), angulo.into())
    }
    pub fn trazo(self, grosor: impl Into<Expr>) -> Forma {
        Forma::Trazo(Box::new(self), grosor.into())
    }

    pub fn aplanar(&self, c: Ctx) -> Plana {
        let mut p = Plana { tipo: 0, cx: 0.0, cy: 0.0, mx: 0.0, my: 0.0, radio: 0.0, giro: 0.0, ex: 1.0, ey: 1.0, trazo: 0.0, afin: Afin::IDENTIDAD };
        match self {
            Forma::Elipse { centro, radio, escala } => {
                (p.cx, p.cy) = (centro.0.evaluar(c), centro.1.evaluar(c));
                p.radio = radio.evaluar(c).max(0.0);
                (p.ex, p.ey) = (escala.0.evaluar(c).max(0.01), escala.1.evaluar(c).max(0.01));
            }
            Forma::Caja { centro, mitad, radio } => {
                p.tipo = 1;
                (p.cx, p.cy) = (centro.0.evaluar(c), centro.1.evaluar(c));
                (p.mx, p.my) = (mitad.0.evaluar(c).max(0.0), mitad.1.evaluar(c).max(0.0));
                p.radio = radio.evaluar(c).min(p.mx).min(p.my).max(0.0);
            }
            Forma::Arco { centro, radio, apertura, grosor } => {
                p.tipo = 2;
                (p.cx, p.cy) = (centro.0.evaluar(c), centro.1.evaluar(c));
                p.radio = radio.evaluar(c).max(0.0);
                p.ex = apertura.evaluar(c);
                p.trazo = grosor.evaluar(c).max(0.0);
            }
            Forma::Segmento { de, a, grosor } => {
                p.tipo = 3;
                (p.cx, p.cy) = (de.0.evaluar(c), de.1.evaluar(c));
                (p.mx, p.my) = (a.0.evaluar(c) - p.cx, a.1.evaluar(c) - p.cy);
                p.trazo = grosor.evaluar(c).max(0.0);
            }
            Forma::Girada(f, angulo) => {
                p = f.aplanar(c);
                p.giro += angulo.evaluar(c);
            }
            Forma::Trazo(f, grosor) => {
                p = f.aplanar(c);
                p.trazo = grosor.evaluar(c).max(0.0);
            }
        }
        p
    }

    #[allow(dead_code)]
    /// La misma distancia que calcula el shader, para saber si el ratón está
    /// dentro sin preguntarle a la GPU.
    pub fn distancia(&self, c: Ctx, x: f32, y: f32) -> f32 {
        self.aplanar(c).distancia(x, y)
    }
}

/// Una forma con sus expresiones ya evaluadas.
#[derive(Clone, Copy, Debug)]
pub struct Plana {
    pub tipo: u8,
    pub cx: f32,
    pub cy: f32,
    /// Caja: media anchura y altura. Segmento: el vector hasta el otro extremo.
    pub mx: f32,
    pub my: f32,
    pub radio: f32,
    pub giro: f32,
    /// Elipse: su escala. Arco: `ex` es la media apertura.
    pub ex: f32,
    pub ey: f32,
    pub trazo: f32,
    /// Lo heredado del grupo y de sus padres, ya multiplicado.
    pub afin: Afin,
}

fn girar(x: f32, y: f32, pivote: (f32, f32), angulo: f32) -> (f32, f32) {
    if angulo == 0.0 {
        return (x, y);
    }
    let (qx, qy) = (x - pivote.0, y - pivote.1);
    let (s, c) = angulo.sin_cos();
    (c * qx + s * qy + pivote.0, -s * qx + c * qy + pivote.1)
}

impl Plana {
    pub fn distancia(&self, x: f32, y: f32) -> f32 {
        let (x, y) = self.afin.inversa().aplicar(x, y);
        let (x, y) = girar(x, y, (self.cx, self.cy), self.giro);
        let (px, py) = (x - self.cx, y - self.cy);
        let mut d = match self.tipo {
            0 => ((px / self.ex).hypot(py / self.ey) - self.radio) * self.ex.min(self.ey),
            1 => {
                if self.mx < 0.5 || self.my < 0.5 {
                    return f32::MAX;
                }
                let (qx, qy) = (px.abs() - self.mx + self.radio, py.abs() - self.my + self.radio);
                qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - self.radio
            }
            2 => {
                let (s, c) = self.ex.sin_cos();
                let (qx, qy) = (px.abs(), -py);
                if c * qx > s * qy { (qx - s * self.radio).hypot(qy - c * self.radio) } else { (qx.hypot(qy) - self.radio).abs() }
            }
            _ => {
                let h = ((px * self.mx + py * self.my) / (self.mx * self.mx + self.my * self.my).max(0.0001)).clamp(0.0, 1.0);
                (px - self.mx * h).hypot(py - self.my * h)
            }
        };
        if self.trazo > 0.0 {
            d = if self.tipo >= 2 { d - self.trazo * 0.5 } else { d.abs() - self.trazo * 0.5 };
        }
        d * self.afin.factor()
    }

    /// Para recortar con un margen hacia dentro.
    pub fn encoger(mut self, m: f32) -> Plana {
        match self.tipo {
            0 => self.radio = (self.radio - m).max(0.0),
            1 => {
                self.mx = (self.mx - m).max(0.0);
                self.my = (self.my - m).max(0.0);
                self.radio = (self.radio - m).max(0.0);
            }
            _ => {}
        }
        self
    }

    /// La caja que seguro contiene a la forma. `None` si no existe.
    pub fn caja(&self) -> Option<[f32; 4]> {
        let medio = self.trazo * 0.5;
        let (mut hx, mut hy) = match self.tipo {
            0 => (self.radio * self.ex, self.radio * self.ey),
            1 => {
                if self.mx < 0.5 || self.my < 0.5 {
                    return None;
                }
                (self.mx, self.my)
            }
            2 => (self.radio, self.radio),
            _ => {
                let l = self.mx.hypot(self.my);
                (l, l)
            }
        };
        if self.giro != 0.0 {
            let r = hx.hypot(hy);
            (hx, hy) = (r, r);
        }
        let local = [self.cx - hx - medio, self.cy - hy - medio, self.cx + hx + medio, self.cy + hy + medio];
        Some(self.afin.caja(local))
    }

    pub fn codificar(&self, fusion: f32, s: &mut [f32]) {
        let (c2, c3) = if self.tipo == 2 { (self.ex, 0.0) } else { (self.ex, self.ey) };
        s[..12].copy_from_slice(&[
            self.tipo as f32, fusion, self.trazo, 0.0,
            self.cx, self.cy, self.mx, self.my,
            self.radio, self.giro, c2, c3,
        ]);
        self.afin.codificar(&mut s[12..20]);
    }
}
