//! Texto e imágenes: todo lo que acaba siendo un trozo del atlas.
//!
//! El texto se da forma con `cosmic-text` —ligaduras, idiomas de derecha a
//! izquierda, emoji, fuentes de reserva— y cada glifo se pinta una vez, a la
//! escala de la lámina más fina, en un atlas que comparte con las imágenes. Las
//! tres piezas (`cosmic-text`, `image`, `resvg`) son Rust puro y existen en
//! Linux, Windows y macOS; lo único que depende del sistema es encontrar un
//! icono por su nombre, y eso lo contesta la plataforma.

use crate::escena::{ARender, Alineado, Estilo, Fuente};
use std::collections::HashSet;
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use cosmic_text::{Align, Attrs, Buffer, CacheKey, Ellipsize, EllipsizeHeightLimit, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Weight, Wrap};
use std::collections::HashMap;

pub const LADO_DEL_ATLAS: u32 = 2048;

/// Un trozo del atlas, en píxeles.
#[derive(Clone, Copy, Debug)]
pub struct Hueco {
    pub x: u32,
    pub y: u32,
    pub ancho: u32,
    pub alto: u32,
}

impl Hueco {
    pub fn uv(&self) -> [f32; 4] {
        let l = LADO_DEL_ATLAS as f32;
        [self.x as f32 / l, self.y as f32 / l, (self.x + self.ancho) as f32 / l, (self.y + self.alto) as f32 / l]
    }
}

/// Un glifo ya colocado, relativo a la esquina de su texto y en píxeles lógicos.
#[derive(Clone, Copy)]
pub struct GlifoPuesto {
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    /// Un emoji trae su color; una letra es una máscara que se tiñe.
    pub en_color: bool,
}

pub struct Maqueta {
    pub glifos: Vec<GlifoPuesto>,
    pub tam: (f32, f32),
}

/// Todo lo que decide cómo queda un texto. Dos textos con la misma clave son
/// la misma maqueta, y con ella basta para hacerla: es lo que viaja al taller.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct Clave {
    texto: String,
    familia: Option<&'static str>,
    px: u32,
    peso: u16,
    interlinea: u32,
    ancho: Option<u32>,
    alineado: Alineado,
    max_lineas: Option<usize>,
}

impl Clave {
    pub fn de(texto: &str, e: &Estilo, ancho: Option<f32>) -> Clave {
        Clave {
            texto: texto.to_owned(), familia: e.familia, px: e.px.to_bits(), peso: e.peso, interlinea: e.interlinea.to_bits(),
            ancho: ancho.map(|a| a.round().max(1.0) as u32), alineado: e.alineado, max_lineas: e.max_lineas,
        }
    }
}

/// Reparte el atlas por estantes: filas de la altura de lo primero que cayó en ellas.
struct Estantes {
    filas: Vec<(u32, u32, u32)>, // y, alto, hasta dónde está lleno
    siguiente_y: u32,
}

impl Estantes {
    fn nuevo() -> Self {
        Estantes { filas: Vec::new(), siguiente_y: 1 }
    }

    fn pedir(&mut self, ancho: u32, alto: u32) -> Option<Hueco> {
        let (w, h) = (ancho + 1, alto + 1); // un píxel de aire para que el filtrado no sangre
        let fila = self.filas.iter_mut().find(|(_, fh, fx)| *fh >= h && *fh <= h + h / 3 + 2 && fx + w <= LADO_DEL_ATLAS);
        let (y, x) = match fila {
            Some((y, _, fx)) => {
                let x = *fx;
                *fx += w;
                (*y, x)
            }
            None => {
                if self.siguiente_y + h > LADO_DEL_ATLAS || w > LADO_DEL_ATLAS {
                    return None;
                }
                let y = self.siguiente_y;
                self.siguiente_y += h;
                self.filas.push((y, h, 1 + w));
                (y, 1)
            }
        };
        Some(Hueco { x, y, ancho, alto })
    }
}

