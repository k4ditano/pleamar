//! La parte que habla con la GPU: el dispositivo, las láminas —una por
//! superficie de Wayland— y la composición de la lista de dibujo en elementos.

use crate::escena::*;
use crate::plataforma::Ventana;
use crate::texto::{Clave, Hueco, Textos, LADO_DEL_ATLAS};
use std::ops::Range;

const POR_FORMA: usize = 20;
const POR_ELEMENTO: usize = 52;
/// Cuántos grupos con opacidad pueden estar fundiéndose en el mismo frame.
pub const MAX_CAPAS: usize = 4;
/// Cuántos frames pintados sin grupos con opacidad hasta devolver sus capas.
const CAPAS_OCIOSAS: u32 = 300;
/// La franja que se le añade a la superficie para la gráfica de frames.
pub const ALTO_INSTRUMENTOS: f32 = 84.0;
/// Con cuánto sitio se empieza. Si una escena pide más, los almacenes crecen al doble.
const FORMAS_DE_SALIDA: usize = 1024;
/// Floats (pares x, y) para los caminos. Crece como los demás.
const PUNTOS_DE_SALIDA: usize = 1024;
/// Floats (r, g, b, dónde) para las paradas de los degradados.
const PARADAS_DE_SALIDA: usize = 512;
const ELEMENTOS_DE_SALIDA: usize = 1024;

/// La lista de dibujo convertida en lo que pinta la GPU: formas evaluadas y
/// elementos con su caja. Se recompone cada frame; son unos pocos cientos de
/// números.
#[derive(Default)]
pub struct Dibujo {
    pub formas: Vec<f32>,
    /// Los puntos de los caminos, en pares x, y: cada camino se lleva un tramo.
    pub puntos: Vec<f32>,
    /// Las paradas de los degradados: r, g, b y dónde cae cada una.
    pub paradas: Vec<f32>,
    pub elementos: Vec<f32>,
    tam: (f32, f32),
    /// Lo que han medido los textos que lo pidieron: propiedad y valor.
    pub medidas: Vec<(PropId, f32)>,
    /// Los campos de texto, como quedaron: para saber dónde cae un clic.
    pub campos: Vec<CampoPuesto>,
    /// Grupos que se pintan aparte: qué elementos, y en qué capa.
    pub apartes: Vec<(Range<u32>, usize)>,
    /// Los trozos de la escena que alguna emergente abierta está enseñando.
    pub vistas: Vec<[f32; 4]>,
    avisado_de_recortes: bool,
    /// Una sombra cortada y una forma cortada, esperando a decirse.
    sombra: Pendiente,
    corte: Pendiente,
    /// Lo que mide la superficie de la escena, sin la franja de instrumentos.
    suya: (f32, f32),
    /// Tramos de instrucciones que no se miran este frame: los de una copia de
    /// pantalla cuya superficie está cerrada.
    pub saltar: Vec<std::ops::Range<usize>>,
    /// Y a qué bordes está pegada: contra esos no se avisa de nada.
    pegada: [bool; 4],
    /// Dónde hay cristal este frame, en franjas del plano de la escena: lo que
    /// se le pide al compositor que desenfoque.
    pub cristales: Vec<[f32; 4]>,
    /// Las franjas de cada forma de cristal, en el origen, por lo que la hace
    /// ser como es menos dónde está: mientras solo se mueva, no se vuelven a buscar.
    cache_de_cristales: std::collections::HashMap<[u32; 12], (Vec<[f32; 4]>, bool)>,
}

/// La lista de dibujo del frame anterior, para saber **dónde** ha cambiado algo.
/// Cada superficie paga por presentar un frame (~0,5 ms, ver P11) aunque no
/// cambie nada suyo: con una copia por monitor, la bolita respirando en uno
/// repintaba también el otro. Comparar bit a bit es mucho más barato que eso.
#[derive(Default)]
pub struct Anterior {
    formas: Vec<f32>,
    puntos: Vec<f32>,
    paradas: Vec<f32>,
    elementos: Vec<f32>,
    apartes: Vec<(Range<u32>, usize)>,
    hay: bool,
}

impl Anterior {
    /// Las cajas —en el plano de la escena— de lo que ha cambiado desde el
    /// frame anterior: la de antes y la de ahora, porque lo que se va de una
    /// superficie también la cambia. `false` si no se puede saber elemento a
    /// elemento —algo ha aparecido o desaparecido y los índices ya no casan—:
    /// entonces ha cambiado todo.
    pub fn cambios(&mut self, d: &Dibujo, cajas: &mut Vec<[f32; 4]>) -> bool {
        cajas.clear();
        fn bits(v: &[f32]) -> &[u32] {
            bytemuck::cast_slice(v)
        }
        let misma_forma = self.hay
            && self.formas.len() == d.formas.len()
            && self.puntos.len() == d.puntos.len()
            && self.paradas.len() == d.paradas.len()
            && self.elementos.len() == d.elementos.len()
            && self.apartes == d.apartes;
        let sabido = misma_forma && {
            let puntos_iguales = bits(&self.puntos) == bits(&d.puntos);
            let paradas_iguales = bits(&self.paradas) == bits(&d.paradas);
            // Una forma cambia si cambian sus números, o si es un camino y han cambiado los puntos.
            let forma_cambiada: Vec<bool> = self
                .formas
                .chunks_exact(POR_FORMA)
                .zip(d.formas.chunks_exact(POR_FORMA))
                .map(|(a, b)| bits(a) != bits(b) || (!puntos_iguales && b[0] as u32 == 4))
                .collect();
            let cambiada = |k: f32| k >= 0.0 && forma_cambiada.get(k as usize).copied().unwrap_or(true);
            for (a, b) in self.elementos.chunks_exact(POR_ELEMENTO).zip(d.elementos.chunks_exact(POR_ELEMENTO)) {
                let mut cambio = bits(a) != bits(b);
                // Un cuerpo es sus formas; un recorte también es una forma.
                if !cambio && b[0] == 0.0 {
                    cambio = (b[1] as usize..b[1] as usize + b[2] as usize).any(|k| cambiada(k as f32)) || (!paradas_iguales && b[15] > 0.5);
                }
                if !cambio {
                    cambio = b[32..36].iter().any(|&r| cambiada(r));
                }
                if cambio {
                    cajas.push([a[4], a[5], a[6], a[7]]);
                    cajas.push([b[4], b[5], b[6], b[7]]);
                }
            }
            true
        };
        self.formas.clone_from(&d.formas);
        self.puntos.clone_from(&d.puntos);
        self.paradas.clone_from(&d.paradas);
        self.elementos.clone_from(&d.elementos);
        self.apartes.clone_from(&d.apartes);
        self.hay = true;
        sabido
    }

    /// Que el frame que viene no se compare con este: lo que había en el atlas ya no vale.
    pub fn olvidar(&mut self) {
        self.hay = false;
    }
}

/// El campo donde se está escribiendo, visto desde quien pinta.
#[derive(Clone, Copy)]
pub struct VistaDeCampo {
    pub texto: usize,
    pub cursor: usize,
    pub ancla: usize,
    /// El cursor parpadea.
    pub se_ve: bool,
}

pub struct CampoPuesto {
    pub texto: usize,
    pub zona: &'static str,
    pub maqueta: Option<std::sync::Arc<crate::texto::Maqueta>>,
    /// Dónde empieza el texto, y cuánto se ha corrido para que el cursor se vea.
    pub x0: f32,
    pub corrido: f32,
    /// Si lo pintado son puntos: entonces un byte de la maqueta no es un byte
    /// del texto, y hay que pasar de uno a otro por el número de letra.
    pub secreto: bool,
}

/// El punto con el que se pinta cada letra de un campo secreto. Mide tres
/// bytes, sea cual sea la letra que tapa: de ahí salen las dos cuentas.
pub const PUNTO: &str = "•";
/// El byte del texto de verdad que corresponde a uno de la maqueta de puntos.
pub fn byte_de_verdad(valor: &str, en_puntos: usize) -> usize {
    valor.char_indices().nth(en_puntos / PUNTO.len()).map_or(valor.len(), |(i, _)| i)
}

/// Un grupo con opacidad, mientras se va llenando.
enum GrupoOpaco {
    /// No hace falta capa: opaco del todo, o no quedan capas. Se multiplica y ya.
    Multiplica(f32),
    /// Invisible: nada de dentro se emite.
    Oculto,
    Capa { alfa: f32, indice: usize, primer_elemento: usize },
}

struct CuerpoAbierto {
    primera: usize,
    n: usize,
    caja: Option<[f32; 4]>,
    holgura: f32,
    sombra: Option<Sombra>,
    /// Sus formas tal cual, para saber en la CPU por dónde pasa su borde: lo
    /// necesita el cristal, que pide al compositor que desenfoque justo ahí.
    planas: Vec<crate::formas::Plana>,
}

