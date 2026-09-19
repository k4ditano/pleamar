//! Las formas: lo que una escena declara (con expresiones) y lo que queda al
//! evaluarlas (números). La misma geometría sirve para pintar en la GPU, para
//! saber qué hay bajo el ratón y para calcular la caja de cada elemento.

use crate::escena::{Ctx, Expr, Punto};

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
        let mut p = Plana { tipo: 0, cx: 0.0, cy: 0.0, mx: 0.0, my: 0.0, radio: 0.0, giro: 0.0, ex: 1.0, ey: 1.0, trazo: 0.0, pivote: (0.0, 0.0), giro_heredado: 0.0 };
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
    pub pivote: (f32, f32),
    pub giro_heredado: f32,
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
        let (x, y) = girar(x, y, self.pivote, self.giro_heredado);
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
        d
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
        let (mut cx, mut cy) = (self.cx, self.cy);
        if self.giro_heredado != 0.0 {
            // Girar el punto de muestreo por −θ es girar la forma por +θ.
            (cx, cy) = girar(cx, cy, self.pivote, -self.giro_heredado);
            let r = hx.hypot(hy);
            (hx, hy) = (r, r);
        }
        Some([cx - hx - medio, cy - hy - medio, cx + hx + medio, cy + hy + medio])
    }

    pub fn codificar(&self, fusion: f32, s: &mut [f32]) {
        let (c2, c3) = if self.tipo == 2 { (self.ex, 0.0) } else { (self.ex, self.ey) };
        s.copy_from_slice(&[
            self.tipo as f32, fusion, self.trazo, 0.0,
            self.cx, self.cy, self.mx, self.my,
            self.radio, self.giro, c2, c3,
            self.pivote.0, self.pivote.1, self.giro_heredado, 0.0,
        ]);
    }
}
