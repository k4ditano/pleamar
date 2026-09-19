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
/// La franja que se le añade a la superficie para la gráfica de frames.
pub const ALTO_INSTRUMENTOS: f32 = 84.0;
const MAX_FORMAS: usize = 4096;
const MAX_ELEMENTOS: usize = 2048;

/// La lista de dibujo convertida en lo que pinta la GPU: formas evaluadas y
/// elementos con su caja. Se recompone cada frame; son unos pocos cientos de
/// números.
#[derive(Default)]
pub struct Dibujo {
    pub formas: Vec<f32>,
    pub elementos: Vec<f32>,
    tam: (f32, f32),
    /// Lo que han medido los textos que lo pidieron: propiedad y valor.
    pub medidas: Vec<(PropId, f32)>,
    /// Grupos que se pintan aparte: qué elementos, y en qué capa.
    pub apartes: Vec<(Range<u32>, usize)>,
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
}

fn unir(a: Option<[f32; 4]>, b: [f32; 4]) -> [f32; 4] {
    a.map_or(b, |a| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
}

impl Dibujo {
    pub fn n_elementos(&self) -> usize {
        self.elementos.len() / POR_ELEMENTO
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

    /// Un elemento solo existe si su caja, recortada, toca la pantalla.
    fn elemento(&mut self, tipo: f32, caja: [f32; 4], recortes: &[(usize, [f32; 4])], rellenar: impl FnOnce(&mut [f32])) {
        let mut c = [caja[0].max(0.0), caja[1].max(0.0), caja[2].min(self.tam.0), caja[3].min(self.tam.1)];
        for (_, r) in recortes {
            c = [c[0].max(r[0]), c[1].max(r[1]), c[2].min(r[2]), c[3].min(r[3])];
        }
        if c[2] <= c[0] || c[3] <= c[1] || self.n_elementos() >= MAX_ELEMENTOS {
            return;
        }
        let k = self.elementos.len();
        self.elementos.resize(k + POR_ELEMENTO, 0.0);
        let e = &mut self.elementos[k..];
        e[0] = tipo;
        e[4..8].copy_from_slice(&c);
        for j in 0..4 {
            e[32 + j] = recortes.get(j).map_or(-1.0, |r| r.0 as f32);
        }
        rellenar(e);
    }

    pub fn componer(&mut self, instrs: &[Instr], c: Ctx, textos: &[String], tip: &mut Textos, tam: (f32, f32), hud: bool) {
        self.medidas.clear();
        self.tam = tam;
        self.formas.clear();
        self.elementos.clear();
        self.apartes.clear();
        let mut recortes: Vec<(usize, [f32; 4])> = Vec::new();
        // Cada entrada es ya el producto de todas las de encima.
        let mut giros: Vec<Afin> = Vec::new();
        let mut opacos: Vec<GrupoOpaco> = Vec::new();
        let mut cuerpo: Option<CuerpoAbierto> = None;
        let aplanar = |f: &Forma, giros: &[Afin]| {
            let mut p = f.aplanar(c);
            p.afin = giros.last().copied().unwrap_or(Afin::IDENTIDAD);
            p
        };
        let color = |col: &Color| [col[0].evaluar(c), col[1].evaluar(c), col[2].evaluar(c)];
        let tip_escala = tip.escala();

        for (sitio, i) in instrs.iter().enumerate() {
            if self.formas.len() / POR_FORMA >= MAX_FORMAS - 8 {
                break;
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
                    cuerpo = Some(CuerpoAbierto { primera: self.formas.len() / POR_FORMA, n: 0, caja: None, holgura: 0.0, sombra: sombra.clone() })
                }
                Instr::Forma { forma, fusion } => {
                    let p = aplanar(forma, &giros);
                    let k = fusion.evaluar(c).max(0.0);
                    let caja = p.caja();
                    self.forma(p, k);
                    if let Some(g) = &mut cuerpo {
                        g.n += 1;
                        g.holgura = g.holgura.max(k * 0.5);
                        if let Some(b) = caja {
                            g.caja = Some(unir(g.caja, b));
                        }
                    }
                }
                Instr::Relleno { pintura, alfa, filo, luz, borde } => {
                    let Some(g) = cuerpo.take() else { continue };
                    let Some(mut caja) = g.caja else { continue };
                    let h = g.holgura + 2.0;
                    caja = [caja[0] - h, caja[1] - h, caja[2] + h, caja[3] + h];
                    if let Some(s) = &g.sombra {
                        let d = s.difusa + 2.0;
                        caja = unir(Some(caja), [caja[0] + s.desplazada.0 - d, caja[1] + s.desplazada.1 - d, caja[2] + s.desplazada.0 + d, caja[3] + s.desplazada.1 + d]);
                    }
                    let a = alfa.evaluar(c).clamp(0.0, 1.0) * veces;
                    self.elemento(0.0, caja, &recortes, |e| {
                        afin.codificar(&mut e[44..52]);
                        e[1] = g.primera as f32;
                        e[2] = g.n as f32;
                        e[3] = a;
                        match pintura {
                            Pintura::Color(col) => e[8..11].copy_from_slice(&color(col)),
                            Pintura::Lineal { de, a, c0, c1 } => {
                                e[8..11].copy_from_slice(&color(c0));
                                e[12..15].copy_from_slice(&color(c1));
                                e[15] = 1.0;
                                e[16..20].copy_from_slice(&[de.0.evaluar(c), de.1.evaluar(c), a.0.evaluar(c), a.1.evaluar(c)]);
                            }
                        }
                        e[11] = *filo;
                        if let Some(l) = luz {
                            e[20..23].copy_from_slice(&[l.cantidad, l.desde_y.evaluar(c), l.alto]);
                        }
                        if let Some((grosor, col)) = borde {
                            e[23] = grosor.evaluar(c).max(0.0);
                            e[24..27].copy_from_slice(&color(col));
                        }
                        if let Some(s) = &g.sombra {
                            e[28..32].copy_from_slice(&[s.desplazada.0, s.desplazada.1, s.difusa, s.alfa]);
                        }
                    });
                }
                Instr::Plano { forma, color: col, alfa } => {
                    let a = alfa.evaluar(c).clamp(0.0, 1.0) * veces;
                    if a <= 0.001 {
                        continue; // lo invisible no ocupa ni un quad
                    }
                    let p = aplanar(forma, &giros);
                    let Some(b) = p.caja() else { continue };
                    let k = self.forma(p, 0.0);
                    let rgb = color(col);
                    self.elemento(0.0, [b[0] - 2.0, b[1] - 2.0, b[2] + 2.0, b[3] + 2.0], &recortes, |e| {
                        afin.codificar(&mut e[44..52]);
                        e[1] = k as f32;
                        e[2] = 1.0;
                        e[3] = a;
                        e[8..11].copy_from_slice(&rgb);
                    });
                }
                Instr::Imagen { imagen, destino, alfa, tinte } => {
                    let a = alfa.evaluar(c).clamp(0.0, 1.0) * veces;
                    let Some(hueco) = tip.imagen(imagen.0 as usize) else { continue };
                    if a <= 0.001 {
                        continue;
                    }
                    let d = [destino.0.evaluar(c), destino.1.evaluar(c), destino.2.evaluar(c), destino.3.evaluar(c)];
                    let rgb = tinte.as_ref().map(&color);
                    self.trozo(d, hueco.uv(), a, rgb, afin, &recortes);
                }
                Instr::Texto { contenido, en, ancla, ancho, estilo, alfa, mide } => {
                    let texto = match contenido {
                        Contenido::Fijo(t) => t.as_str(),
                        Contenido::Vivo(id) => textos.get(id.0 as usize).map_or("", String::as_str),
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
                    let p = aplanar(forma, &giros).encoger(*margen);
                    let caja = p.caja().unwrap_or([0.0; 4]);
                    let k = self.forma(p, 0.0);
                    if recortes.len() < 4 {
                        recortes.push((k, [caja[0] - 1.0, caja[1] - 1.0, caja[2] + 1.0, caja[3] + 1.0]));
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
        if hud {
            // Los instrumentos ocupan la franja de abajo, que se añadió para ellos.
            self.elemento(9.0, [0.0, tam.1 - ALTO_INSTRUMENTOS, tam.0, tam.1], &[], |e| Afin::IDENTIDAD.codificar(&mut e[44..52]));
        }
        // Un almacén vacío no se puede enlazar ni escribir.
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
    /// Milihercios del monitor; 0 si no se sabe.
    pub mhz: i32,
    pub nombre: String,
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
    superficie: wgpu::Surface<'static>,
    ventana: Box<dyn Ventana>,
    px: (u32, u32),
    uniformes: wgpu::Buffer,
    grupo_uniformes: wgpu::BindGroup,
    vistas_de_capa: Vec<wgpu::TextureView>,
    grupo_capas: wgpu::BindGroup,
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
    bufer_elementos: wgpu::Buffer,
    atlas: wgpu::Texture,
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
        .expect("no hay adaptador gráfico");
        let (dispositivo, cola) =
            pollster::block_on(adaptador.request_device(&wgpu::DeviceDescriptor::default())).expect("no hay dispositivo");
        let caps = primera.get_capabilities(&adaptador);
        // Sin sRGB: el compositor mezcla los bytes tal cual, y el alfa
        // premultiplicado solo cuadra si nadie los recodifica por el camino.
        let formato = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let alfa = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else {
            eprintln!("aviso: sin alfa premultiplicado ({:?}); el fondo saldrá opaco", caps.alpha_modes);
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
        let bufer_formas = almacen("formas", MAX_FORMAS * POR_FORMA);
        let bufer_elementos = almacen("elementos", MAX_ELEMENTOS * POR_ELEMENTO);
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
        let grupo_escena = dispositivo.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &tuberia.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: bufer_formas.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: bufer_elementos.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&atlas.create_view(&Default::default())) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&muestreo) },
            ],
        });
        let grupo_sin_capas = Self::capas_de(&dispositivo, &tuberia, formato, 1, 1).1;
        Gpu { adaptador, dispositivo, cola, formato, alfa, sin_bloqueo, tuberia, bufer_formas, bufer_elementos, atlas, grupo_escena, grupo_sin_capas }
    }

    /// Sube al atlas lo que el taller haya pintado desde la última vez.
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
    fn capas_de(dispositivo: &wgpu::Device, tuberia: &wgpu::RenderPipeline, formato: wgpu::TextureFormat, ancho: u32, alto: u32) -> (Vec<wgpu::TextureView>, wgpu::BindGroup) {
        let t = dispositivo.create_texture(&wgpu::TextureDescriptor {
            label: Some("capas"),
            size: wgpu::Extent3d { width: ancho.max(1), height: alto.max(1), depth_or_array_layers: MAX_CAPAS as u32 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: formato,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let todas = t.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let cada = (0..MAX_CAPAS as u32)
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
        let (vistas_de_capa, grupo_capas) = Self::capas_de(&self.dispositivo, &self.tuberia, self.formato, 1, 1);
        let mut l = Lamina {
            id: n.id, nombre: n.nombre, mhz: n.mhz, escala: n.escala, marca_el_ritmo: true,
            superficie: n.superficie, ventana: n.ventana, px: (0, 0), uniformes, grupo_uniformes, vistas_de_capa, grupo_capas,
        };
        self.configurar(&mut l, tam);
        l
    }

    /// Al cambiar de escala, de tamaño o de papel en el ritmo.
    pub fn configurar(&self, l: &mut Lamina, tam: (f32, f32)) {
        let px = ((tam.0 * l.escala).round().max(1.0) as u32, (tam.1 * l.escala).round().max(1.0) as u32);
        self.superficie_configurar(l, px);
        if px != l.px {
            l.px = px;
            (l.vistas_de_capa, l.grupo_capas) = Self::capas_de(&self.dispositivo, &self.tuberia, self.formato, px.0, px.1);
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
                present_mode: if l.marca_el_ritmo { wgpu::PresentMode::Fifo } else { self.sin_bloqueo.unwrap_or(wgpu::PresentMode::Fifo) },
                color_space: wgpu::SurfaceColorSpace::Auto,
            },
        );
    }

    pub fn subir(&self, d: &Dibujo) {
        self.cola.write_buffer(&self.bufer_formas, 0, bytemuck::cast_slice(&d.formas));
        self.cola.write_buffer(&self.bufer_elementos, 0, bytemuck::cast_slice(&d.elementos));
    }

    /// Pinta el dibujo en una lámina. Devuelve si llegó a presentarse.
    pub fn pintar(&self, l: &mut Lamina, d: &Dibujo, uniformes: &[f32]) -> bool {
        let mut u = uniformes.to_vec();
        u[3] = l.escala;
        self.cola.write_buffer(&l.uniformes, 0, bytemuck::cast_slice(&u));
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
            for t in tramos.iter().filter(|t| !t.is_empty()) {
                pase.draw(0..6, t.clone());
            }
        };
        // Primero los grupos con opacidad, cada uno a su capa…
        let mut principal: Vec<Range<u32>> = Vec::new();
        let mut desde = 0u32;
        for (tramo, capa) in &d.apartes {
            pase_a(&mut codificador, &l.vistas_de_capa[*capa], &self.grupo_sin_capas, std::slice::from_ref(tramo));
            principal.push(desde..tramo.start);
            desde = tramo.end;
        }
        principal.push(desde..d.n_elementos() as u32);
        // …y luego todo lo demás, con las capas ya hechas entre medias.
        pase_a(&mut codificador, &vista, &l.grupo_capas, &principal);
        self.cola.submit(Some(codificador.finish()));
        self.cola.present(marco);
        true
    }
}

impl Lamina {
    /// Por dónde entra el ratón. El resto de la superficie, aunque sea suya,
    /// deja pasar el clic a lo de debajo.
    pub fn region_de_entrada(&self, cajas: &[[i32; 4]]) {
        self.ventana.region_de_entrada(cajas);
    }
}

pub const N_UNIFORMES: usize = 8 + 120;