/// Por debajo de esto, un cristal casi no se ve y no se pide desenfoque.
const CRISTAL_VISIBLE: f32 = 0.3;
/// El alto de cada franja de la región de desenfoque, en píxeles lógicos.
const FRANJA: f32 = 2.0;

/// La silueta de una forma como rectángulos, **centrada en el origen**: franjas
/// de 2 px, y en cada una los tramos que caen dentro. La región que se pide al
/// compositor se hace de rectángulos; con la caja entera, alrededor de una
/// bolita redonda se veían las esquinas de un cuadrado desenfocado. Las filas
/// iguales seguidas —los lados rectos de una tarjeta— se juntan en una.
///
/// En el origen porque lo que casi siempre cambia de una forma es dónde está:
/// así, una bolita que viaja no se vuelve a medir, solo se corre.
fn franjas_de(p: &crate::formas::Plana, puntos: &[f32]) -> Vec<[f32; 4]> {
    let mut franjas: Vec<[f32; 4]> = Vec::new();
    let Some(caja) = p.caja() else { return franjas };
    // Una caja redondeada o una elipse, rellenas, sin girar ni torcer: el ancho
    // de cada fila tiene fórmula, y no hace falta buscarlo. Es lo que hace que
    // una tarjeta que crece al abrirse no cueste un milisegundo por frame.
    let m = p.afin.m;
    if (p.tipo == 0 || p.tipo == 1) && p.giro == 0.0 && p.trazo == 0.0 && m[1] == 0.0 && m[2] == 0.0 && m[0] > 0.0 && m[3] > 0.0 {
        let (sx, sy) = (m[0], m[3]);
        // Media altura y, para cada altura desde el centro, medio ancho: en local.
        let (alto, ancho): (f32, Box<dyn Fn(f32) -> f32>) = if p.tipo == 1 {
            let r = p.radio.clamp(0.0, p.mx.min(p.my));
            let (mx, my) = (p.mx, p.my);
            (my, Box::new(move |y: f32| {
                let dy = y.abs() - (my - r);
                if dy <= 0.0 { mx } else { mx - r + (r * r - dy * dy).max(0.0).sqrt() }
            }))
        } else {
            let (a, b) = (p.radio * p.ex, p.radio * p.ey);
            (b, Box::new(move |y: f32| a * (1.0 - (y / b).powi(2)).max(0.0).sqrt()))
        };
        let (y0, y1) = (-alto * sy, alto * sy);
        let mut y = (y0 / FRANJA).floor() * FRANJA;
        while y < y1 {
            // El ancho a media franja, como cuando se busca.
            let medio = ancho(((y + FRANJA * 0.5) / sy).clamp(-alto, alto)) * sx;
            if medio > 0.25 {
                let (a, b) = ((-medio).floor(), medio.ceil());
                match franjas.last_mut() {
                    Some(f) if f[0] == a && f[2] == b && f[3] == y => f[3] = y + FRANJA,
                    _ => franjas.push([a, y, b, y + FRANJA]),
                }
            }
            y += FRANJA;
        }
        return franjas;
    }
    let dentro = |x: f32, y: f32| p.distancia_con(x, y, puntos) < 0.0;
    // Dónde cambia de fuera a dentro entre a y b: por bisección, a un cuarto de píxel.
    let borde = |mut a: f32, mut b: f32, y: f32, entra: bool| {
        while b - a > 0.25 {
            let m = (a + b) * 0.5;
            if dentro(m, y) == entra { b = m } else { a = m }
        }
        if entra { a } else { b }
    };
    let paso = 3.0;
    let mut y = (caja[1] / FRANJA).floor() * FRANJA;
    let mut fila: Vec<(f32, f32)> = Vec::new();
    while y < caja[3] {
        let yc = y + FRANJA * 0.5;
        fila.clear();
        let mut x = caja[0];
        let mut desde: Option<f32> = dentro(x, yc).then_some(x);
        while x < caja[2] {
            let sig = (x + paso).min(caja[2]);
            let esta = dentro(sig, yc);
            match (desde, esta) {
                (None, true) => desde = Some(borde(x, sig, yc, true)),
                (Some(a), false) => {
                    fila.push((a, borde(x, sig, yc, false)));
                    desde = None;
                }
                _ => {}
            }
            x = sig;
        }
        if let Some(a) = desde {
            fila.push((a, caja[2]));
        }
        for &(a, b) in &fila {
            let (a, b) = (a.floor(), b.ceil());
            // La fila de arriba con el mismo tramo, pegada a esta: se alarga.
            match franjas.iter_mut().rev().take(fila.len() + 2).find(|f| f[0] == a && f[2] == b && f[3] == y) {
                Some(f) => f[3] = y + FRANJA,
                None => franjas.push([a, y, b, y + FRANJA]),
            }
        }
        y += FRANJA;
    }
    franjas
}