/// Quien da forma y pinta. Vive en su propio hilo —el taller—: leer las fuentes
/// del sistema, dar forma a un párrafo o decodificar un PNG pueden tardar, y
/// nada de eso puede costarle un frame al render.
struct Tipografo {
    fuentes: FontSystem,
    swash: SwashCache,
    estantes: Estantes,
    glifos: HashMap<CacheKey, Option<(Hueco, i32, i32, bool)>>,
    /// Lo pintado desde la última entrega: dónde y qué (RGBA premultiplicado).
    por_subir: Vec<(Hueco, Vec<u8>)>,
    /// A cuántos píxeles de verdad por píxel lógico se pinta: la de la lámina más fina.
    escala: f32,
}

impl Tipografo {
    fn nuevo() -> Self {
        let t0 = std::time::Instant::now();
        let fuentes = FontSystem::new();
        println!("texto  · {} fuentes del sistema en {} ms", fuentes.db().len(), t0.elapsed().as_millis());
        Tipografo { fuentes, swash: SwashCache::new(), estantes: Estantes::nuevo(), glifos: HashMap::new(), por_subir: Vec::new(), escala: 1.0 }
    }

    /// Al cambiar la escala, todo lo pintado deja de valer.
    fn vaciar(&mut self, escala: f32) {
        self.escala = escala;
        self.estantes = Estantes::nuevo();
        self.glifos.clear();
        self.por_subir.clear();
    }

    fn maquetar(&mut self, c: &Clave) -> Maqueta {
        let (px, interlinea, ancho) = (f32::from_bits(c.px), f32::from_bits(c.interlinea), c.ancho.map(|a| a as f32));
        let mut buffer = Buffer::new(&mut self.fuentes, Metrics::new(px, px * interlinea));
        buffer.set_wrap(if ancho.is_some() { Wrap::WordOrGlyph } else { Wrap::None });
        if let Some(n) = c.max_lineas {
            buffer.set_ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(n)));
        }
        buffer.set_size(ancho, None);
        let attrs = Attrs::new().family(c.familia.map_or(Family::SansSerif, Family::Name)).weight(Weight(c.peso));
        let alineado = match c.alineado {
            Alineado::Izquierda => Align::Left,
            Alineado::Centro => Align::Center,
            Alineado::Derecha => Align::Right,
        };
        buffer.set_text(&c.texto, &attrs, Shaping::Advanced, Some(alineado));
        buffer.shape_until_scroll(&mut self.fuentes, false);

        let s = self.escala;
        let mut glifos = Vec::new();
        let (mut w, mut h) = (0f32, 0f32);
        let mut lleno = false;
        for run in buffer.layout_runs() {
            w = w.max(run.line_w);
            h = h.max(run.line_top + run.line_height);
            for g in run.glyphs {
                let f = g.physical((0.0, 0.0), s);
                let Some((hueco, izquierda, arriba, en_color)) = self.glifo(f.cache_key, &mut lleno) else { continue };
                let x = (f.x + izquierda) as f32;
                let y = ((run.line_y * s).round() as i32 + f.y - arriba) as f32;
                glifos.push(GlifoPuesto { rect: [x / s, y / s, hueco.ancho as f32 / s, hueco.alto as f32 / s], uv: hueco.uv(), en_color });
            }
        }
        if lleno {
            eprintln!("texto  · el atlas de {LADO_DEL_ATLAS}² está lleno: algunos glifos no se pintan");
        }
        // Sin ancho fijo, el alineado es respecto a lo que mida el propio texto.
        Maqueta { glifos, tam: (ancho.unwrap_or(w), h) }
    }

    fn glifo(&mut self, clave: CacheKey, lleno: &mut bool) -> Option<(Hueco, i32, i32, bool)> {
        if let Some(g) = self.glifos.get(&clave) {
            return *g;
        }
        let pintado = self.swash.get_image_uncached(&mut self.fuentes, clave).and_then(|img| {
            let (w, h) = (img.placement.width, img.placement.height);
            if w == 0 || h == 0 {
                return None;
            }
            let rgba: Vec<u8> = match img.content {
                // Una máscara: blanco premultiplicado, que el shader teñirá.
                SwashContent::Mask => img.data.iter().flat_map(|a| [*a, *a, *a, *a]).collect(),
                SwashContent::Color => img.data.chunks_exact(4).flat_map(|p| {
                    let a = p[3] as u16;
                    [(p[0] as u16 * a / 255) as u8, (p[1] as u16 * a / 255) as u8, (p[2] as u16 * a / 255) as u8, p[3]]
                }).collect(),
                SwashContent::SubpixelMask => return None,
            };
            let Some(hueco) = self.estantes.pedir(w, h) else {
                *lleno = true;
                return None;
            };
            self.por_subir.push((hueco, rgba));
            Some((hueco, img.placement.left, img.placement.top, matches!(img.content, SwashContent::Color)))
        });
        self.glifos.insert(clave, pintado);
        pintado
    }

    /// Las imágenes de una escena, al tamaño lógico que pidan por la escala. Un
    /// SVG se pinta a ese tamaño exacto; un PNG se reduce si sobra.
    fn cargar_imagenes(&mut self, imagenes: &[(Fuente, (u32, u32))]) -> Vec<Option<Hueco>> {
        imagenes
            .iter()
            .map(|(fuente, (w, h))| {
                let px = (((*w as f32) * self.escala).round().max(1.0) as u32, ((*h as f32) * self.escala).round().max(1.0) as u32);
                let ruta = match fuente {
                    Fuente::Ruta(r) => Some(r.clone()),
                    Fuente::Icono(nombre) => crate::plataforma::icono(nombre),
                };
                let hueco = ruta.as_ref().and_then(|r| pintar_imagen(r, px)).and_then(|rgba| {
                    let hueco = self.estantes.pedir(px.0, px.1)?;
                    self.por_subir.push((hueco, rgba));
                    Some(hueco)
                });
                if hueco.is_none() {
                    eprintln!("imagen · no puedo cargar {fuente:?}");
                }
                hueco
            })
            .collect()
    }
}

