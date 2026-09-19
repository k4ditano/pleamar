//! El hilo que pinta. Es dueño de los muelles y del reloj: recibe intenciones,
//! las recorre a la cadencia de la pantalla y, cuando todo está quieto, deja
//! de pintar del todo.

use crate::escena::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Opciones {
    pub hud: bool,
    pub ingenuo: bool,
}

struct Azar(u64);
impl Azar {
    fn entre(&mut self, a: f32, b: f32) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        a + (b - a) * ((self.0 >> 40) as f32 / (1u64 << 24) as f32)
    }
}

#[derive(Default)]
struct Ciclo {
    dts: Vec<f32>,
    max_bloqueada: f32,
    frames_bloqueada: u32,
}

impl Ciclo {
    fn cerrar(&mut self) {
        if self.dts.len() < 8 {
            self.dts.clear();
            return;
        }
        let mut o = self.dts.clone();
        o.sort_by(|a, b| a.total_cmp(b));
        let media = o.iter().sum::<f32>() / o.len() as f32;
        let p99 = o[((o.len() as f32 * 0.99) as usize).min(o.len() - 1)];
        println!(
            "ciclo · {:>4} frames · media {:>5.2} ms · p99 {:>6.2} ms · máx {:>6.2} ms · con la lógica bloqueada: {} frames, máx {:.2} ms",
            o.len(), media, p99, o[o.len() - 1], self.frames_bloqueada, self.max_bloqueada
        );
        *self = Ciclo::default();
    }
}