fn unir(a: Option<[f32; 4]>, b: [f32; 4]) -> [f32; 4] {
    a.map_or(b, |a| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
}

/// Algo que se ve cortado y aún no se ha dicho: lo peor visto y cómo contarlo.
/// No se dice en cuanto se ve —una tarjeta que se abre se sale más a cada
/// frame, y el primer píxel no es el que hay que arreglar—, sino cuando la
/// cuenta deja de crecer o la escena se queda quieta. Y si deja de estar
/// cortado, es que pasaba por ahí: entonces no se dice nunca.
#[derive(Default)]
struct Pendiente {
    /// Lo peor de este episodio, que es lo que dice si la cosa sigue creciendo,
    /// y lo peor de ESTE frame, que es quien pone el texto cuando hay varias
    /// cosas cortadas a la vez.
    peor: f32,
    peor_ahora: f32,
    dicho: Option<String>,
    sin_crecer: u8,
    /// Cuántos frames seguidos lleva cortado. Una escena viva —una que respira—
    /// no se queda quieta nunca, así que esperar al reposo sería no decirlo
    /// jamás: a los tres segundos cortado, ya no pasaba por ahí.
    visto: u16,
    este_frame: bool,
    dicha: bool,
}

impl Pendiente {
    fn apunta(&mut self, peor: f32, dicho: impl FnOnce() -> String) {
        self.este_frame = true;
        if peor > self.peor {
            self.peor = peor;
            self.sin_crecer = 0;
        }
        // El texto es el de AHORA, no el del pico: una tarjeta que se abre pasa
        // por encima del borde de arriba y acaba sobrando por abajo, y lo que
        // hay que arreglar es lo segundo. Se rehace cuatro veces por segundo, y
        // lo dice la peor de las que estén cortadas en ese frame.
        if peor >= self.peor_ahora {
            self.peor_ahora = peor;
            if self.dicho.is_none() || self.visto % 15 == 14 {
                self.dicho = Some(dicho());
            }
        }
    }
    /// Al cerrar el frame: lo que ya no está cortado se olvida. Y lo que lleva
    /// un cuarto de segundo sin crecer se cuenta ya, si es de los que pueden
    /// adelantarse. **Una forma no lo es**: cruzar un borde de camino es lo
    /// normal —una tarjeta que se abre sube por encima del borde y vuelve—, así
    /// que de esas solo se habla cuando la escena se queda quieta y lo cortado
    /// es lo que se queda mirando.
    fn cierra_el_frame(&mut self, puede_adelantarse: bool) -> Option<String> {
        self.peor_ahora = 0.0;
        if !std::mem::take(&mut self.este_frame) {
            self.dicho = None;
            self.peor = 0.0;
            self.sin_crecer = 0;
            self.visto = 0;
            return None;
        }
        self.visto = self.visto.saturating_add(1);
        self.sin_crecer = self.sin_crecer.saturating_add(1);
        if (puede_adelantarse && self.sin_crecer >= 15) || self.visto >= 180 {
            return self.ya();
        }
        None
    }
    fn ya(&mut self) -> Option<String> {
        let dicho = self.dicho.take()?;
        self.dicha = true;
        Some(dicho)
    }
}

impl Dibujo {
    pub fn n_elementos(&self) -> usize {
        self.elementos.len() / POR_ELEMENTO
    }

    /// Si algún elemento de ese tramo cae en ese trozo del plano.
    fn toca_la_vista(&self, tramo: &Range<u32>, v: [f32; 4]) -> bool {
        tramo.clone().any(|k| {
            let b = &self.elementos[k as usize * POR_ELEMENTO + 4..k as usize * POR_ELEMENTO + 8];
            b[0] < v[2] && b[2] > v[0] && b[1] < v[3] && b[3] > v[1]
        })
    }

    fn forma(&mut self, p: crate::formas::Plana, fusion: f32) -> usize {
        let k = self.formas.len() / POR_FORMA;
        self.formas.resize(self.formas.len() + POR_FORMA, 0.0);
        p.codificar(fusion, &mut self.formas[k * POR_FORMA..]);
        k
    }

    /// Un trozo del atlas —un glifo, una imagen— colocado en pantalla. Con
    /// `tinte`, su alfa es una máscara que se pinta de ese color.
    fn trozo(&mut self, d: [f32; 4], uv: [f32; 4], alfa: f32, tinte: Option<[f32; 3]>, afin: Afin, recortes: &[(usize, [f32; 4])]) {
        let caja = afin.caja([d[0], d[1], d[0] + d[2], d[1] + d[3]]);
        self.elemento(1.0, [caja[0] - 1.0, caja[1] - 1.0, caja[2] + 1.0, caja[3] + 1.0], recortes, |e| {
            afin.codificar(&mut e[44..52]);
            e[3] = alfa;
            if let Some(rgb) = tinte {
                e[2] = 1.0;
                e[8..11].copy_from_slice(&rgb);
            }
            e[36..40].copy_from_slice(&d);
            e[40..44].copy_from_slice(&uv);
        });
    }

    /// Una sombra no se corta a propósito nunca. Si la forma entra entera en la
    /// superficie y su sombra no, el borde queda recto y quien lo ve no tiene
    /// dónde mirar: la sombra no se declara con un tamaño, sale de dos números.
    /// La cuenta ya está hecha ahí arriba; decirla cuesta cuatro restas.
    fn mirar_la_sombra(&mut self, forma: [f32; 4], (dx, dy, d): (f32, f32, f32)) {
        if self.sombra.dicha {
            return;
        }
        let (w, alto) = self.suya;
        // Lo que pide la sombra son sus propios números: desplazamiento y
        // difusión. Los márgenes que el render se guarda no cuentan, o el aviso
        // diría dos píxeles que nadie escribió.
        let falta = [d - dx - forma[0], d - dy - forma[1], forma[2] + dx + d - w, forma[3] + dy + d - alto];
        // Solo por los lados donde la forma flota dentro. Una barra pegada al
        // borde de arriba tiene la sombra cortada por arriba, claro: ahí no
        // había sitio ni lo quería. Lo que no se explica solo es una tarjeta que
        // cabe entera y cuya sombra, aun así, choca contra el borde.
        let flota = [forma[0] > 0.5, forma[1] > 0.5, forma[2] < w - 0.5, forma[3] < alto - 0.5];
        let lados = ["on the left", "above", "on the right", "below"];
        let dichos: Vec<String> = (0..4)
            .filter(|&k| flota[k] && falta[k] > 0.5)
            .map(|k| format!("{:.0} px {}", falta[k].ceil(), lados[k]))
            .collect();
        let peor = falta.iter().zip(flota).filter(|(_, f)| *f).map(|(v, _)| *v).fold(0.0f32, f32::max);
        if dichos.is_empty() {
            return;
        }
        self.sombra.apunta(peor, || {
            format!(
                "render · a shadow is cut: it needs {} more than this {:.0} x {:.0} surface has. The shape fits; its shadow does not",
                dichos.join(" and "),
                w,
                alto
            )
        });
    }

    /// Y lo mismo de la forma, que es lo que de verdad se ve cortado cuando un
    /// panel crece más de lo que su superficie tiene. Salirse por un borde a
    /// propósito es legítimo —el cuello de una bolita cuelga del borde de
    /// arriba, y ahí no sobra sitio ni se quiere—, así que solo se dice de lo
    /// que **casi entero** cabía: si tres cuartas partes de lo que se dibuja
    /// están dentro y el resto choca contra el borde, la que se ha quedado
    /// corta es la superficie, y nadie lo va a ver en `--comprobar` porque
    /// dónde acaba una tarjeta es una cuenta que solo existe mientras corre.
    fn mirar_el_corte(&mut self, forma: [f32; 4]) {
        if self.corte.dicha {
            return;
        }
        let (w, alto) = self.suya;
        let (mide_x, mide_y) = (forma[2] - forma[0], forma[3] - forma[1]);
        if mide_x <= 0.5 || mide_y <= 0.5 {
            return;
        }
        let falta = [-forma[0], -forma[1], forma[2] - w, forma[3] - alto];
        let dentro_x = (forma[2].min(w) - forma[0].max(0.0)).max(0.0) / mide_x;
        let dentro_y = (forma[3].min(alto) - forma[1].max(0.0)).max(0.0) / mide_y;
        let casi = [dentro_x, dentro_y, dentro_x, dentro_y];
        let lados = ["on the left", "above", "on the right", "below"];
        let vale = |k: usize| !self.pegada[k] && falta[k] > 0.5 && casi[k] >= 0.75;
        let dichos: Vec<String> = (0..4).filter(|&k| vale(k)).map(|k| format!("{:.0} px {}", falta[k].ceil(), lados[k])).collect();
        if dichos.is_empty() {
            return;
        }
        let peor = (0..4).filter(|&k| vale(k)).map(|k| falta[k]).fold(0.0f32, f32::max);
        self.corte.apunta(peor, || {
            format!(
                "render · a drawing is cut: it needs {} more than this {:.0} x {:.0} surface has. Almost all of it is inside, so it looks like the surface is the one that fell short",
                dichos.join(" and "),
                w,
                alto
            )
        });
    }

    /// Lo de la sombra, cuando ya se sabe del todo: la escena se ha quedado
    /// quieta, o la cuenta lleva un cuarto de segundo sin crecer.
    pub fn decir_lo_pendiente(&mut self) {
        for dicho in [self.sombra.ya(), self.corte.ya()].into_iter().flatten() {
            eprintln!("{dicho}");
        }
    }

    /// Que el compositor desenfoque lo que hay detrás de estas formas: sus
    /// franjas, cada una por separado —la unión de sus siluetas es la del
    /// cuerpo, menos el cuello donde dos se funden, que es poco— y recortadas.
    fn pedir_cristal(&mut self, planas: &[crate::formas::Plana], recortes: &[(usize, [f32; 4])]) {
        let mut corte = [f32::MIN, f32::MIN, f32::MAX, f32::MAX];
        for (_, r) in recortes {
            corte = [corte[0].max(r[0]), corte[1].max(r[1]), corte[2].min(r[2]), corte[3].min(r[3])];
        }
        for p in planas {
            let (ox, oy) = p.afin.aplicar(p.cx, p.cy);
            let mut en_origen = *p;
            (en_origen.cx, en_origen.cy, en_origen.afin.t) = (0.0, 0.0, [0.0, 0.0]);
            // Un camino es sus puntos, que la clave no ve: ese se mide siempre.
            let franjas = if p.tipo == 4 {
                franjas_de(&en_origen, &self.puntos)
            } else {
                let clave = [en_origen.tipo as f32, en_origen.mx, en_origen.my, en_origen.radio, en_origen.giro, en_origen.ex, en_origen.ey, en_origen.trazo, en_origen.afin.m[0], en_origen.afin.m[1], en_origen.afin.m[2], en_origen.afin.m[3]].map(f32::to_bits);
                let puesta = self.cache_de_cristales.entry(clave).or_insert_with(|| (franjas_de(&en_origen, &[]), false));
                puesta.1 = true;
                puesta.0.clone()
            };
            self.cristales.extend(franjas.into_iter().map(|f| [(f[0] + ox).max(corte[0]), (f[1] + oy).max(corte[1]), (f[2] + ox).min(corte[2]), (f[3] + oy).min(corte[3])]).filter(|f| f[2] > f[0] && f[3] > f[1]));
        }
    }

    /// Un elemento solo existe si su caja, recortada, toca la pantalla.
    fn elemento(&mut self, tipo: f32, caja: [f32; 4], recortes: &[(usize, [f32; 4])], rellenar: impl FnOnce(&mut [f32])) {
        // Lo que no cae en la superficie ni en ninguna emergente abierta, no existe.
        let toca = |v: &[f32; 4]| caja[0] < v[2] && caja[2] > v[0] && caja[1] < v[3] && caja[3] > v[1];
        let marco = [0.0, 0.0, self.tam.0, self.tam.1];
        let marco = if toca(&marco) { marco } else { self.vistas.iter().copied().find(|v| toca(v)).unwrap_or(marco) };
        let mut c = [caja[0].max(marco[0]), caja[1].max(marco[1]), caja[2].min(marco[2]), caja[3].min(marco[3])];
        for (_, r) in recortes {
            c = [c[0].max(r[0]), c[1].max(r[1]), c[2].min(r[2]), c[3].min(r[3])];
        }
        if c[2] <= c[0] || c[3] <= c[1] {
            return;
        }
        let k = self.elementos.len();
        self.elementos.resize(k + POR_ELEMENTO, 0.0);
        let e = &mut self.elementos[k..];
        e[0] = tipo;
        e[4..8].copy_from_slice(&c);
        for j in 0..4 {
            // Caben cuatro: si hay más, los de más adentro. La caja sí es la de todos.
            e[32 + j] = recortes[recortes.len().saturating_sub(4)..].get(j).map_or(-1.0, |r| r.0 as f32);
        }
        rellenar(e);
    }

    /// A qué bordes está pegada la superficie, que lo sabe quien la pide.
    pub fn pegada_a(&mut self, lados: [bool; 4]) {
        self.pegada = lados;
    }

    pub fn componer(&mut self, instrs: &[Instr], c: Ctx, textos: &[String], tip: &mut Textos, campo: Option<VistaDeCampo>, tam: (f32, f32), hud: bool) {
        self.medidas.clear();
        self.campos.clear();
        self.tam = tam;
        self.suya = (tam.0, tam.1 - if hud { ALTO_INSTRUMENTOS } else { 0.0 });
        self.formas.clear();
        self.puntos.clear();
        self.elementos.clear();
        let mut paradas_puestas: Vec<f32> = Vec::new();
        self.apartes.clear();
        self.cristales.clear();
        let mut recortes: Vec<(usize, [f32; 4])> = Vec::new();
        // Cada entrada es ya el producto de todas las de encima.
        let mut giros: Vec<Afin> = Vec::new();
        let mut opacos: Vec<GrupoOpaco> = Vec::new();
        let mut cuerpo: Option<CuerpoAbierto> = None;
        let aplanar = |f: &Forma, giros: &[Afin], pts: &mut Vec<f32>| {
            let mut p = f.aplanar_en(c, pts);
            p.afin = giros.last().copied().unwrap_or(Afin::IDENTIDAD);
            p
        };
        let color = |col: &Color| [col[0].evaluar(c), col[1].evaluar(c), col[2].evaluar(c)];
        let tip_escala = tip.escala();

        let mut saltar = self.saltar.clone();
        saltar.sort_by_key(|r| r.start);
        let mut salto = saltar.into_iter().peekable();
        for (sitio, i) in instrs.iter().enumerate() {
            if let Some(r) = salto.peek() {
                if r.contains(&sitio) {
                    continue;
                }
                if sitio >= r.end {
                    salto.next();
                }
            }
            let oculto = opacos.iter().any(|g| matches!(g, GrupoOpaco::Oculto));
            // Lo que multiplica a cada elemento: los grupos que no tienen capa propia.
            let veces: f32 = opacos.iter().map(|g| if let GrupoOpaco::Multiplica(a) = g { *a } else { 1.0 }).product();
            let afin = giros.last().copied().unwrap_or(Afin::IDENTIDAD);
            match i {
                Instr::Opacidad(Some(a)) => {
                    let a = a.evaluar(c).clamp(0.0, 1.0);
                    let dentro_de_capa = opacos.iter().any(|g| matches!(g, GrupoOpaco::Capa { .. }));
                    opacos.push(if a <= 0.001 {
                        GrupoOpaco::Oculto
                    } else if a >= 0.999 || dentro_de_capa || self.apartes.len() >= MAX_CAPAS {
                        GrupoOpaco::Multiplica(a)
                    } else {
                        GrupoOpaco::Capa { alfa: a, indice: self.apartes.len(), primer_elemento: self.n_elementos() }
                    });
                    if let Some(GrupoOpaco::Capa { indice, primer_elemento, .. }) = opacos.last() {
                        self.apartes.push((*primer_elemento as u32..*primer_elemento as u32, *indice));
                    }
                }
                Instr::Opacidad(None) => {
                    if let Some(GrupoOpaco::Capa { alfa, indice, primer_elemento }) = opacos.pop() {
                        let fin = self.n_elementos();
                        self.apartes[indice].0 = primer_elemento as u32..fin as u32;
                        // La caja del grupo es la unión de las de dentro.
                        let caja = (primer_elemento..fin).fold(None, |u, k| {
                            let e = &self.elementos[k * POR_ELEMENTO + 4..k * POR_ELEMENTO + 8];
                            Some(unir(u, [e[0], e[1], e[2], e[3]]))
                        });
                        if let Some(caja) = caja {
                            self.elemento(2.0, caja, &[], |e| {
                                e[1] = indice as f32;
                                e[3] = alfa * veces;
                            });
                        }
                    }
                }
                _ if oculto => {}
                Instr::Grupo { sombra } => {
                    cuerpo = Some(CuerpoAbierto { primera: self.formas.len() / POR_FORMA, n: 0, caja: None, holgura: 0.0, sombra: sombra.clone(), planas: Vec::new() })
                }
                Instr::Forma { forma, fusion } => {
                    let p = aplanar(forma, &giros, &mut self.puntos);
                    let k = fusion.evaluar(c).max(0.0);
                    let caja = p.caja();
                    self.forma(p, k);
                    if let Some(g) = &mut cuerpo {
                        g.planas.push(p);
                        g.n += 1;
                        g.holgura = g.holgura.max(k * 0.5);
                        if let Some(b) = caja {
                            g.caja = Some(unir(g.caja, b));
                        }
                    }
                }
                Instr::Relleno { pintura, alfa, filo, luz, borde, vidrio } => {
                    let Some(g) = cuerpo.take() else { continue };
                    let Some(mut caja) = g.caja else { continue };
                    let forma_sola = caja;
                    let h = g.holgura + 2.0;
                    caja = [caja[0] - h, caja[1] - h, caja[2] + h, caja[3] + h];
                    //  Sus números se leen aquí, que es donde se sabe cómo está
                    //  la escena ahora mismo: una sombra puede ir cambiando.
                    let sombra = g.sombra.as_ref().map(|s| (s.desplazada.0.evaluar(c), s.desplazada.1.evaluar(c), s.difusa.evaluar(c).max(0.0), s.alfa.evaluar(c).clamp(0.0, 1.0)));
                    if let Some((sx, sy, sd, _)) = sombra {
                        let d = sd + 2.0;
                        caja = unir(Some(caja), [caja[0] + sx - d, caja[1] + sy - d, caja[2] + sx + d, caja[3] + sy + d]);
                    }
                    let a = alfa.evaluar(c).clamp(0.0, 1.0) * veces;
                    if let Some((sx, sy, sd, sa)) = sombra.filter(|s| a > 0.01 && s.3 > 0.01) {
                        self.mirar_la_sombra(forma_sola, (sx, sy, sd));
                        let _ = sa;
                    }
                    if a > 0.01 {
                        self.mirar_el_corte(forma_sola);
                    }
                    // Un cristal que se ve pide que se desenfoque lo de detrás, por su silueta.
                    let v = vidrio.as_ref().map_or(0.0, |v| v.evaluar(c));
                    if v * a > CRISTAL_VISIBLE {
                        self.pedir_cristal(&g.planas, &recortes);
                    }
                    self.elemento(0.0, caja, &recortes, |e| {
                        afin.codificar(&mut e[44..52]);
                        e[1] = g.primera as f32;
                        e[2] = g.n as f32;
                        e[3] = a;
                        match pintura {
                            Pintura::Color(col) => e[8..11].copy_from_slice(&color(col)),
                            Pintura::Degradado { radial, de, a, paradas } => {
                                e[15] = if *radial { 2.0 } else { 1.0 };
                                e[16..20].copy_from_slice(&[de.0.evaluar(c), de.1.evaluar(c), a.0.evaluar(c), a.1.evaluar(c)]);
                                // Las paradas van en su almacén: dónde cae cada una y de qué color.
                                e[36] = (paradas_puestas.len() / 4) as f32;
                                e[37] = paradas.len() as f32;
                                for (donde, col) in paradas {
                                    let rgb = color(col);
                                    paradas_puestas.extend_from_slice(&[rgb[0], rgb[1], rgb[2], donde.evaluar(c).clamp(0.0, 1.0)]);
                                }
                            }
                        }
                        e[11] = *filo;
                        // Un cuerpo no usa `uv`, que es de las texturas: ahí va el cristal.
                        e[40] = v;
                        if let Some(l) = luz {
                            e[20..23].copy_from_slice(&[l.cantidad, l.desde_y.evaluar(c), l.alto]);
                        }
                        if let Some((grosor, col)) = borde {
                            e[23] = grosor.evaluar(c).max(0.0);
                            e[24..27].copy_from_slice(&color(col));
                        }
                        if let Some((sx, sy, sd, sa)) = sombra {
                            e[28..32].copy_from_slice(&[sx, sy, sd, sa]);
                            //  Su color va en los tres huecos de `color1`, que
                            //  solo usaba el cuarto para decir si hay degradado.
                            if let Some(col) = g.sombra.as_ref().and_then(|s| s.color.as_ref()) {
                                e[12..15].copy_from_slice(&color(col));
                            }
                        }
                    });
                }
                Instr::Plano { forma, color: col, alfa, vidrio } => {
                    let a = alfa.evaluar(c).clamp(0.0, 1.0) * veces;
                    if a <= 0.001 {
                        continue; // lo invisible no ocupa ni un quad
                    }
                    let p = aplanar(forma, &giros, &mut self.puntos);
                    let Some(b) = p.caja() else { continue };
                    let v = vidrio.as_ref().map_or(0.0, |v| v.evaluar(c).clamp(0.0, 1.0));
                    if v * a > CRISTAL_VISIBLE {
                        self.pedir_cristal(&[p], &recortes);
                    }
                    let k = self.forma(p, 0.0);
                    let rgb = color(col);
                    self.elemento(0.0, [b[0] - 2.0, b[1] - 2.0, b[2] + 2.0, b[3] + 2.0], &recortes, |e| {
                        afin.codificar(&mut e[44..52]);
                        e[1] = k as f32;
                        e[2] = 1.0;
                        e[3] = a;
                        e[8..11].copy_from_slice(&rgb);
                        e[40] = v;
                    });
                }
                Instr::Imagen { imagen, destino, alfa, tinte } => {
                    let a = alfa.evaluar(c).clamp(0.0, 1.0) * veces;
                    let Some(hueco) = tip.imagen(imagen.0 as usize, textos) else { continue };
                    if a <= 0.001 {
                        continue;
                    }
                    let d = [destino.0.evaluar(c), destino.1.evaluar(c), destino.2.evaluar(c), destino.3.evaluar(c)];
                    let rgb = tinte.as_ref().map(&color);
                    self.trozo(d, hueco.uv(), a, rgb, afin, &recortes);
                }
                Instr::Campo { texto, zona, en, ancho, estilo, alfa, marcador, seleccion, secreto } => {
                    let k = texto.0 as usize;
                    let de_verdad = textos.get(k).map_or("", String::as_str);
                    let vacio = de_verdad.is_empty();
                    // Secreto, lo que se pinta son puntos: uno por letra.
                    let puntos = if *secreto { PUNTO.repeat(de_verdad.chars().count()) } else { String::new() };
                    let valor = if *secreto { puntos.as_str() } else { de_verdad };
                    // Vacío, enseña lo que se espera de él, más tenue.
                    let m = tip.maqueta(sitio, Clave::de(if vacio { marcador } else { valor }, estilo, None));
                    let (x0, y0, w) = (en.0.evaluar(c), en.1.evaluar(c), ancho.evaluar(c));
                    let h = estilo.px * estilo.interlinea;
                    let mio = campo.filter(|v| v.texto == k);
                    // El cursor cuenta bytes del texto de verdad; en puntos, son letras por tres.
                    let x_de = |b: usize| {
                        let b = if *secreto { de_verdad[..b.min(de_verdad.len())].chars().count() * PUNTO.len() } else { b };
                        if vacio { 0.0 } else { m.as_ref().map_or(0.0, |m| m.x_de(b)) }
                    };
                    // Si el cursor se sale por la derecha, el texto se corre.
                    let corrido = mio.map_or(0.0, |v| (x_de(v.cursor) - w + 6.0).max(0.0));
                    self.campos.push(CampoPuesto { texto: k, zona, maqueta: if vacio { None } else { m.clone() }, x0, corrido, secreto: *secreto });
                    let a = alfa.evaluar(c).clamp(0.0, 1.0) * veces;
                    if a <= 0.001 {
                        continue;
                    }
                    // Recortado a su caja: lo que no cabe, no se ve.
                    let mut caja = crate::formas::Plana { tipo: 1, cx: x0 + w * 0.5, cy: y0 + h * 0.5, mx: w * 0.5, my: h * 0.5 + 2.0, radio: 0.0, giro: 0.0, ex: 1.0, ey: 1.0, trazo: 0.0, afin };
                    let limite = caja.caja().unwrap_or([0.0; 4]);
                    let kf = self.forma(caja, 0.0);
                    recortes.push((kf, limite));
                    let rgb = color(&estilo.color);
                    if let Some(v) = mio {
                        let (s0, s1) = (x_de(v.cursor.min(v.ancla)), x_de(v.cursor.max(v.ancla)));
                        if s1 > s0 {
                            caja = crate::formas::Plana { cx: x0 - corrido + (s0 + s1) * 0.5, mx: (s1 - s0) * 0.5, my: h * 0.5, ..caja };
                            let (kf, b) = (self.forma(caja, 0.0), caja.caja().unwrap_or([0.0; 4]));
                            let sel = color(seleccion);
                            self.elemento(0.0, b, &recortes, |e| {
                                afin.codificar(&mut e[44..52]);
                                e[1] = kf as f32;
                                e[2] = 1.0;
                                e[3] = a;
                                e[8..11].copy_from_slice(&sel);
                            });
                        }
                    }
                    if let Some(m) = &m {
                        let s = tip_escala;
                        let (tx, ty) = (((x0 - corrido) * s).round() / s, (y0 * s).round() / s);
                        for g in &m.glifos {
                            let d = [tx + g.rect[0], ty + g.rect[1], g.rect[2], g.rect[3]];
                            self.trozo(d, g.uv, if vacio { a * 0.4 } else { a }, if g.en_color { None } else { Some(rgb) }, afin, &recortes);
                        }
                    }
                    if let Some(v) = mio.filter(|v| v.se_ve) {
                        caja = crate::formas::Plana { cx: x0 - corrido + x_de(v.cursor) + 0.5, mx: 0.8, my: h * 0.5 - 1.0, ..caja };
                        let (kf, b) = (self.forma(caja, 0.0), caja.caja().unwrap_or([0.0; 4]));
                        self.elemento(0.0, [b[0] - 1.0, b[1], b[2] + 1.0, b[3]], &recortes, |e| {
                            afin.codificar(&mut e[44..52]);
                            e[1] = kf as f32;
                            e[2] = 1.0;
                            e[3] = a;
                            e[8..11].copy_from_slice(&rgb);
                        });
                    }
                    recortes.pop();
                }
                Instr::Texto { contenido, en, ancla, ancho, estilo, alfa, mide } => {
                    let numero;
                    let texto = match contenido {
                        Contenido::Fijo(t) => t.as_str(),
                        Contenido::Vivo(id) => textos.get(id.0 as usize).map_or("", String::as_str),
                        Contenido::Numero(e, decimales, detras) => {
                            numero = format!("{:.*}{detras}", *decimales as usize, e.evaluar(c));
                            numero.as_str()
                        }
                        Contenido::Plantilla(trozos) => {
                            let mut montado = String::new();
                            Trozo::escribir(trozos, c, textos, &mut montado);
                            numero = montado;
                            numero.as_str()
                        }
                    };
                    // Se encarga aunque no se vea: cuando aparezca, que ya esté.
                    let clave = Clave::de(texto, estilo, ancho.as_ref().map(|w| w.evaluar(c)));
                    let Some(m) = tip.maqueta(sitio, clave) else { continue };
                    if let Some((w, h)) = mide {
                        self.medidas.push((*w, m.tam.0));
                        self.medidas.push((*h, m.tam.1));
                    }
                    let a = alfa.evaluar(c).clamp(0.0, 1.0) * veces;
                    if a <= 0.001 {
                        continue;
                    }
                    let rgb = color(&estilo.color);
                    // A píxeles de verdad: un texto a medio píxel sale blando.
                    let s = tip_escala;
                    let x0 = ((en.0.evaluar(c) - m.tam.0 * ancla.0) * s).round() / s;
                    let y0 = ((en.1.evaluar(c) - m.tam.1 * ancla.1) * s).round() / s;
                    for g in &m.glifos {
                        let d = [x0 + g.rect[0], y0 + g.rect[1], g.rect[2], g.rect[3]];
                        self.trozo(d, g.uv, a, if g.en_color { None } else { Some(rgb) }, afin, &recortes);
                    }
                }
                Instr::Recorte(Some((forma, margen))) => {
                    let p = aplanar(forma, &giros, &mut self.puntos).encoger(*margen);
                    let caja = p.caja().unwrap_or([0.0; 4]);
                    let k = self.forma(p, 0.0);
                    recortes.push((k, [caja[0] - 1.0, caja[1] - 1.0, caja[2] + 1.0, caja[3] + 1.0]));
                    if recortes.len() == 5 && !std::mem::replace(&mut self.avisado_de_recortes, true) {
                        eprintln!("render · more than four nested clips: the outer ones clip by their box only, not by their shape");
                    }
                }
                Instr::Recorte(None) => {
                    recortes.pop();
                }
                Instr::Transformar(Some(t)) => {
                    let propia = t.afin(c);
                    giros.push(afin.por(propia));
                }
                Instr::Transformar(None) => {
                    giros.pop();
                }
            }
        }
        for dicho in [self.sombra.cierra_el_frame(true), self.corte.cierra_el_frame(false)].into_iter().flatten() {
            eprintln!("{dicho}");
        }
        // Las siluetas que nadie ha usado este frame se olvidan; las demás, a
        // empezar de nuevo la cuenta.
        self.cache_de_cristales.retain(|_, (_, usada)| std::mem::take(usada));
        if hud {
            // Los instrumentos ocupan la franja de abajo, que se añadió para ellos.
            self.elemento(9.0, [0.0, tam.1 - ALTO_INSTRUMENTOS, tam.0, tam.1], &[], |e| Afin::IDENTIDAD.codificar(&mut e[44..52]));
        }
        self.paradas = paradas_puestas;
        // Un almacén vacío no se puede enlazar ni escribir.
        if self.paradas.is_empty() {
            self.paradas.resize(4, 0.0);
        }
        if self.puntos.is_empty() {
            self.puntos.resize(2, 0.0);
        }
        if self.formas.is_empty() {
            self.formas.resize(POR_FORMA, 0.0);
        }
        if self.elementos.is_empty() {
            self.elementos.resize(POR_ELEMENTO, 0.0);
        }
    }
}

// ── el dispositivo y las láminas ────────────────────────────────

/// Lo que el hilo de Wayland le entrega al de render por cada superficie.
pub struct NuevaLamina {
    pub id: u32,
    pub superficie: wgpu::Surface<'static>,
    pub ventana: Box<dyn Ventana>,
    pub escala: f32,
    /// El tamaño lógico que le ha dado el compositor.
    pub tam: (u32, u32),
    /// Milihercios del monitor; 0 si no se sabe.
    pub mhz: i32,
    pub nombre: String,
    pub vista: Vista,
}

/// Qué trozo del plano de la escena enseña una lámina, y de quién es. Cada superficie
/// —y cada emergente— mira a un sitio distinto del mismo plano.
#[derive(Clone, Copy, Debug)]
pub struct Vista {
    /// Qué superficie de la escena es.
    pub superficie: usize,
    /// Y si es una emergente, cuál.
    pub emergente: Option<usize>,
    pub origen: (f32, f32),
    pub tam: (f32, f32),
}

impl Vista {
    pub fn caja(&self) -> [f32; 4] {
        [self.origen.0, self.origen.1, self.origen.0 + self.tam.0, self.origen.1 + self.tam.1]
    }
}

/// Una superficie de Wayland vista desde la GPU: su cadena de imágenes, a su
/// escala, con sus capas y sus uniformes.
pub struct Lamina {
    pub id: u32,
    #[allow(dead_code)]
    pub nombre: String,
    pub mhz: i32,
    pub escala: f32,
    /// La que espera a la pantalla y marca el ritmo; las demás no bloquean.
    pub marca_el_ritmo: bool,
    /// Si su superficie está abierta ahora. Una cerrada no marca el ritmo ni se
    /// pinta más que una vez, para vaciarla: el compositor no le da frames a lo
    /// que no se ve, y esperarlos con vsync paraba el render entero.
    pub abierta: bool,
    pub vaciada: bool,
    pub vista: Vista,
    superficie: wgpu::Surface<'static>,
    ventana: Box<dyn Ventana>,
    px: (u32, u32),
    uniformes: wgpu::Buffer,
    grupo_uniformes: wgpu::BindGroup,
    vistas_de_capa: Vec<wgpu::TextureView>,
    grupo_capas: wgpu::BindGroup,
    /// Cuántas capas tiene del tamaño de la superficie (0: solo la de mentira),
    /// y cuántos frames lleva sin usar ninguna.
    capas: u32,
    capas_ociosas: u32,
    /// Lo último que se le pidió al compositor que desenfocase, en sus coordenadas.
    pub desenfoque: Vec<[i32; 4]>,
    /// Qué trozo del plano y a qué escala tiene pintado, si lo que enseña está
    /// al día. Con otro sitio, otra escala o sin nada (recién hecha,
    /// reconfigurada), se pinta aunque la escena no haya cambiado.
    pub pintada: Option<([f32; 4], f32)>,
}

pub struct Gpu {
    adaptador: wgpu::Adapter,
    pub dispositivo: wgpu::Device,
    pub cola: wgpu::Queue,
    formato: wgpu::TextureFormat,
    alfa: wgpu::CompositeAlphaMode,
    sin_bloqueo: Option<wgpu::PresentMode>,
    tuberia: wgpu::RenderPipeline,
    bufer_formas: wgpu::Buffer,
    bufer_puntos: wgpu::Buffer,
    bufer_paradas: wgpu::Buffer,
    bufer_elementos: wgpu::Buffer,
    atlas: wgpu::Texture,
    vista_del_atlas: wgpu::TextureView,
    muestreo: wgpu::Sampler,
    /// Cuántos floats caben ahora en cada almacén, y cuántos como mucho en esta tarjeta.
    caben: (usize, usize, usize, usize),
    tope: usize,
    avisado_del_tope: bool,
    grupo_escena: wgpu::BindGroup,
    grupo_sin_capas: wgpu::BindGroup,
}

impl Gpu {
    /// Se crea con la primera superficie que llega: hace falta una para saber
    /// qué adaptador y qué formato valen.
    pub fn nueva(instancia: &wgpu::Instance, primera: &wgpu::Surface<'static>) -> Gpu {
        let adaptador = pollster::block_on(instancia.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(primera),
            power_preference: wgpu::PowerPreference::LowPower,
            ..Default::default()
        }))
        .expect("there is no graphics adapter");
        let (dispositivo, cola) =
            pollster::block_on(adaptador.request_device(&wgpu::DeviceDescriptor {
                // Por defecto, wgpu reserva bloques de 128 MB en la tarjeta y 64 en
                // la memoria del sistema, pensando en un juego. Una escena entera
                // cabe en menos de 20: con bloques de 8, lo reservado es lo usado.
                // Pedir memoria es raro aquí —crecer un almacén, una capa—, así que
                // lo que se pierde en rapidez no se nota.
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                ..Default::default()
            })).expect("there is no device");
        let caps = primera.get_capabilities(&adaptador);
        // Sin sRGB: el compositor mezcla los bytes tal cual, y el alfa
        // premultiplicado solo cuadra si nadie los recodifica por el camino.
        let formato = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let alfa = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else {
            eprintln!("warning: no premultiplied alpha ({:?}); the background will come out opaque", caps.alpha_modes);
            wgpu::CompositeAlphaMode::Auto
        };
        let sin_bloqueo = [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate].into_iter().find(|m| caps.present_modes.contains(m));
        let info = adaptador.get_info();
        println!("render · {} ({:?}) · {:?} · {:?}", info.name, info.backend, formato, alfa);

        let almacen = |etiqueta, floats: usize| {
            dispositivo.create_buffer(&wgpu::BufferDescriptor {
                label: Some(etiqueta),
                size: (floats * 4) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let bufer_formas = almacen("formas", FORMAS_DE_SALIDA * POR_FORMA);
        let bufer_puntos = almacen("puntos", PUNTOS_DE_SALIDA);
        let bufer_paradas = almacen("paradas", PARADAS_DE_SALIDA);
        let bufer_elementos = almacen("elementos", ELEMENTOS_DE_SALIDA * POR_ELEMENTO);
        let muestreo = dispositivo.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let modulo = dispositivo.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("forma"),
            source: wgpu::ShaderSource::Wgsl(include_str!("forma.wgsl").into()),
        });
        let tuberia = dispositivo.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("elementos"),
            layout: None,
            vertex: wgpu::VertexState { module: &modulo, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &modulo,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: formato,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        // Un solo atlas para glifos e imágenes. 2048² en RGBA son 16 MB.
        let atlas = dispositivo.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas"),
            size: wgpu::Extent3d { width: LADO_DEL_ATLAS, height: LADO_DEL_ATLAS, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let vista_del_atlas = atlas.create_view(&Default::default());
        let grupo_escena = Self::grupo_de_escena(&dispositivo, &tuberia, &bufer_formas, &bufer_elementos, &bufer_puntos, &bufer_paradas, &vista_del_atlas, &muestreo);
        let tope = dispositivo.limits().max_storage_buffer_binding_size as usize / 4;
        let grupo_sin_capas = Self::capas_de(&dispositivo, &tuberia, formato, 1, 1, 1).1;
        Gpu { adaptador, dispositivo, cola, formato, alfa, sin_bloqueo, tuberia, bufer_formas, bufer_elementos, bufer_puntos, bufer_paradas, atlas, vista_del_atlas, muestreo, caben: (FORMAS_DE_SALIDA * POR_FORMA, ELEMENTOS_DE_SALIDA * POR_ELEMENTO, PUNTOS_DE_SALIDA, PARADAS_DE_SALIDA), tope, avisado_del_tope: false, grupo_escena, grupo_sin_capas }
    }

    /// Sube al atlas lo que el taller haya pintado desde la última vez.
    /// Si se presenta por buzón y el paso lo marca el render con su reloj.
    ///
    /// Con vsync de cola (`Fifo`) pedir la siguiente imagen espera a que el
    /// compositor suelte una, y aquí —Hyprland con NVIDIA— a veces no la suelta:
    /// cuatro frames después de despertar de un reposo largo, 300 ms parado en
    /// `vkAcquireNextImage` en mitad de una animación. Con buzón nunca se espera
    /// a nadie; lo que se pierde es ir clavado al refresco, y se recupera
    /// marcando el paso con plazos absolutos al periodo del monitor.
    /// `PLEAMAR_FIFO=1` vuelve a lo de antes, para comparar.
    pub fn con_buzon(&self) -> bool {
        self.sin_bloqueo == Some(wgpu::PresentMode::Mailbox) && std::env::var_os("PLEAMAR_FIFO").is_none()
    }

    pub fn subir_atlas(&self, por_subir: &mut Vec<(Hueco, Vec<u8>)>) {
        for (h, rgba) in por_subir.drain(..) {
            self.cola.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.atlas, mip_level: 0, origin: wgpu::Origin3d { x: h.x, y: h.y, z: 0 }, aspect: wgpu::TextureAspect::All },
                &rgba,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * h.ancho), rows_per_image: None },
                wgpu::Extent3d { width: h.ancho, height: h.alto, depth_or_array_layers: 1 },
            );
        }
    }

    /// Las capas donde se pintan aparte los grupos con opacidad. Mientras se
    /// pinta EN una no se puede leer de ellas, y ese rato se enlaza una de mentira.
    /// Cada capa mide lo que la superficie entera: se piden solo las que hacen falta.
    fn capas_de(dispositivo: &wgpu::Device, tuberia: &wgpu::RenderPipeline, formato: wgpu::TextureFormat, ancho: u32, alto: u32, n: u32) -> (Vec<wgpu::TextureView>, wgpu::BindGroup) {
        let t = dispositivo.create_texture(&wgpu::TextureDescriptor {
            label: Some("capas"),
            size: wgpu::Extent3d { width: ancho.max(1), height: alto.max(1), depth_or_array_layers: n.max(1) },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: formato,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let todas = t.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let cada = (0..n.max(1))
            .map(|k| {
                t.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: k,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let grupo = dispositivo.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &tuberia.get_bind_group_layout(1),
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&todas) }],
        });
        (cada, grupo)
    }

    pub fn lamina(&self, n: NuevaLamina, tam: (f32, f32)) -> Lamina {
        let uniformes = self.dispositivo.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniformes"),
            size: (N_UNIFORMES * 4) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let grupo_uniformes = self.dispositivo.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.tuberia.get_bind_group_layout(2),
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniformes.as_entire_binding() }],
        });
        let (vistas_de_capa, grupo_capas) = Self::capas_de(&self.dispositivo, &self.tuberia, self.formato, 1, 1, 1);
        let mut l = Lamina {
            id: n.id, nombre: n.nombre, mhz: n.mhz, escala: n.escala, marca_el_ritmo: true, abierta: true, vaciada: false, vista: n.vista,
            superficie: n.superficie, ventana: n.ventana, px: (0, 0), uniformes, grupo_uniformes, vistas_de_capa, grupo_capas, capas: 0, capas_ociosas: 0, desenfoque: Vec::new(), pintada: None,
        };
        self.configurar(&mut l, tam);
        l
    }

    /// Al cambiar de escala, de tamaño o de papel en el ritmo.
    pub fn configurar(&self, l: &mut Lamina, _tam: (f32, f32)) {
        let tam = l.vista.tam;
        let px = ((tam.0 * l.escala).round().max(1.0) as u32, (tam.1 * l.escala).round().max(1.0) as u32);
        self.superficie_configurar(l, px);
        l.pintada = None;
        if px != l.px {
            l.px = px;
            // Las que hubiera ya no miden lo que la superficie: se piden de nuevo cuando hagan falta.
            self.soltar_capas(l);
        }
    }

    fn soltar_capas(&self, l: &mut Lamina) {
        (l.vistas_de_capa, l.grupo_capas) = Self::capas_de(&self.dispositivo, &self.tuberia, self.formato, 1, 1, 1);
        l.capas = 0;
        l.capas_ociosas = 0;
    }

    /// Que haya al menos `n` capas del tamaño de la superficie; y si lleva un
    /// rato sin necesitar ninguna, devolverlas. Un grupo que se funde al abrir
    /// un panel las pide un momento, no para siempre.
    fn capas_para(&self, l: &mut Lamina, n: u32) {
        if n > l.capas {
            (l.vistas_de_capa, l.grupo_capas) = Self::capas_de(&self.dispositivo, &self.tuberia, self.formato, l.px.0, l.px.1, n);
            l.capas = n;
        }
        if n == 0 && l.capas > 0 {
            l.capas_ociosas += 1;
            if l.capas_ociosas > CAPAS_OCIOSAS {
                self.soltar_capas(l);
            }
        } else {
            l.capas_ociosas = 0;
        }
    }

    fn superficie_configurar(&self, l: &Lamina, px: (u32, u32)) {
        let _ = &self.adaptador;
        l.superficie.configure(
            &self.dispositivo,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: self.formato,
                view_formats: vec![],
                alpha_mode: self.alfa,
                width: px.0,
                height: px.1,
                desired_maximum_frame_latency: 1,
                // Solo una lámina espera a su pantalla. Si esperasen todas, dos
                // monitores a distinto ritmo se frenarían el uno al otro.
                // Con buzón, ninguna espera a la pantalla: el paso lo marca el render
                // (ver `con_buzon`). Sin él, la que marca el ritmo espera con vsync.
                present_mode: if l.marca_el_ritmo && !self.con_buzon() { wgpu::PresentMode::Fifo } else { self.sin_bloqueo.unwrap_or(wgpu::PresentMode::Fifo) },
                color_space: wgpu::SurfaceColorSpace::Auto,
            },
        );
    }

    fn grupo_de_escena(dispositivo: &wgpu::Device, tuberia: &wgpu::RenderPipeline, formas: &wgpu::Buffer, elementos: &wgpu::Buffer, puntos: &wgpu::Buffer, paradas: &wgpu::Buffer, atlas: &wgpu::TextureView, muestreo: &wgpu::Sampler) -> wgpu::BindGroup {
        dispositivo.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &tuberia.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: formas.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: elementos.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(atlas) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(muestreo) },
                wgpu::BindGroupEntry { binding: 4, resource: puntos.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: paradas.as_entire_binding() },
            ],
        })
    }

    /// Lo que se va a pintar, a la tarjeta. Si no cabe, los almacenes crecen al
    /// doble —las veces que haga falta— y no vuelven a encoger.
    pub fn subir(&mut self, d: &Dibujo) {
        let pide = (d.formas.len(), d.elementos.len(), d.puntos.len(), d.paradas.len());
        if pide.0 > self.caben.0 || pide.1 > self.caben.1 || pide.2 > self.caben.2 || pide.3 > self.caben.3 {
            let crecer = |cabe: usize, pide: usize| if pide > cabe { pide.next_power_of_two() } else { cabe }.min(self.tope);
            let nuevo = (crecer(self.caben.0, pide.0) / POR_FORMA * POR_FORMA, crecer(self.caben.1, pide.1) / POR_ELEMENTO * POR_ELEMENTO, crecer(self.caben.2, pide.2) / 2 * 2, crecer(self.caben.3, pide.3) / 4 * 4);
            if nuevo != self.caben {
                let almacen = |etiqueta, floats: usize| self.dispositivo.create_buffer(&wgpu::BufferDescriptor { label: Some(etiqueta), size: (floats * 4) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
                if nuevo.0 != self.caben.0 { self.bufer_formas = almacen("formas", nuevo.0) }
                if nuevo.1 != self.caben.1 { self.bufer_elementos = almacen("elementos", nuevo.1) }
                if nuevo.2 != self.caben.2 { self.bufer_puntos = almacen("puntos", nuevo.2) }
                if nuevo.3 != self.caben.3 { self.bufer_paradas = almacen("paradas", nuevo.3) }
                self.grupo_escena = Self::grupo_de_escena(&self.dispositivo, &self.tuberia, &self.bufer_formas, &self.bufer_elementos, &self.bufer_puntos, &self.bufer_paradas, &self.vista_del_atlas, &self.muestreo);
                self.caben = nuevo;
                println!("render · the scene has grown: now {} shapes and {} elements fit", nuevo.0 / POR_FORMA, nuevo.1 / POR_ELEMENTO);
            }
            if (pide.0 > self.caben.0 || pide.1 > self.caben.1) && !std::mem::replace(&mut self.avisado_del_tope, true) {
                eprintln!("render · this card cannot take more than {} shapes and {} elements: the rest is not painted", self.caben.0 / POR_FORMA, self.caben.1 / POR_ELEMENTO);
            }
        }
        let (f, e, pt, pa) = (pide.0.min(self.caben.0), pide.1.min(self.caben.1), pide.2.min(self.caben.2), pide.3.min(self.caben.3));
        self.cola.write_buffer(&self.bufer_formas, 0, bytemuck::cast_slice(&d.formas[..f]));
        self.cola.write_buffer(&self.bufer_elementos, 0, bytemuck::cast_slice(&d.elementos[..e]));
        self.cola.write_buffer(&self.bufer_puntos, 0, bytemuck::cast_slice(&d.puntos[..pt]));
        self.cola.write_buffer(&self.bufer_paradas, 0, bytemuck::cast_slice(&d.paradas[..pa]));
    }

    /// Pinta el dibujo en una lámina. Devuelve si llegó a presentarse.
    pub fn pintar(&self, l: &mut Lamina, d: &Dibujo, uniformes: &[f32], pedir_frame: bool) -> bool {
        // Solo las capas de los grupos que caen en esta superficie: el que se
        // funde en otro monitor no le cuesta memoria ni un pase a esta. Lo que
        // funde la capa ocupa la unión de lo de dentro, así que si eso no toca
        // la vista, la capa no se lee.
        let v = l.vista.caja();
        let hacen_falta = d.apartes.iter().filter(|(t, _)| d.toca_la_vista(t, v)).map(|(_, c)| *c as u32 + 1).max().unwrap_or(0);
        self.capas_para(l, hacen_falta);
        let mut u = uniformes.to_vec();
        u[3] = l.escala;
        let (v, o) = (l.vista.tam, l.vista.origen);
        (u[0], u[1], u[4], u[7]) = (v.0, v.1, o.0, o.1);
        self.cola.write_buffer(&l.uniformes, 0, bytemuck::cast_slice(&u));
        // Con `PLEAMAR_CRONO=1`, cuánto se va en pedir el hueco de la pantalla y
        // cuánto en mandarle el trabajo a la tarjeta. Es la cuenta que dice si un
        // frame cuesta por lo que dibuja o por esperar al monitor.
        let crono = crono();
        let t0 = crono.then(std::time::Instant::now);
        let marco = match l.superficie.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.superficie_configurar(l, l.px);
                return false;
            }
            _ => return false,
        };
        let vista = marco.texture.create_view(&Default::default());
        let mut codificador = self.dispositivo.create_command_encoder(&Default::default());
        let pase_a = |codificador: &mut wgpu::CommandEncoder, destino: &wgpu::TextureView, capas: &wgpu::BindGroup, tramos: &[Range<u32>]| {
            let mut pase = codificador.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: destino,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pase.set_pipeline(&self.tuberia);
            pase.set_bind_group(0, &self.grupo_escena, &[]);
            pase.set_bind_group(1, capas, &[]);
            pase.set_bind_group(2, &l.grupo_uniformes, &[]);
            // Nunca más allá de lo que se subió (solo pasa si la tarjeta no dio para todo).
            let subidos = (self.caben.1 / POR_ELEMENTO) as u32;
            for t in tramos.iter().map(|t| t.start.min(subidos)..t.end.min(subidos)).filter(|t| !t.is_empty()) {
                pase.draw(0..6, t);
            }
        };
        // Primero los grupos con opacidad, cada uno a su capa…
        let mut principal: Vec<Range<u32>> = Vec::new();
        let mut desde = 0u32;
        for (tramo, capa) in &d.apartes {
            if l.abierta && l.capas as usize > *capa && d.toca_la_vista(tramo, l.vista.caja()) {
                pase_a(&mut codificador, &l.vistas_de_capa[*capa], &self.grupo_sin_capas, std::slice::from_ref(tramo));
            }
            principal.push(desde..tramo.start);
            desde = tramo.end;
        }
        principal.push(desde..d.n_elementos() as u32);
        // Cerrada, se vacía: transparente y nada encima. No vale pintar lo que
        // haya en el dibujo, porque se cierra en mitad de un frame —el dibujo se
        // compuso cuando aún estaba abierta— y ese frame se quedaba para
        // siempre: en Marea, un trozo de bolita a punto de salir por el borde.
        if !l.abierta {
            principal.clear();
        }
        // …y luego todo lo demás, con las capas ya hechas entre medias.
        pase_a(&mut codificador, &vista, &l.grupo_capas, &principal);
        let t1 = crono.then(std::time::Instant::now);
        let orden = codificador.finish();
        let t2 = crono.then(std::time::Instant::now);
        self.cola.submit(Some(orden));
        let t3 = crono.then(std::time::Instant::now);
        if pedir_frame {
            l.ventana.pedir_frame();
        }
        self.cola.present(marco);
        if let (Some(t0), Some(t1), Some(t2), Some(t3)) = (t0, t1, t2, t3) {
            let ms = |a: std::time::Instant, b: std::time::Instant| b.duration_since(a).as_secs_f32() * 1000.0;
            CRONO.with(|c| {
                let mut v = c.get();
                v.0 += ms(t0, t1);
                v.1 += ms(t1, t2);
                v.2 += ms(t2, t3);
                v.3 += t3.elapsed().as_secs_f32() * 1000.0;
                v.4 += 1.0;
                if v.4 >= 300.0 {
                    println!("crono  · hueco {:.2} · apuntar {:.2} · cerrar {:.2} · mandar+presentar {:.2} ms", v.0 / v.4, v.1 / v.4, v.2 / v.4, v.3 / v.4);
                    // Lo que ocupa en la tarjeta: lo pedido de verdad, no los bloques que reserva el asignador.
                    if let Some(r) = self.dispositivo.generate_allocator_report() {
                        println!("crono  · memoria de la tarjeta: {:.1} MB en uso ({:.1} MB reservados)", r.total_allocated_bytes as f64 / 1e6, r.total_reserved_bytes as f64 / 1e6);
                    }
                    v = (0.0, 0.0, 0.0, 0.0, 0.0);
                }
                c.set(v);
            });
        }
        true
    }
}