// ── el taller y su mostrador ────────────────────────────────────

enum Encargo {
    Maqueta(Clave, u32),
    Imagenes(Vec<(Fuente, (u32, u32))>, u32),
    /// Atlas nuevo, a esta escala. Todo lo de generaciones anteriores se tira.
    Vaciar(f32),
}

/// Lo que el taller devuelve, con lo que haya pintado por el camino.
pub enum Entrega {
    Maqueta { clave: Clave, maqueta: Arc<Maqueta>, generacion: u32 },
    Imagenes { huecos: Vec<Option<Hueco>>, generacion: u32 },
}

pub struct Paquete {
    pub entrega: Entrega,
    pub por_subir: Vec<(Hueco, Vec<u8>)>,
}

/// El lado del render: pide, no espera, y mientras tanto enseña lo que tenía.
pub struct Textos {
    al_taller: Sender<Encargo>,
    maquetas: HashMap<Clave, Arc<Maqueta>>,
    pedidas: HashSet<Clave>,
    /// La última maqueta que se enseñó en cada sitio de la escena: es lo que se
    /// ve mientras llega la nueva, en vez de un hueco.
    ultima: HashMap<usize, Arc<Maqueta>>,
    imagenes: Vec<Option<Hueco>>,
    pub por_subir: Vec<(Hueco, Vec<u8>)>,
    generacion: u32,
    escala: f32,
}

impl Textos {
    /// Arranca el taller. Lo primero que hace es leer las fuentes del sistema,
    /// así que conviene llamarlo cuanto antes.
    pub fn abrir(al_render: Sender<ARender>) -> Textos {
        let (al_taller, encargos) = channel::<Encargo>();
        std::thread::Builder::new()
            .name("taller".into())
            .spawn(move || {
                let mut t = Tipografo::nuevo();
                for encargo in encargos {
                    let entrega = match encargo {
                        Encargo::Vaciar(escala) => {
                            t.vaciar(escala);
                            continue;
                        }
                        Encargo::Maqueta(clave, generacion) => {
                            let maqueta = Arc::new(t.maquetar(&clave));
                            Entrega::Maqueta { clave, maqueta, generacion }
                        }
                        Encargo::Imagenes(lista, generacion) => Entrega::Imagenes { huecos: t.cargar_imagenes(&lista), generacion },
                    };
                    let por_subir = std::mem::take(&mut t.por_subir);
                    // Por el canal de siempre, que además despierta al render si dormía.
                    if al_render.send(ARender::Taller(Box::new(Paquete { entrega, por_subir }))).is_err() {
                        return;
                    }
                }
            })
            .unwrap();
        Textos { al_taller, maquetas: HashMap::new(), pedidas: HashSet::new(), ultima: HashMap::new(), imagenes: Vec::new(), por_subir: Vec::new(), generacion: 0, escala: 1.0 }
    }

