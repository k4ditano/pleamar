//! La lente: ver lo que hay detrás del cristal para poder doblarlo.
//!
//! Un cristal de verdad no deja ver lo de detrás tal cual: cerca del borde lo
//! dobla, como una gota. Para eso hacen falta los píxeles de detrás, y en Wayland
//! solo los tiene el compositor. Se le pide una foto de la pantalla en el sitio
//! de la superficie —con ella encima, que es lo único que sabe dar— y se despeja:
//! foto = lo nuestro + (1 − nuestro alfa) · fondo, y lo nuestro lo sabemos porque
//! lo pintamos en un lienzo antes de presentarlo. Medido: el fondo despejado se
//! aparta del de verdad menos de medio nivel de 255 con un tinte al 30 %, y
//! menos de dos con un canto al 85 %.
//!
//! Con el fondo ya limpio se desenfoca un poco —el esmerilado del cristal— y el
//! shader de las formas lo lee desplazado según el bisel. Así la lente la pinta
//! pleamar, sin que el compositor sepa nada de cristales.

use crate::plataforma::Detras;

/// Cuánto se esmerila lo de detrás, en píxeles lógicos.
const ESMERILADO: f32 = 8.0;

pub struct Tuberias {
    despejar: wgpu::RenderPipeline,
    borrar: wgpu::RenderPipeline,
    lineal: wgpu::Sampler,
}

impl Tuberias {
    pub fn nuevas(d: &wgpu::Device) -> Tuberias {
        let modulo = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("detras"), source: wgpu::ShaderSource::Wgsl(include_str!("detras.wgsl").into()) });
        let tuberia = |fs: &str| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: None,
                vertex: wgpu::VertexState { module: &modulo, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &modulo,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba8Unorm, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let lineal = d.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        Tuberias { despejar: tuberia("despejar"), borrar: tuberia("borrar"), lineal }
    }
}

fn textura(d: &wgpu::Device, etiqueta: &str, (w, h): (u32, u32), formato: wgpu::TextureFormat, uso: wgpu::TextureUsages) -> (wgpu::Texture, wgpu::TextureView) {
    let t = d.create_texture(&wgpu::TextureDescriptor {
        label: Some(etiqueta),
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: formato,
        usage: uso,
        view_formats: &[],
    });
    let v = t.create_view(&Default::default());
    (t, v)
}

/// Lo de una superficie con cristal: dónde se pinta cada frame antes de
/// presentarlo, la última foto, y el fondo despejado —nítido y esmerilado—.
pub struct Lente {
    pub px: (u32, u32),
    /// Dos lienzos: en uno se pinta, el otro es el que el compositor tiene.
    lienzos: [(wgpu::Texture, wgpu::TextureView); 2],
    siguiente: usize,
    presentado: Option<usize>,
    foto: Option<(wgpu::Texture, wgpu::TextureView, (u32, u32))>,
    /// El fondo despejado, en dos para ir y volver: cada vez se escribe en el
    /// que no es el actual, leyendo el actual donde no se puede despejar.
    fondos: [(wgpu::Texture, wgpu::TextureView); 2],
    actual: usize,
    medio: (wgpu::Texture, wgpu::TextureView),
    borroso: (wgpu::Texture, wgpu::TextureView),
    uniformes_borrar: [wgpu::Buffer; 2],
    /// Una muestra de la última foto, y de qué caja era: para no repetir el
    /// trabajo si nada ha cambiado.
    muestra: Vec<u8>,
    pub caja: [i32; 4],
    uniformes_caja: wgpu::Buffer,
    /// Una foto recibida cuyo fondo aún está por despejar: qué lienzo se
    /// presentó y en qué recorte.
    pendiente: Option<(usize, (u32, u32, u32, u32))>,
    /// Ya hay un fondo que enseñar.
    pub listo: bool,
    /// Lo que lee el shader de las formas: el fondo nítido y el esmerilado.
    pub grupo: Option<wgpu::BindGroup>,
}