pub fn hilo(
    instancia: wgpu::Instance,
    superficie: wgpu::Surface<'static>,
    rx: Receiver<ARender>,
    a_logica: Sender<Evento>,
    logica_bloqueada: Arc<AtomicBool>,
    op: Opciones,
) {
    let adaptador = pollster::block_on(instancia.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: Some(&superficie),
        power_preference: wgpu::PowerPreference::LowPower,
        ..Default::default()
    }))
    .expect("no hay adaptador gráfico");
    let (dispositivo, cola) =
        pollster::block_on(adaptador.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("no hay dispositivo");

    let caps = superficie.get_capabilities(&adaptador);
    // Sin sRGB: el compositor mezcla los bytes tal cual, y el alfa
    // premultiplicado solo cuadra si nadie los recodifica por el camino.
    let formato = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
    let alfa = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
        wgpu::CompositeAlphaMode::PreMultiplied
    } else {
        eprintln!("aviso: sin alfa premultiplicado ({:?}); el fondo saldrá opaco", caps.alpha_modes);
        wgpu::CompositeAlphaMode::Auto
    };
    let info = adaptador.get_info();
    println!("render · {} ({:?}) · {:?} · {:?}", info.name, info.backend, formato, alfa);

    superficie.configure(
        &dispositivo,
        &wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: formato,
            view_formats: vec![],
            alpha_mode: alfa,
            width: ANCHO,
            height: ALTO,
            desired_maximum_frame_latency: 1,
            present_mode: wgpu::PresentMode::Fifo,
            color_space: wgpu::SurfaceColorSpace::Auto,
        },
    );

    let mut uniformes = [0f32; 8 + 120];
    let bufer = dispositivo.create_buffer(&wgpu::BufferDescriptor {
        label: Some("uniformes"),
        size: std::mem::size_of_val(&uniformes) as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut lista = vec![0f32; MAX_INSTR * POR_INSTR];
    let bufer_lista = dispositivo.create_buffer(&wgpu::BufferDescriptor {
        label: Some("lista de dibujo"),
        size: (lista.len() * 4) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
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
        label: Some("intérprete"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &modulo,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &modulo,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: formato,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    // El atlas cambia con la escena, y con él el grupo de enlaces.
    let enlazar = |atlas: &Lienzo| {
        let tam = wgpu::Extent3d { width: atlas.ancho as u32, height: atlas.alto as u32, depth_or_array_layers: 1 };
        let textura = dispositivo.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas"),
            size: tam,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        cola.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &textura, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &atlas.rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * tam.width), rows_per_image: None },
            tam,
        );
        let vista = textura.create_view(&Default::default());
        dispositivo.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &tuberia.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: bufer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: bufer_lista.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&vista) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&muestreo) },
            ],
        })
    };
    let mut grupo = enlazar(&Lienzo { ancho: 1, alto: 1, rgba: vec![0; 4] });

    // ── estado ───────────────────────────────────────────────────
    let mut escena = Escena::default();
    let mut props: Vec<Animada> = Vec::new();
    let mut dentro: Vec<bool> = Vec::new();
    let mut parpadeos: Vec<(Instant, Option<Instant>)> = Vec::new();
    let mut pendientes: Vec<(Instant, Transicion)> = Vec::new();
    let mut puntero: Option<(f32, f32)> = None;
    let mut azar = Azar(0x9E3779B97F4A7C15);
    let inicio = Instant::now();

    let mut historial = [0f32; 120];
    let mut ciclo = Ciclo::default();
    let mut ultimo = Instant::now();
    let mut en_reposo = false;
    let mut periodo_ms = 16.7f32;
    let mut frames_para_periodo = 0u32;

    loop {
        // ── 1. lo que ha llegado ────────────────────────────────
        let mut bloquear = None;
        let mut pulsado = false;
        let mut entrada: Vec<ARender> = Vec::new();
        if en_reposo {
            // Quieta: ni un frame. Solo la despiertan un mensaje, un retraso
            // que vence o el próximo parpadeo.
            let mut hasta = Instant::now() + Duration::from_secs(3600);
            for (cuando, _) in &pendientes {
                hasta = hasta.min(*cuando);
            }
            for (proximo, _) in &parpadeos {
                hasta = hasta.min(*proximo);
            }
            match rx.recv_timeout(hasta.saturating_duration_since(Instant::now())) {
                Ok(m) => entrada.push(m),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            en_reposo = false;
            ultimo = Instant::now() - Duration::from_secs_f32(periodo_ms / 1000.0);
        }
        entrada.extend(rx.try_iter());
        for m in entrada {
            match m {
                ARender::Escena(nueva) => {
                    // Las propiedades que se llaman igual sobreviven al cambio.
                    let viejas: Vec<(&str, Animada)> =
                        escena.props.iter().map(|p| p.0).zip(props.iter().copied()).collect();
                    props = nueva
                        .props
                        .iter()
                        .map(|(nombre, inicial, muelle)| {
                            viejas.iter().find(|(n, _)| n == nombre).map(|(_, a)| *a).unwrap_or(Animada::en(*inicial, *muelle))
                        })
                        .collect();
                    dentro = vec![false; nueva.zonas.len()];
                    parpadeos = nueva
                        .comportamientos
                        .iter()
                        .filter_map(|c| match c {
                            Comportamiento::Parpadeo { cada, .. } => {
                                Some((Instant::now() + Duration::from_secs_f32(azar.entre(cada.0, cada.1) * 0.6), None))
                            }
                            _ => None,
                        })
                        .collect();
                    pendientes.clear();
                    if let Some(atlas) = &nueva.atlas {
                        grupo = enlazar(atlas);
                    }
                    assert!(nueva.instrs.len() <= MAX_INSTR, "la escena no cabe en la lista de dibujo");
                    println!(
                        "render · escena: {} propiedades, {} instrucciones, {} comportamientos, {} zonas",
                        nueva.props.len(), nueva.instrs.len(), nueva.comportamientos.len(), nueva.zonas.len()
                    );
                    escena = nueva;
                }
                ARender::Orden(Orden::Animar(t)) => pendientes.push((Instant::now() + t.retraso, t)),
                ARender::Orden(Orden::Impulso { prop, velocidad }) => props[prop.0 as usize].v += velocidad,
                ARender::Orden(Orden::Bloquear(d)) => bloquear = Some(d),
                ARender::Puntero(p) => puntero = p,
                ARender::Pulsar => pulsado = true,
                ARender::Salir => {
                    ciclo.cerrar();
                    return;
                }
            }
        }
        if let Some(d) = bloquear {
            // Modo ingenuo: el trabajo de la lógica ocurre aquí, en el hilo
            // que pinta. Es lo que pasa en QtQuick con un handler pesado.
            if op.ingenuo {
                logica_bloqueada.store(true, Ordering::Relaxed);
                std::thread::sleep(d);
                logica_bloqueada.store(false, Ordering::Relaxed);
            }
        }

        // ── 2. avanzar el tiempo ────────────────────────────────
        let ahora = Instant::now();
        let dt = (ahora - ultimo).as_secs_f32();
        ultimo = ahora;
        let t_total = (ahora - inicio).as_secs_f32();

        // Zonas: quién tiene el ratón encima. Lo declarado se ejecuta aquí
        // mismo; a la lógica solo se le cuenta.
        for (z, estaba) in escena.zonas.iter().zip(dentro.iter_mut()) {
            let esta = z.activa.evaluar(&props) > 0.5
                && puntero.is_some_and(|(x, y)| z.forma.distancia(&props, x, y) < 0.0);
            if esta != *estaba {
                *estaba = esta;
                for t in if esta { &z.al_entrar } else { &z.al_salir } {
                    pendientes.push((ahora + t.retraso, t.clone()));
                }
                let _ = a_logica.send(if esta { Evento::Entra(z.id) } else { Evento::Sale(z.id) });
            }
        }
        if pulsado {
            // La de más arriba es la última declarada.
            if let Some((z, _)) = escena.zonas.iter().zip(&dentro).rev().find(|(_, d)| **d) {
                let _ = a_logica.send(Evento::Pulsa(z.id));
            }
        }

        pendientes.retain(|(cuando, t)| {
            if *cuando <= ahora {
                let a = &mut props[t.prop.0 as usize];
                a.objetivo = t.a;
                a.muelle = t.muelle;
                false
            } else {
                true
            }
        });

        // Comportamientos: la vida propia de la escena.
        let mut vivo = false;
        let mut n_parpadeo = 0;
        for c in &escena.comportamientos {
            match c {
                Comportamiento::Parpadeo { prop, cada, dura } => {
                    let (proximo, desde) = &mut parpadeos[n_parpadeo];
                    n_parpadeo += 1;
                    if desde.is_none() && ahora >= *proximo {
                        *desde = Some(ahora);
                    }
                    if let Some(d) = *desde {
                        let t = (ahora - d).as_secs_f32() / dura;
                        if t >= 1.0 {
                            *desde = None;
                            *proximo = ahora + Duration::from_secs_f32(azar.entre(cada.0, cada.1));
                            props[prop.0 as usize].fijar(1.0);
                        } else {
                            props[prop.0 as usize].fijar((2.0 * t - 1.0).abs().powf(1.6));
                            vivo = true;
                        }
                    }
                }
                Comportamiento::Onda { prop, frecuencia, amplitud } => {
                    let a = amplitud.evaluar(&props);
                    props[prop.0 as usize].fijar(a * (t_total * frecuencia).sin());
                    vivo |= a.abs() > 0.01;
                }
                Comportamiento::Mirada { x, y, centro, alcance, distancia, reposo } => {
                    let (mx, my) = match puntero {
                        Some((px, py)) => {
                            let (dx, dy) = (px - centro.0.evaluar(&props), py - centro.1.evaluar(&props));
                            let lejos = dx.hypot(dy).max(1.0);
                            let tira = (lejos / distancia).min(1.0);
                            (dx / lejos * alcance.0 * tira, dy / lejos * alcance.1 * tira)
                        }
                        None => (reposo.0.evaluar(&props), reposo.1.evaluar(&props)),
                    };
                    props[x.0 as usize].objetivo = mx;
                    props[y.0 as usize].objetivo = my;
                }
            }
        }
        for a in &mut props {
            a.avanzar(dt);
        }

        // ── 3. pintar ───────────────────────────────────────────
        let bloqueada = logica_bloqueada.load(Ordering::Relaxed);
        for (k, i) in escena.instrs.iter().enumerate() {
            codificar(i, &props, &mut lista[k * POR_INSTR..(k + 1) * POR_INSTR]);
        }
        let n = escena.instrs.len();
        cola.write_buffer(&bufer_lista, 0, bytemuck::cast_slice(&lista[..n.max(1) * POR_INSTR]));
        let cabecera = [
            ANCHO as f32, ALTO as f32, t_total, n as f32,
            if op.hud { 1.0 } else { 0.0 }, periodo_ms, if bloqueada { 1.0 } else { 0.0 }, 0.0,
        ];
        uniformes[..8].copy_from_slice(&cabecera);
        uniformes[8..].copy_from_slice(&historial);
        cola.write_buffer(&bufer, 0, bytemuck::cast_slice(&uniformes));

        let marco = match superficie.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            otro => {
                eprintln!("render · sin textura este frame: {otro:?}");
                std::thread::sleep(Duration::from_millis(8));
                continue;
            }
        };
        let vista = marco.texture.create_view(&Default::default());
        let mut codificador = dispositivo.create_command_encoder(&Default::default());
        {
            let mut pase = codificador.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &vista,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pase.set_pipeline(&tuberia);
            pase.set_bind_group(0, &grupo, &[]);
            pase.draw(0..3, 0..1);
        }
        cola.submit(Some(codificador.finish()));
        cola.present(marco);

        // ── 4. medir ────────────────────────────────────────────
        let ms = dt * 1000.0;
        historial.copy_within(1.., 0);
        historial[119] = if bloqueada { -ms } else { ms };
        ciclo.dts.push(ms);
        if bloqueada {
            ciclo.frames_bloqueada += 1;
            ciclo.max_bloqueada = ciclo.max_bloqueada.max(ms);
        }
        // El periodo de la pantalla, aprendido de los primeros frames buenos.
        if frames_para_periodo < 40 && ms > 3.0 && ms < 40.0 {
            frames_para_periodo += 1;
            if frames_para_periodo == 40 {
                let mut o: Vec<f32> = ciclo.dts.iter().copied().filter(|m| *m > 3.0 && *m < 40.0).collect();
                o.sort_by(|a, b| a.total_cmp(b));
                if !o.is_empty() {
                    periodo_ms = o[o.len() / 2];
                }
            }
        }

        // ── 5. ¿queda algo moviéndose? ──────────────────────────
        if !vivo && !bloqueada && props.iter().all(Animada::quieta) {
            for a in &mut props {
                a.posar();
            }
            ciclo.cerrar();
            en_reposo = true;
        }
    }
}

