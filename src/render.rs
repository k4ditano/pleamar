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
                    assert!(nueva.instrs.len() <= MAX_INSTR, "la escena no cabe en la lista de dibujo");
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
                let esta = z.activa.es_verdad(c) && puntero.is_some_and(|(x, y)| z.forma.distancia(c, x, y) < 0.0);
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
        for (k, i) in escena.instrs.iter().enumerate() {
            codificar(i, Ctx { props: &props, hechos: &hechos }, &mut lista[k * POR_INSTR..(k + 1) * POR_INSTR]);
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

const POR_INSTR: usize = 20;

/// Una instrucción, con sus expresiones ya evaluadas, en los veinte números
/// que lee el shader.
fn codificar(i: &Instr, props: Ctx, s: &mut [f32]) {
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