impl Lente {
    pub fn nueva(d: &wgpu::Device, formato: wgpu::TextureFormat, px: (u32, u32)) -> Lente {
        use wgpu::TextureUsages as U;
        let lienzo = || textura(d, "lienzo", px, formato, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC);
        let fondo = || textura(d, "fondo", px, wgpu::TextureFormat::Rgba8Unorm, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC | U::COPY_DST);
        let mitad = (px.0.div_ceil(2), px.1.div_ceil(2));
        let uniformes = || d.create_buffer(&wgpu::BufferDescriptor { label: Some("borrar"), size: 32, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        Lente {
            px,
            lienzos: [lienzo(), lienzo()],
            siguiente: 0,
            presentado: None,
            foto: None,
            fondos: [fondo(), fondo()],
            actual: 0,
            medio: textura(d, "medio", mitad, wgpu::TextureFormat::Rgba8Unorm, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING),
            borroso: textura(d, "borroso", mitad, wgpu::TextureFormat::Rgba8Unorm, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING),
            uniformes_borrar: [uniformes(), uniformes()],
            muestra: Vec::new(),
            pendiente: None,
            caja: [0; 4],
            uniformes_caja: d.create_buffer(&wgpu::BufferDescriptor { label: Some("caja"), size: 32, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
            listo: false,
            grupo: None,
        }
    }

    /// Dónde se pinta este frame.
    pub fn lienzo(&self) -> &wgpu::TextureView {
        &self.lienzos[self.siguiente].1
    }

    /// Lo pintado pasa a la pantalla: se copia, y queda como el presentado.
    pub fn copiar_a(&mut self, codificador: &mut wgpu::CommandEncoder, pantalla: &wgpu::Texture) {
        let (w, h) = self.px;
        codificador.copy_texture_to_texture(
            self.lienzos[self.siguiente].0.as_image_copy(),
            pantalla.as_image_copy(),
            wgpu::Extent3d { width: w.min(pantalla.width()), height: h.min(pantalla.height()), depth_or_array_layers: 1 },
        );
        self.presentado = Some(self.siguiente);
        self.siguiente ^= 1;
    }

    /// Llega una foto de la caja `caja` (píxeles lógicos de la superficie): se
    /// despeja el fondo y se esmerila, solo ahí. `true` si ha cambiado algo que
    /// se vea, y hay que volver a pintar.
    pub fn recibir(&mut self, g: &crate::gpu::Gpu, d: Detras, escala: f32, caja: [i32; 4]) -> bool {
        let Some(presentado) = self.presentado else { return false };
        // Una foto sin datos es una que falló.
        let Some(datos) = d.datos else { return false };
        let datos: &[u8] = (*datos).as_ref();
        // Igual que la anterior —o casi: el redondeo de ir y volver da algún
        // nivel suelto—, y no hay nada nuevo detrás. Sin esto, pintar la lente
        // cambia la foto, que cambia la lente, y así sin parar. Se mira un
        // byte de cada trece, que para saber si algo se ha movido basta.
        let muestra: Vec<u8> = datos.iter().copied().step_by(13).collect();
        if self.listo && caja == self.caja && muestra.len() == self.muestra.len() && muestra.iter().zip(&self.muestra).all(|(a, b)| a.abs_diff(*b) <= 3) {
            return false;
        }
        self.muestra = muestra;
        self.caja = caja;
        let dev = &g.dispositivo;
        let tam = (d.ancho, d.alto);
        if self.foto.as_ref().is_none_or(|f| f.2 != tam) {
            let (tx, v) = textura(dev, "foto", tam, wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
            self.foto = Some((tx, v, tam));
        }
        let foto = self.foto.as_ref().unwrap();
        g.cola.write_texture(
            foto.0.as_image_copy(),
            datos,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(d.zancada), rows_per_image: Some(d.alto) },
            wgpu::Extent3d { width: d.ancho, height: d.alto, depth_or_array_layers: 1 },
        );
        // La caja en píxeles de verdad, dentro del lienzo.
        let (lw, lh) = self.px;
        let x0 = ((caja[0] as f32 * escala).floor() as u32).min(lw);
        let y0 = ((caja[1] as f32 * escala).floor() as u32).min(lh);
        let x1 = (((caja[0] + caja[2]) as f32 * escala).ceil() as u32).min(lw);
        let y1 = (((caja[1] + caja[3]) as f32 * escala).ceil() as u32).min(lh);
        if x1 <= x0 || y1 <= y0 {
            return false;
        }
        let tijera = (x0, y0, x1 - x0, y1 - y0);
        g.cola.write_buffer(&self.uniformes_caja, 0, bytemuck::cast_slice(&[caja[0] as f32 * escala, caja[1] as f32 * escala, caja[2] as f32 * escala, caja[3] as f32 * escala, d.opacidad, 0.0, 0.0, 0.0]));
        // Los pases van en el mismo encargo que el frame que se pinta ahora
        // —hay que volver a pintar—: uno aparte costaba un `submit` más, un
        // tercio de milisegundo por foto.
        self.pendiente = Some((presentado, tijera));
        let nuevo = self.actual ^ 1;
        self.grupo = Some(g.grupo_de_detras(&self.fondos[nuevo].1, &self.borroso.1));
        self.listo = true;
        true
    }

    /// Despeja y esmerila lo que dejó la última foto, en el encargo del frame,
    /// antes de pintarlo.
    pub fn preparar(&mut self, g: &crate::gpu::Gpu, t: &Tuberias, cod: &mut wgpu::CommandEncoder, escala: f32) {
        let Some((presentado, tijera)) = self.pendiente.take() else { return };
        let (Some(foto), (lw, lh)) = (self.foto.as_ref(), self.px) else { return };
        let dev = &g.dispositivo;
        // Lo de fuera de la caja se queda como estaba: el fondo nuevo empieza
        // siendo una copia del actual, y solo se escribe dentro.
        let nuevo = self.actual ^ 1;
        cod.copy_texture_to_texture(self.fondos[self.actual].0.as_image_copy(), self.fondos[nuevo].0.as_image_copy(), wgpu::Extent3d { width: lw, height: lh, depth_or_array_layers: 1 });
        let pase = |cod: &mut wgpu::CommandEncoder, tuberia: &wgpu::RenderPipeline, destino: &wgpu::TextureView, grupo: &wgpu::BindGroup, tijera: (u32, u32, u32, u32)| {
            let mut p = cod.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: destino,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            p.set_pipeline(tuberia);
            p.set_bind_group(0, grupo, &[]);
            p.set_scissor_rect(tijera.0, tijera.1, tijera.2, tijera.3);
            p.draw(0..3, 0..1);
        };
        let grupo_despejar = dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &t.despejar.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&foto.1) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.lienzos[presentado].1) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&self.fondos[self.actual].1) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&t.lineal) },
                wgpu::BindGroupEntry { binding: 4, resource: self.uniformes_caja.as_entire_binding() },
            ],
        });
        pase(cod, &t.despejar, &self.fondos[nuevo].1, &grupo_despejar, tijera);
        // Esmerilar, a media resolución: horizontal a `medio`, vertical a `borroso`.
        let (mw, mh) = (self.medio.0.width(), self.medio.0.height());
        let sigma = ESMERILADO * escala * 0.5;
        for (k, paso) in [[1.0f32, 0.0], [0.0, 1.0]].iter().enumerate() {
            g.cola.write_buffer(&self.uniformes_borrar[k], 0, bytemuck::cast_slice(&[paso[0], paso[1], sigma, 0.0, mw as f32, mh as f32, 0.0, 0.0]));
        }
        let borrar = |origen: &wgpu::TextureView, u: &wgpu::Buffer| {
            dev.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &t.borrar.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(origen) },
                    wgpu::BindGroupEntry { binding: 1, resource: u.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&t.lineal) },
                ],
            })
        };
        // El recorte, a la mitad, y con el margen que alcanza la gaussiana.
        let m = (sigma * 2.5).ceil() as u32 + 1;
        let x0 = (tijera.0 / 2).saturating_sub(m);
        let y0 = (tijera.1 / 2).saturating_sub(m);
        let x1 = ((tijera.0 + tijera.2).div_ceil(2) + m).min(mw);
        let y1 = ((tijera.1 + tijera.3).div_ceil(2) + m).min(mh);
        let medio_recorte = (x0, y0, x1.saturating_sub(x0).max(1), y1.saturating_sub(y0).max(1));
        pase(cod, &t.borrar, &self.medio.1, &borrar(&self.fondos[nuevo].1, &self.uniformes_borrar[0]), medio_recorte);
        pase(cod, &t.borrar, &self.borroso.1, &borrar(&self.medio.1, &self.uniformes_borrar[1]), medio_recorte);
        self.actual = nuevo;
    }
}

/// Como mucho, una foto de lo de detrás cada tanto mientras se pinta.
pub const ENTRE_FOTOS: std::time::Duration = std::time::Duration::from_millis(50);

/// Cuánto margen necesita el esmerilado alrededor de un cristal, en píxeles lógicos.
pub const MARGEN: f32 = ESMERILADO * 2.5 + 2.0;