/// `PLEAMAR_CRONO=1`: leído una vez, no en cada frame de cada lámina.
pub fn crono() -> bool {
    static CRONO: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CRONO.get_or_init(|| std::env::var_os("PLEAMAR_CRONO").is_some())
}

thread_local! {
    static CRONO: std::cell::Cell<(f32, f32, f32, f32, f32)> = const { std::cell::Cell::new((0.0, 0.0, 0.0, 0.0, 0.0)) };
}

impl Lamina {
    /// Por dónde entra el ratón. El resto de la superficie, aunque sea suya,
    /// deja pasar el clic a lo de debajo.
    pub fn region_de_entrada(&self, cajas: &[[i32; 4]]) {
        self.ventana.region_de_entrada(cajas);
    }

    pub fn cursor(&self, c: Cursor) {
        self.ventana.cursor(c);
    }

    pub fn region_de_desenfoque(&self, cajas: &[[i32; 4]]) {
        self.ventana.region_de_desenfoque(cajas);
    }

    pub fn teclado(&self, t: Teclado) {
        self.ventana.teclado(t);
    }
}

pub const N_UNIFORMES: usize = 8 + 120;

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::formas::{Afin, Plana};

    fn area(f: &[[f32; 4]]) -> f32 {
        f.iter().map(|r| (r[2] - r[0]) * (r[3] - r[1])).sum()
    }

    fn forma(tipo: u8, mx: f32, my: f32, radio: f32, ex: f32, ey: f32, escala: f32) -> Plana {
        Plana { tipo, cx: 0.0, cy: 0.0, mx, my, radio, giro: 0.0, ex, ey, trazo: 0.0, afin: Afin { m: [escala, 0.0, 0.0, escala], t: [0.0, 0.0] } }
    }

    #[test]
    fn la_formula_de_las_franjas_da_lo_mismo_que_buscarlas() {
        for p in [forma(1, 210.0, 110.0, 36.0, 1.0, 1.0, 1.0), forma(0, 0.0, 0.0, 46.0, 1.0, 1.0, 1.0), forma(0, 0.0, 0.0, 30.0, 1.0, 0.4, 1.5), forma(1, 60.0, 20.0, 20.0, 1.0, 1.0, 2.0)] {
            let exacta = franjas_de(&p, &[]);
            // Un giro que no gira: obliga a buscar el borde por bisección.
            let mut girada = p;
            girada.giro = 1e-7;
            let buscada = franjas_de(&girada, &[]);
            // Contra el área de verdad: la fórmula se queda un pelo por dentro (el
            // más estrecho de cada franja) y la búsqueda un pelo por fuera.
            let s = p.afin.m[0] * p.afin.m[3];
            let real = s * if p.tipo == 1 {
                4.0 * p.mx * p.my - (4.0 - std::f32::consts::PI) * p.radio * p.radio
            } else {
                std::f32::consts::PI * p.radio * p.radio * p.ex * p.ey
            };
            for (cual, a) in [("fórmula", area(&exacta)), ("búsqueda", area(&buscada))] {
                assert!((a - real).abs() / real < 0.04, "tipo {}, {cual}: {a} frente a {real}", p.tipo);
            }
            // Y los lados rectos se juntan: una caja no son cien franjas.
            if p.tipo == 1 {
                assert!(exacta.len() < buscada.len() + 4, "{} frente a {}", exacta.len(), buscada.len());
            }
        }
    }
}
