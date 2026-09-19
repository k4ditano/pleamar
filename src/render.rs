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
    /// Movimiento reducido: los muelles se posan y los gestos enseñan su cara quieta.
    pub reducido: bool,
    /// Sin esperar a la pantalla y sin reposo: para medir lo que cuesta pintar.
    pub sin_vsync: bool,
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
            present_mode: if op.sin_vsync {
                *caps.present_modes.iter().find(|m| matches!(m, wgpu::PresentMode::Immediate | wgpu::PresentMode::Mailbox)).unwrap_or(&wgpu::PresentMode::Fifo)
            } else {
                wgpu::PresentMode::Fifo
            },
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
    let mut dibujo = Dibujo::default();
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
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    // Las capas donde se pintan aparte los grupos con opacidad. Mientras se
    // pinta EN una no se puede leer de ellas, así que ese rato se enlaza una
    // de mentira.
    let capas_de = |ancho: u32, alto: u32| {
        let t = dispositivo.create_texture(&wgpu::TextureDescriptor {
            label: Some("capas"),
            size: wgpu::Extent3d { width: ancho, height: alto, depth_or_array_layers: MAX_CAPAS as u32 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: formato,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let todas = t.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let cada: Vec<wgpu::TextureView> = (0..MAX_CAPAS as u32)
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
    };
    let (vistas_de_capa, grupo_capas) = capas_de(ANCHO, ALTO);
    let (_, grupo_sin_capas) = capas_de(1, 1);

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
                wgpu::BindGroupEntry { binding: 1, resource: bufer_formas.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: bufer_elementos.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&vista) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&muestreo) },
            ],
        })
    };
    let mut grupo = enlazar(&Lienzo { ancho: 1, alto: 1, rgba: vec![0; 4] });

    // ── estado ───────────────────────────────────────────────────
    let mut escena = Escena::default();
    let mut props: Vec<Animada> = Vec::new();
    let mut hechos: Vec<f32> = Vec::new();
    let mut dentro: Vec<bool> = Vec::new();
    let mut parpadeos: Vec<(Instant, Option<Instant>)> = Vec::new();
    let mut pendientes: Vec<(Instant, Transicion)> = Vec::new();
    let mut capas: Vec<EstadoCapa> = Vec::new();
    let mut reglas: Vec<EstadoRegla> = Vec::new();
    let mut gesto: Option<Reproduccion> = None;
    // Lo que emite un fotograma se atiende en el frame siguiente.
    let mut sucesos_tardios: Vec<SucesoId> = Vec::new();
    let mut puntero: Option<(f32, f32)> = None;
    let mut ultima_actividad = Instant::now();
    let mut azar = Azar(0x9E3779B97F4A7C15);
    let inicio = Instant::now();

    let mut historial = [0f32; 120];
    let mut ciclo = Ciclo::default();
    let mut ultimo = Instant::now();
    let mut en_reposo = false;
    let mut proxima_cita: Option<Instant> = None;
    let mut periodo_ms = 16.7f32;
    let mut frames_para_periodo = 0u32;

    loop {
        // ── 1. lo que ha llegado ────────────────────────────────
        let mut bloquear = None;
        let mut pulsado = false;
        // (cuál, si viene de la lógica)
        let mut sucesos: Vec<(usize, bool)> = sucesos_tardios.drain(..).map(|s| (s.0 as usize, false)).collect();
        let mut gestos_pedidos: Vec<usize> = Vec::new();
        let mut entrada: Vec<ARender> = Vec::new();
        if en_reposo {
            // Quieta: ni un frame. Solo la despiertan un mensaje o la próxima
            // cita: un retraso que vence, un parpadeo, una reclamación que caduca.
            let hasta = proxima_cita.unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
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
                    hechos = nueva.hechos.iter().map(|h| h.1).collect();
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
                    capas = nueva.capas.iter().map(|c| EstadoCapa::nueva(c.reclamaciones.len())).collect();
                    reglas = nueva
                        .reglas
                        .iter()
                        .map(|r| {
                            let mut e = EstadoRegla::default();
                            if let Disparador::Cada { entre, .. } = &r.cuando {
                                e.proxima = Some(Instant::now() + Duration::from_secs_f32(azar.entre(entre.0, entre.1)));
                            }
                            e
                        })
                        .collect();
                    gesto = None;
                    pendientes.clear();
                    if let Some(atlas) = &nueva.atlas {
                        grupo = enlazar(atlas);
                    }
                    println!(
                        "render · escena: {} propiedades, {} instrucciones, {} hechos, {} capas, {} gestos, {} reglas, {} zonas",
                        nueva.props.len(), nueva.instrs.len(), nueva.hechos.len(), nueva.capas.len(),
                        nueva.gestos.len(), nueva.reglas.len(), nueva.zonas.len()
                    );
                    escena = nueva;
                }
                ARender::Orden(Orden::Animar(t)) => pendientes.push((Instant::now() + t.retraso, t)),
                ARender::Orden(Orden::Impulso { prop, velocidad }) => props[prop.0 as usize].v += velocidad,
                ARender::Orden(Orden::Bloquear(d)) => bloquear = Some(d),
                ARender::Hecho(nombre, v) => match escena.hechos.iter().position(|h| h.0 == nombre) {
                    Some(i) => hechos[i] = v,
                    None => eprintln!("render · no conozco el hecho «{nombre}»"),
                },
                ARender::Suceso(nombre) => match escena.sucesos.iter().position(|s| s.0 == nombre) {
                    Some(i) => sucesos.push((i, true)),
                    None => eprintln!("render · no conozco el suceso «{nombre}»"),
                },
                ARender::Gesto(nombre) => match escena.gestos.iter().position(|g| g.nombre == nombre) {
                    Some(i) => gestos_pedidos.push(i),
                    None => eprintln!("render · no conozco el gesto «{nombre}»"),
                },
                ARender::Puntero(p) => {
                    puntero = p;
                    ultima_actividad = Instant::now();
                }
                ARender::Pulsar => {
                    pulsado = true;
                    ultima_actividad = Instant::now();
                }
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
        let mut efectos: Vec<Efecto> = Vec::new();
        let mut citas: Vec<Instant> = Vec::new();

        // Zonas: quién tiene el ratón encima.
        let mut bordes: Vec<(bool, usize)> = Vec::new(); // (entra, zona)
        {
            let c = Ctx { props: &props, hechos: &hechos };
            for (k, (z, estaba)) in escena.zonas.iter().zip(dentro.iter_mut()).enumerate() {
                let esta = z.activa.es_verdad(c) && puntero.is_some_and(|(x, y)| z.contiene(c, x, y));
                if esta != *estaba {
                    *estaba = esta;
                    bordes.push((esta, k));
                    let _ = a_logica.send(if esta { Evento::Entra(z.id) } else { Evento::Sale(z.id) });
                }
            }
        }
        // La de más arriba es la última declarada.
        let pulsada = if pulsado { dentro.iter().rposition(|d| *d) } else { None };
        if let Some(k) = pulsada {
            let _ = a_logica.send(Evento::Pulsa(escena.zonas[k].id));
        }

        // Reglas: todo esto ocurre aquí, esté la lógica como esté.
        {
            let c = Ctx { props: &props, hechos: &hechos };
            for (r, e) in escena.reglas.iter().zip(reglas.iter_mut()) {
                let dispara = match &r.cuando {
                    Disparador::Entra(z) => bordes.contains(&(true, z.0 as usize)),
                    Disparador::Sale(z) => bordes.contains(&(false, z.0 as usize)),
                    Disparador::Pulsa(z) => pulsada == Some(z.0 as usize),
                    Disparador::Encima { zona, durante } => {
                        e.esperar(dentro[zona.0 as usize], *durante, ahora, &mut citas)
                    }
                    Disparador::Fuera { zona, durante } => {
                        let esta = dentro[zona.0 as usize];
                        e.armada |= esta;
                        let ya = e.esperar(e.armada && !esta, *durante, ahora, &mut citas);
                        if ya {
                            e.armada = false;
                        }
                        ya
                    }
                    Disparador::Quieto { durante, mientras } => {
                        let falta = (ultima_actividad + *durante).saturating_duration_since(ahora);
                        let listo = mientras.es_verdad(c) && falta.is_zero();
                        if mientras.es_verdad(c) && !falta.is_zero() {
                            citas.push(ultima_actividad + *durante);
                        }
                        let ya = listo && e.ultima_vez != Some(ultima_actividad);
                        if ya {
                            e.ultima_vez = Some(ultima_actividad);
                        }
                        ya
                    }
                    Disparador::Cada { entre, mientras } => {
                        let toca = e.proxima.is_some_and(|p| ahora >= p);
                        if toca {
                            e.proxima = Some(ahora + Duration::from_secs_f32(azar.entre(entre.0, entre.1)));
                        }
                        if let Some(p) = e.proxima {
                            if mientras.es_verdad(c) {
                                citas.push(p);
                            }
                        }
                        toca && mientras.es_verdad(c)
                    }
                    Disparador::Al(_) => false, // se atienden con los sucesos, abajo
                };
                if dispara {
                    efectos.extend(r.efectos.iter().cloned());
                }
            }
        }

        // Efectos y sucesos, hasta que no quede ninguno (con tope: una regla
        // que se dispara a sí misma no cuelga al render).
        let mut tope = 8;
        while (!efectos.is_empty() || !sucesos.is_empty() || !gestos_pedidos.is_empty()) && tope > 0 {
            tope -= 1;
            for ef in std::mem::take(&mut efectos) {
                match ef {
                    Efecto::Animar(t) => pendientes.push((ahora + t.retraso, t)),
                    Efecto::Hecho(h, v) => hechos[h.0 as usize] = v,
                    Efecto::Alternar(h) => hechos[h.0 as usize] = if hechos[h.0 as usize] > 0.5 { 0.0 } else { 1.0 },
                    Efecto::Suceso(s) => sucesos.push((s.0 as usize, false)),
                    Efecto::Impulso(p, v) => props[p.0 as usize].v += v,
                    Efecto::Gesto(g) => gestos_pedidos.push(g.0 as usize),
                }
            }
            for (s, de_la_logica) in std::mem::take(&mut sucesos) {
                let id = SucesoId(s as u16);
                for (capa, est) in escena.capas.iter().zip(capas.iter_mut()) {
                    for (k, r) in capa.reclamaciones.iter().enumerate() {
                        match &r.cuando {
                            Cuando::Tras { sucesos: ss, dura } if ss.contains(&id) => est.hasta[k] = Some(ahora + *dura),
                            Cuando::DesdeHasta { desde, hasta } => {
                                if desde.contains(&id) {
                                    est.encendida[k] = true;
                                } else if hasta.contains(&id) {
                                    est.encendida[k] = false;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                for r in &escena.reglas {
                    if matches!(&r.cuando, Disparador::Al(x) if *x == id) {
                        efectos.extend(r.efectos.iter().cloned());
                    }
                }
                let (nombre, sale) = escena.sucesos[s];
                if sale && !de_la_logica {
                    let _ = a_logica.send(Evento::Suceso(nombre));
                }
            }
            // Gestos: uno solo corta a otro de su clase o inferior.
            for g in std::mem::take(&mut gestos_pedidos) {
                let nuevo = &escena.gestos[g];
                let puesto = gesto.as_ref().map(|r| escena.gestos[r.gesto].clase);
                if puesto.is_some_and(|c| c > nuevo.clase) {
                    let _ = a_logica.send(Evento::GestoRechazado(nuevo.nombre));
                    continue;
                }
                gesto = Some(Reproduccion::empezar(g, nuevo, &escena.pose, &props, ahora, op.reducido));
            }
        }

        pendientes.retain(|(cuando, t)| {
            if *cuando <= ahora {
                let a = &mut props[t.prop.0 as usize];
                a.objetivo = t.a;
                a.muelle = t.muelle;
                if op.reducido {
                    a.posar();
                }
                false
            } else {
                true
            }
        });
        citas.extend(pendientes.iter().map(|p| p.0));

        // Capas: gana la primera reclamación que se cumple.
        for (capa, est) in escena.capas.iter().zip(capas.iter_mut()) {
            let c = Ctx { props: &props, hechos: &hechos };
            let gana = capa
                .reclamaciones
                .iter()
                .enumerate()
                .position(|(k, r)| match &r.cuando {
                    Cuando::Siempre => true,
                    Cuando::Mientras(e) => e.es_verdad(c),
                    Cuando::Tras { .. } => est.hasta[k].is_some_and(|h| ahora < h),
                    Cuando::DesdeHasta { .. } => est.encendida[k],
                })
                .unwrap_or(capa.reclamaciones.len().saturating_sub(1));
            citas.extend(est.hasta.iter().flatten().filter(|h| **h > ahora));
            if est.gana != Some(gana) {
                let primera_vez = est.gana.is_none();
                let antes = est.gana.map_or("—", |k| capa.reclamaciones[k].nombre);
                est.gana = Some(gana);
                let r = &capa.reclamaciones[gana];
                for (k, p) in capa.presencias.iter().enumerate() {
                    let a = &mut props[p.0 as usize];
                    a.objetivo = if k == gana { 1.0 } else { 0.0 };
                    if primera_vez || op.reducido {
                        a.posar();
                    }
                }
                for t in &r.fija {
                    if primera_vez {
                        props[t.prop.0 as usize].fijar(t.a);
                    } else {
                        pendientes.push((ahora + t.retraso, t.clone()));
                    }
                }
                if !primera_vez {
                    println!("capa {} · {} → {}", capa.nombre, antes, r.nombre);
                    let _ = a_logica.send(Evento::Capa(capa.nombre, r.nombre));
                }
            }
        }

        // Comportamientos: la vida propia de la escena.
        let mut vivo = false;
        let mut n_parpadeo = 0;
        for comp in &escena.comportamientos {
            match comp {
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
                    citas.push(*proximo);
                }
                Comportamiento::Onda { prop, frecuencia, amplitud } => {
                    let a = amplitud.evaluar(Ctx { props: &props, hechos: &hechos });
                    props[prop.0 as usize].fijar(a * (t_total * frecuencia).sin());
                    vivo |= a.abs() > 0.01;
                }
                Comportamiento::Avance { prop, por_segundo } => {
                    let v = por_segundo.evaluar(Ctx { props: &props, hechos: &hechos });
                    let a = &mut props[prop.0 as usize];
                    a.fijar(a.x + v * dt);
                    vivo |= v.abs() > 1e-4;
                }
                Comportamiento::Mirada { x, y, centro, alcance, distancia, reposo } => {
                    let c = Ctx { props: &props, hechos: &hechos };
                    let (mx, my) = match puntero {
                        Some((px, py)) => {
                            let (dx, dy) = (px - centro.0.evaluar(c), py - centro.1.evaluar(c));
                            let lejos = dx.hypot(dy).max(1.0);
                            let tira = (lejos / distancia).min(1.0);
                            (dx / lejos * alcance.0 * tira, dy / lejos * alcance.1 * tira)
                        }
                        None => (reposo.0.evaluar(c), reposo.1.evaluar(c)),
                    };
                    props[x.0 as usize].objetivo = mx;
                    props[y.0 as usize].objetivo = my;
                }
            }
        }

        // Posturas: gestos que se repiten solos mientras algo sea verdad.
        if gesto.is_none() {
            let c = Ctx { props: &props, hechos: &hechos };
            if let Some((g, _)) = escena.posturas.iter().find(|(_, e)| e.es_verdad(c)) {
                let k = g.0 as usize;
                gesto = Some(Reproduccion::empezar(k, &escena.gestos[k], &escena.pose, &props, ahora, op.reducido));
            }
        }

        for a in &mut props {
            a.avanzar(dt);
        }

        // El gesto lleva de la mano a las propiedades de la pose; al acabar
        // las suelta y sus muelles las devuelven a la base.
        let mut fin_del_gesto = false;
        if let Some(r) = &mut gesto {
            vivo = true;
            let g = &escena.gestos[r.gesto];
            loop {
                let f = &g.fotogramas[r.fotograma];
                let pasado = (ahora - r.inicio).as_secs_f32() * 1000.0;
                if pasado < (f.ms + f.aguanta) as f32 {
                    break;
                }
                // Llegó: lo que venga parte de donde este termina.
                for (k, p) in escena.pose.iter().enumerate() {
                    r.desde[k] = r.destino(f, *p, &props);
                }
                r.inicio += Duration::from_millis((f.ms + f.aguanta) as u64);
                r.fotograma += 1;
                if r.fotograma >= g.fotogramas.len() {
                    break;
                }
                if let Some(s) = g.fotogramas[r.fotograma].emite {
                    sucesos_tardios.push(s);
                }
            }
            if r.fotograma >= g.fotogramas.len() {
                for (k, p) in escena.pose.iter().enumerate() {
                    let a = &mut props[p.0 as usize];
                    a.x = if op.reducido { a.objetivo } else { r.desde[k] };
                    a.v = 0.0;
                }
                fin_del_gesto = true;
            } else {
                let f = &g.fotogramas[r.fotograma];
                let t = if f.ms == 0 { 1.0 } else { (ahora - r.inicio).as_secs_f32() * 1000.0 / f.ms as f32 };
                let avance = if r.quieta.is_some() { 1.0 } else { f.curva.aplicar(t) };
                let de = r.quieta.map_or(f, |q| &g.fotogramas[q]);
                for (k, p) in escena.pose.iter().enumerate() {
                    let hasta = r.destino(de, *p, &props);
                    let a = &mut props[p.0 as usize];
                    a.x = r.desde[k] + (hasta - r.desde[k]) * avance;
                    a.v = 0.0;
                }
            }
        }
        if fin_del_gesto {
            gesto = None;
        }

        // ── 3. pintar ───────────────────────────────────────────
        let bloqueada = logica_bloqueada.load(Ordering::Relaxed);
        dibujo.componer(&escena.instrs, Ctx { props: &props, hechos: &hechos }, op.hud);
        cola.write_buffer(&bufer_formas, 0, bytemuck::cast_slice(&dibujo.formas));
        cola.write_buffer(&bufer_elementos, 0, bytemuck::cast_slice(&dibujo.elementos));
        let cabecera = [
            ANCHO as f32, ALTO as f32, t_total, 0.0,
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
        let pase_a = |codificador: &mut wgpu::CommandEncoder, destino: &wgpu::TextureView, capas: &wgpu::BindGroup, tramos: &[std::ops::Range<u32>]| {
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
            pase.set_pipeline(&tuberia);
            pase.set_bind_group(0, &grupo, &[]);
            pase.set_bind_group(1, capas, &[]);
            for t in tramos.iter().filter(|t| !t.is_empty()) {
                pase.draw(0..6, t.clone());
            }
        };
        // Primero los grupos con opacidad, cada uno a su capa…
        let mut principal: Vec<std::ops::Range<u32>> = Vec::new();
        let mut desde = 0u32;
        for (tramo, capa) in &dibujo.apartes {
            pase_a(&mut codificador, &vistas_de_capa[*capa], &grupo_sin_capas, std::slice::from_ref(tramo));
            principal.push(desde..tramo.start);
            desde = tramo.end;
        }
        principal.push(desde..dibujo.n_elementos() as u32);
        // …y luego todo lo demás, con las capas ya hechas entre medias.
        pase_a(&mut codificador, &vista, &grupo_capas, &principal);
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
        if op.sin_vsync && ciclo.dts.len() >= 600 {
            ciclo.cerrar();
        }
        if !op.sin_vsync && !vivo && !bloqueada && props.iter().all(Animada::quieta) {
            for a in &mut props {
                a.posar();
            }
            ciclo.cerrar();
            proxima_cita = citas.iter().min().copied();
            en_reposo = true;
        }
    }
}

struct EstadoCapa {
    gana: Option<usize>,
    hasta: Vec<Option<Instant>>,
    encendida: Vec<bool>,
}

impl EstadoCapa {
    fn nueva(n: usize) -> Self {
        EstadoCapa { gana: None, hasta: vec![None; n], encendida: vec![false; n] }
    }
}

#[derive(Default)]
struct EstadoRegla {
    desde: Option<Instant>,
    disparada: bool,
    armada: bool,
    proxima: Option<Instant>,
    ultima_vez: Option<Instant>,
}

impl EstadoRegla {
    /// «Lleva este rato siendo verdad»: dispara una vez por episodio.
    fn esperar(&mut self, verdad: bool, durante: Duration, ahora: Instant, citas: &mut Vec<Instant>) -> bool {
        if !verdad {
            self.desde = None;
            self.disparada = false;
            return false;
        }
        let desde = *self.desde.get_or_insert(ahora);
        if self.disparada {
            return false;
        }
        if ahora - desde >= durante {
            self.disparada = true;
            return true;
        }
        citas.push(desde + durante);
        false
    }
}

struct Reproduccion {
    gesto: usize,
    fotograma: usize,
    inicio: Instant,
    desde: Vec<f32>,
    /// Con movimiento reducido: el fotograma que se enseña, quieto, todo el rato.
    quieta: Option<usize>,
}

impl Reproduccion {
    fn empezar(k: usize, g: &Gesto, pose: &[PropId], props: &[Animada], ahora: Instant, reducido: bool) -> Self {
        // La cara quieta de un gesto es el fotograma que más se aparta de la base.
        let quieta = reducido.then(|| {
            let puntos = |f: &Fotograma| -> f32 {
                f.valores.iter().map(|(p, v)| (v - props[p.0 as usize].objetivo).abs() / props[p.0 as usize].objetivo.abs().max(1.0)).sum()
            };
            (0..g.fotogramas.len()).max_by(|a, b| puntos(&g.fotogramas[*a]).total_cmp(&puntos(&g.fotogramas[*b]))).unwrap_or(0)
        });
        Reproduccion { gesto: k, fotograma: 0, inicio: ahora, desde: pose.iter().map(|p| props[p.0 as usize].x).collect(), quieta }
    }

    /// Adónde lleva este fotograma a una propiedad: a lo que diga, o a su base.
    fn destino(&self, f: &Fotograma, p: PropId, props: &[Animada]) -> f32 {
        f.valores.iter().find(|(q, _)| *q == p).map_or(props[p.0 as usize].objetivo, |(_, v)| *v)
    }
}

const POR_FORMA: usize = 20;
const POR_ELEMENTO: usize = 52;
/// Cuántos grupos con opacidad pueden estar fundiéndose en el mismo frame.
const MAX_CAPAS: usize = 4;
const MAX_FORMAS: usize = 4096;
const MAX_ELEMENTOS: usize = 2048;

/// La lista de dibujo convertida en lo que pinta la GPU: formas evaluadas y
/// elementos con su caja. Se recompone cada frame; son unos pocos cientos de
/// números.
#[derive(Default)]
struct Dibujo {
    formas: Vec<f32>,
    elementos: Vec<f32>,
    /// Grupos que se pintan aparte: qué elementos, y en qué capa.
    apartes: Vec<(std::ops::Range<u32>, usize)>,
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
    fn n_elementos(&self) -> usize {
        self.elementos.len() / POR_ELEMENTO
    }

    fn forma(&mut self, p: crate::formas::Plana, fusion: f32) -> usize {
        let k = self.formas.len() / POR_FORMA;
        self.formas.resize(self.formas.len() + POR_FORMA, 0.0);
        p.codificar(fusion, &mut self.formas[k * POR_FORMA..]);
        k
    }

    /// Un elemento solo existe si su caja, recortada, toca la pantalla.
    fn elemento(&mut self, tipo: f32, caja: [f32; 4], recortes: &[(usize, [f32; 4])], rellenar: impl FnOnce(&mut [f32])) {
        let mut c = [caja[0].max(0.0), caja[1].max(0.0), caja[2].min(ANCHO as f32), caja[3].min(ALTO as f32)];
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

    fn componer(&mut self, instrs: &[Instr], c: Ctx, hud: bool) {
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

        for i in instrs {
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
                Instr::Textura { destino, uv, alfa } => {
                    let a = alfa.evaluar(c).clamp(0.0, 1.0) * veces;
                    if a <= 0.001 {
                        continue;
                    }
                    let d = [destino.0.evaluar(c), destino.1.evaluar(c), destino.2.evaluar(c), destino.3.evaluar(c)];
                    // Girada o no, su caja es la de sus cuatro esquinas transformadas.
                    let caja = afin.caja([d[0], d[1], d[0] + d[2], d[1] + d[3]]);
                    self.elemento(1.0, [caja[0] - 1.0, caja[1] - 1.0, caja[2] + 1.0, caja[3] + 1.0], &recortes, |e| {
                        afin.codificar(&mut e[44..52]);
                        e[3] = a;
                        e[36..40].copy_from_slice(&d);
                        e[40..44].copy_from_slice(uv);
                    });
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
            self.elemento(9.0, [0.0, 190.0, ANCHO as f32, ALTO as f32], &[], |e| Afin::IDENTIDAD.codificar(&mut e[44..52]));
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
