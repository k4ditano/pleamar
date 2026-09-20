//! Un SVG que entra como **geometría**, no como estampa.
//!
//! Una imagen se rasteriza a un atlas: se ve, pero es un sello. No se funde con
//! nada, no se tiñe por partes, no se anima por capas y al escalar es píxeles.
//! Un accesorio —un gorro que se apoya en ella, unas gafas pegadas a su cara—
//! necesita justo lo contrario, así que de un `.svg` se sacan sus caminos y
//! pasan a ser las mismas distancias con signo que todo lo demás: se funden con
//! `blend`, llevan filo y sombra, y cada capa gira sobre su pivote.
//!
//! El lector es `usvg`, que ya está dentro por las imágenes, así que esto no
//! trae ni una dependencia nueva. Lo que sí trae es un límite: de un SVG se
//! entienden **caminos rellenos y trazados**, con su color. Degradados,
//! máscaras, filtros y texto no, y se dice en voz alta en vez de fingirlos.

use crate::escena::{Forma, Paso, Punto};

/// Un camino del SVG, ya en geometría de la escena.
pub struct Trazo {
    pub forma: Forma,
    pub color: [f32; 3],
    pub alfa: f32,
    /// 0 si está rellena; si no, el grosor de su trazo.
    pub grosor: f32,
}

/// Una capa: lo que en el editor es un grupo con nombre.
pub struct Capa {
    pub nombre: String,
    pub trazos: Vec<Trazo>,
}

pub struct Figura {
    pub capas: Vec<Capa>,
    /// Lo que mide su lienzo, para poder decir «píntala de 44 × 34».
    pub tam: (f32, f32),
}

impl Figura {
    pub fn capa(&self, nombre: &str) -> Option<&Capa> {
        self.capas.iter().find(|c| c.nombre == nombre)
    }
    pub fn nombres(&self) -> Vec<&str> {
        self.capas.iter().map(|c| c.nombre.as_str()).collect()
    }
}

/// Lo que cuesta pintar un camino es su lista de puntos, y la lista sale de
/// partir sus curvas. Se cuenta aquí para poder decir «esa capa tiene 300
/// puntos» al cargar, y no descubrirlo con media pieza dibujada.
fn cuantos_puntos(origen: &Punto, pasos: &[Paso]) -> usize {
    let val = |e: &crate::escena::Expr| match e {
        crate::escena::Expr::K(k) => *k,
        _ => 0.0,
    };
    let mut d = (val(&origen.0), val(&origen.1));
    let mut n = 1;
    for p in pasos {
        match p {
            Paso::Linea(a) => {
                d = (val(&a.0), val(&a.1));
                n += 1;
            }
            Paso::Curva { via, a } => {
                let v = (val(&via.0), val(&via.1));
                let f = (val(&a.0), val(&a.1));
                let largo = (v.0 - d.0).hypot(v.1 - d.1) + (f.0 - v.0).hypot(f.1 - v.1);
                n += ((largo / 4.0) as usize).clamp(3, 24);
                d = f;
            }
        }
    }
    n
}

fn punto(x: f32, y: f32, centro: (f32, f32)) -> Punto {
    ((x - centro.0).into(), (y - centro.1).into())
}

/// Un cúbico de SVG en dos cuadráticos, que es lo que la escena sabe dibujar.
/// Partirlo por la mitad y aproximar cada mitad deja el error muy por debajo
/// del píxel al tamaño al que vive un accesorio.
fn cubico(p0: (f32, f32), c1: (f32, f32), c2: (f32, f32), p3: (f32, f32)) -> [((f32, f32), (f32, f32)); 2] {
    let med = |a: (f32, f32), b: (f32, f32)| ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
    let (a, b, c) = (med(p0, c1), med(c1, c2), med(c2, p3));
    let (d, e) = (med(a, b), med(b, c));
    let m = med(d, e);
    // El control de un cuadrático que se parece a un cúbico: (3(b+c) − (a+d))/4.
    let ctrl = |p0: (f32, f32), c1: (f32, f32), c2: (f32, f32), p3: (f32, f32)| {
        ((3.0 * (c1.0 + c2.0) - (p0.0 + p3.0)) * 0.25, (3.0 * (c1.1 + c2.1) - (p0.1 + p3.1)) * 0.25)
    };
    [(ctrl(p0, a, d, m), m), (ctrl(m, e, c, p3), p3)]
}