    pub fn escala(&self) -> f32 {
        self.escala
    }

    /// Escena nueva o escala nueva: atlas nuevo. Lo anterior deja de valer, y
    /// con ello lo que se estuviera enseñando.
    pub fn empezar_de_cero(&mut self, escala: f32, imagenes: &[(Fuente, (u32, u32))]) {
        self.generacion += 1;
        self.escala = escala;
        self.maquetas.clear();
        self.pedidas.clear();
        self.ultima.clear();
        self.imagenes.clear();
        self.por_subir.clear();
        let _ = self.al_taller.send(Encargo::Vaciar(escala));
        if !imagenes.is_empty() {
            let _ = self.al_taller.send(Encargo::Imagenes(imagenes.to_vec(), self.generacion));
        }
    }

    pub fn recibir(&mut self, p: Paquete) {
        let generacion = match &p.entrega {
            Entrega::Maqueta { generacion, .. } | Entrega::Imagenes { generacion, .. } => *generacion,
        };
        if generacion != self.generacion {
            return; // de un atlas que ya no existe
        }
        self.por_subir.extend(p.por_subir);
        match p.entrega {
            Entrega::Maqueta { clave, maqueta, .. } => {
                self.pedidas.remove(&clave);
                if self.maquetas.len() > 512 {
                    self.maquetas.clear();
                }
                self.maquetas.insert(clave, maqueta);
            }
            Entrega::Imagenes { huecos, .. } => self.imagenes = huecos,
        }
    }

    /// La maqueta de un texto, si ya está; si no, se encarga y se devuelve la
    /// última que se vio en ese sitio. `sitio` es la posición en la lista de dibujo.
    pub fn maqueta(&mut self, sitio: usize, clave: Clave) -> Option<Arc<Maqueta>> {
        if let Some(m) = self.maquetas.get(&clave) {
            self.ultima.insert(sitio, m.clone());
            return Some(m.clone());
        }
        if self.pedidas.insert(clave.clone()) {
            let _ = self.al_taller.send(Encargo::Maqueta(clave, self.generacion));
        }
        self.ultima.get(&sitio).cloned()
    }

    pub fn imagen(&self, k: usize) -> Option<Hueco> {
        self.imagenes.get(k).copied().flatten()
    }
}

/// RGBA premultiplicado, a `px` exactos, encajada sin deformar.
fn pintar_imagen(ruta: &std::path::Path, px: (u32, u32)) -> Option<Vec<u8>> {
    let datos = std::fs::read(ruta).ok()?;
    let es_svg = ruta.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"));
    if es_svg {
        let arbol = resvg::usvg::Tree::from_data(&datos, &resvg::usvg::Options::default()).ok()?;
        let mut lienzo = resvg::tiny_skia::Pixmap::new(px.0, px.1)?;
        let t = arbol.size();
        let k = (px.0 as f32 / t.width()).min(px.1 as f32 / t.height());
        let (dx, dy) = ((px.0 as f32 - t.width() * k) * 0.5, (px.1 as f32 - t.height() * k) * 0.5);
        resvg::render(&arbol, resvg::tiny_skia::Transform::from_scale(k, k).post_translate(dx, dy), &mut lienzo.as_mut());
        return Some(lienzo.take()); // tiny-skia ya premultiplica
    }
    let img = image::load_from_memory(&datos).ok()?.to_rgba8();
    let k = (px.0 as f32 / img.width() as f32).min(px.1 as f32 / img.height() as f32);
    let (w, h) = (((img.width() as f32 * k).round() as u32).clamp(1, px.0), ((img.height() as f32 * k).round() as u32).clamp(1, px.1));
    let reducida = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle);
    let mut img = image::RgbaImage::new(px.0, px.1);
    image::imageops::overlay(&mut img, &reducida, ((px.0 - w) / 2) as i64, ((px.1 - h) / 2) as i64);
    Some(img.pixels().flat_map(|p| {
        let a = p[3] as u16;
        [(p[0] as u16 * a / 255) as u8, (p[1] as u16 * a / 255) as u8, (p[2] as u16 * a / 255) as u8, p[3]]
    }).collect())
}