const POR_INSTR: usize = 20;

/// Una instrucción, con sus expresiones ya evaluadas, en los veinte números
/// que lee el shader.
fn codificar(i: &Instr, props: &[Animada], s: &mut [f32]) {
    s.fill(0.0);
    let forma = |f: &Forma, s: &mut [f32]| match f {
        Forma::Elipse { centro, radio, escala } => {
            s[1] = 0.0;
            s[4] = centro.0.evaluar(props);
            s[5] = centro.1.evaluar(props);
            s[8] = radio.evaluar(props);
            s[9] = escala.0.evaluar(props);
            s[10] = escala.1.evaluar(props);
        }
        Forma::Caja { centro, mitad, radio } => {
            s[1] = 1.0;
            s[4] = centro.0.evaluar(props);
            s[5] = centro.1.evaluar(props);
            s[6] = mitad.0.evaluar(props).max(0.0);
            s[7] = mitad.1.evaluar(props).max(0.0);
            s[8] = radio.evaluar(props).min(s[6]).min(s[7]).max(0.0);
        }
    };
    match i {
        Instr::Grupo { sombra } => {
            s[0] = 0.0;
            if let Some(so) = sombra {
                s[16..20].copy_from_slice(&[so.desplazada.0, so.desplazada.1, so.difusa, so.alfa]);
            }
        }
        Instr::Forma { forma: f, fusion } => {
            s[0] = 1.0;
            forma(f, s);
            s[2] = fusion.evaluar(props).max(0.0);
        }
        Instr::Relleno { color, alfa, filo, luz } => {
            s[0] = 2.0;
            s[3] = *filo;
            s[11] = alfa.evaluar(props).clamp(0.0, 1.0);
            for k in 0..3 {
                s[12 + k] = color[k].evaluar(props);
            }
            if let Some(l) = luz {
                s[15] = l.cantidad;
                s[16] = l.desde_y.evaluar(props);
                s[17] = l.alto;
            }
        }
        Instr::Recorte(r) => {
            s[0] = 3.0;
            match r {
                Some((f, margen)) => {
                    forma(f, s);
                    s[3] = *margen;
                }
                None => s[1] = 2.0,
            }
        }
        Instr::Plano { forma: f, color, alfa } => {
            s[0] = 4.0;
            forma(f, s);
            s[11] = alfa.evaluar(props).clamp(0.0, 1.0);
            for k in 0..3 {
                s[12 + k] = color[k].evaluar(props);
            }
        }
        Instr::Textura { destino, uv, alfa } => {
            s[0] = 5.0;
            s[4] = destino.0.evaluar(props);
            s[5] = destino.1.evaluar(props);
            s[6] = destino.2.evaluar(props);
            s[7] = destino.3.evaluar(props);
            s[11] = alfa.evaluar(props).clamp(0.0, 1.0);
            s[16..20].copy_from_slice(uv);
        }
    }
}