/// De un camino de `tiny_skia` a los caminos de la escena: uno por trazo
/// suelto, porque aquí un `path` empieza en un sitio y solo en uno.
fn caminos(datos: &resvg::tiny_skia::Path, centro: (f32, f32)) -> Vec<Forma> {
    use resvg::tiny_skia::PathSegment as S;
    let mut fuera = Vec::new();
    let (mut origen, mut pasos, mut cerrado) = (None, Vec::new(), false);
    let mut aqui = (0.0f32, 0.0f32);
    let cerrar = |origen: &mut Option<Punto>, pasos: &mut Vec<Paso>, cerrado: &mut bool, fuera: &mut Vec<Forma>| {
        if let (Some(o), false) = (origen.take(), pasos.is_empty()) {
            fuera.push(Forma::Camino { origen: o, pasos: std::mem::take(pasos), cerrado: *cerrado });
        }
        pasos.clear();
        *cerrado = false;
    };
    for s in datos.segments() {
        match s {
            S::MoveTo(p) => {
                cerrar(&mut origen, &mut pasos, &mut cerrado, &mut fuera);
                aqui = (p.x, p.y);
                origen = Some(punto(p.x, p.y, centro));
            }
            S::LineTo(p) => {
                aqui = (p.x, p.y);
                pasos.push(Paso::Linea(punto(p.x, p.y, centro)));
            }
            S::QuadTo(v, p) => {
                aqui = (p.x, p.y);
                pasos.push(Paso::Curva { via: punto(v.x, v.y, centro), a: punto(p.x, p.y, centro) });
            }
            S::CubicTo(c1, c2, p) => {
                for (v, a) in cubico(aqui, (c1.x, c1.y), (c2.x, c2.y), (p.x, p.y)) {
                    pasos.push(Paso::Curva { via: punto(v.0, v.1, centro), a: punto(a.0, a.1, centro) });
                }
                aqui = (p.x, p.y);
            }
            S::Close => cerrado = true,
        }
    }
    cerrar(&mut origen, &mut pasos, &mut cerrado, &mut fuera);
    fuera
}

fn color_de(pintura: &resvg::usvg::Paint) -> Option<[f32; 3]> {
    match pintura {
        resvg::usvg::Paint::Color(c) => Some([c.red as f32 / 255.0, c.green as f32 / 255.0, c.blue as f32 / 255.0]),
        // Un degradado o un patrón no tienen un color: que lo diga quien lo use.
        _ => None,
    }
}

fn recorrer(grupo: &resvg::usvg::Group, capa: Option<&str>, centro: (f32, f32), capas: &mut Vec<Capa>, aviso: &mut Vec<String>) {
    for hijo in grupo.children() {
        match hijo {
            resvg::usvg::Node::Group(g) => {
                // El nombre de una capa es el `id` del grupo. Dentro de una capa
                // ya nombrada, los grupos de abajo son suyos: lo que se dibuja
                // en el editor dentro de una capa es de esa capa.
                let mio = capa.map(str::to_owned).or_else(|| (!g.id().is_empty()).then(|| g.id().to_owned()));
                recorrer(g, mio.as_deref(), centro, capas, aviso);
            }
            resvg::usvg::Node::Path(p) => {
                let nombre = capa
                    .map(str::to_owned)
                    .or_else(|| (!p.id().is_empty()).then(|| p.id().to_owned()))
                    .unwrap_or_else(|| format!("capa{}", capas.len() + 1));
                let formas = caminos(p.data(), centro);
                let mut trazos = Vec::new();
                if let Some(f) = p.fill() {
                    match color_de(f.paint()) {
                        Some(c) => trazos.extend(formas.iter().map(|forma| Trazo { forma: forma.clone(), color: c, alfa: f.opacity().get(), grosor: 0.0 })),
                        None => aviso.push(format!("'{nombre}' is filled with a gradient or a pattern, which does not come across; give it a `color:` where you draw it")),
                    }
                }
                if let Some(t) = p.stroke() {
                    if let Some(c) = color_de(t.paint()) {
                        trazos.extend(formas.iter().map(|forma| Trazo { forma: forma.clone(), color: c, alfa: t.opacity().get(), grosor: t.width().get() }));
                    }
                }
                if trazos.is_empty() {
                    continue;
                }
                match capas.iter_mut().find(|c| c.nombre == nombre) {
                    Some(c) => c.trazos.extend(trazos),
                    None => capas.push(Capa { nombre, trazos }),
                }
            }
            // Una imagen o un texto dentro del SVG no son geometría: se dicen.
            resvg::usvg::Node::Image(_) => aviso.push("there is an image inside that svg, and an image is not geometry: it is left out".into()),
            resvg::usvg::Node::Text(_) => aviso.push("there is text inside that svg: convert it to paths in the editor, or it is left out".into()),
        }
    }
}

/// Lee un SVG y devuelve sus capas, más lo que haya que decir en voz alta.
pub fn leer(datos: &[u8]) -> Result<(Figura, Vec<String>), String> {
    let arbol = resvg::usvg::Tree::from_data(datos, &resvg::usvg::Options::default()).map_err(|e| e.to_string())?;
    let tam = (arbol.size().width(), arbol.size().height());
    let mut capas = Vec::new();
    let mut aviso = Vec::new();
    recorrer(arbol.root(), None, (tam.0 * 0.5, tam.1 * 0.5), &mut capas, &mut aviso);
    if capas.is_empty() {
        return Err("that svg has no paths in it".into());
    }
    // Lo que cuesta pintarla se sabe ahora, no a medio dibujar.
    for c in &capas {
        for t in &c.trazos {
            if let Forma::Camino { origen, pasos, .. } = &t.forma {
                let n = cuantos_puntos(origen, pasos);
                if n > crate::formas::MAX_PUNTOS {
                    return Err(format!(
                        "the layer '{}' needs {} points and the most a path takes is {}: simplify it in the editor (Path › Simplify), or split it into layers",
                        c.nombre,
                        n,
                        crate::formas::MAX_PUNTOS
                    ));
                }
            }
        }
    }
    Ok((Figura { capas, tam }, aviso))
}
