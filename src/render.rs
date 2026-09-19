//! El hilo que pinta. Es dueño de los muelles y del reloj: recibe intenciones,
//! las recorre a la cadencia de la pantalla y, cuando todo está quieto, deja
//! de pintar del todo.

use crate::escena::*;
use crate::gpu::{Dibujo, Gpu, Lamina, ALTO_INSTRUMENTOS, N_UNIFORMES};
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
    pub arranque: Instant,
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
    rx: Receiver<ARender>,
    mut letras: crate::texto::Textos,
    a_logica: Sender<Evento>,
    logica_bloqueada: Arc<AtomicBool>,
    op: Opciones,
) {
    let mut gpu: Option<Gpu> = None;
    let mut laminas: Vec<Lamina> = Vec::new();
    let mut dibujo = Dibujo::default();
    let mut atlas_por_rehacer = false;
    let mut primer_frame = true;
    let mut textos: Vec<String> = Vec::new();
    let mut uniformes = [0f32; N_UNIFORMES];
    let mut tam = (720.0f32, 224.0f32);
    let mut region: Vec<[i32; 4]> = vec![[i32::MIN; 4]];

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
        let mut sucesos: Vec<(usize, bool, Option<f32>)> = sucesos_tardios.drain(..).map(|s| (s.0 as usize, false, None)).collect();
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
                    // Lo que era verdad y lo que ponían los textos también sobrevive: al
                    // recargar en caliente, la escena sigue donde estaba.
                    let en_caliente = !escena.props.is_empty();
                    hechos = nueva.hechos.iter().map(|(n, inicial)| escena.hechos.iter().position(|h| h.0 == *n).map_or(*inicial, |k| hechos[k])).collect();
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
                    capas = nueva.capas.iter().map(|c| EstadoCapa::nueva(c.reclamaciones.len(), en_caliente)).collect();
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
                    tam = (nueva.superficie.ancho as f32, nueva.superficie.alto as f32 + if op.hud { ALTO_INSTRUMENTOS } else { 0.0 });
                    textos = nueva.textos.iter().map(|(n, inicial)| escena.textos.iter().position(|t| t.0 == *n).map_or_else(|| inicial.clone(), |k| textos[k].clone())).collect();
                    atlas_por_rehacer = true;
                    println!(
                        "render · escena: {} propiedades, {} instrucciones, {} hechos, {} capas, {} gestos, {} reglas, {} zonas",
                        nueva.props.len(), nueva.instrs.len(), nueva.hechos.len(), nueva.capas.len(),
                        nueva.gestos.len(), nueva.reglas.len(), nueva.zonas.len()
                    );
                    escena = nueva;
                }
                ARender::Lamina(n) => {
                    let g = gpu.get_or_insert_with(|| Gpu::nueva(&instancia, &n.superficie));
                    // Una escena que pide «todo el ancho» mide lo que mida su monitor, y lo
                    // puede saber: `screen.width`.
                    if escena.superficie.ancho == 0 {
                        tam.0 = n.tam.0 as f32;
                    }
                    for (k, (nombre, _)) in escena.hechos.iter().enumerate() {
                        match *nombre {
                            "screen.width" => hechos[k] = tam.0,
                            "screen.height" => hechos[k] = escena.superficie.alto as f32,
                            _ => {}
                        }
                    }
                    println!("render · lámina {} en {} · {}×{} · escala {} · {:.0} Hz", n.id, n.nombre, n.tam.0, n.tam.1, n.escala, n.mhz as f32 / 1000.0);
                    laminas.push(g.lamina(*n, tam));
                    repartir_el_ritmo(g, &mut laminas, tam, op.sin_vsync);
                    region = vec![[i32::MIN; 4]];
                }
                ARender::LaminaFuera(id) => {
                    laminas.retain(|l| l.id != id);
                    println!("render · lámina {id} fuera; quedan {}", laminas.len());
                    if let Some(g) = &gpu {
                        repartir_el_ritmo(g, &mut laminas, tam, op.sin_vsync);
                    }
                }
                ARender::Taller(p) => letras.recibir(*p),
                ARender::Escala(id, e) => {
                    if let (Some(g), Some(l)) = (&gpu, laminas.iter_mut().find(|l| l.id == id)) {
                        if (l.escala - e).abs() > 0.001 {
                            println!("render · lámina {id}: escala {} → {e}", l.escala);
                            l.escala = e;
                            g.configurar(l, tam);
                        }
                    }
                }
                ARender::Orden(Orden::Animar(t)) => pendientes.push((Instant::now() + t.retraso, t)),
                ARender::Orden(Orden::Impulso { prop, velocidad }) => props[prop.0 as usize].v += velocidad,
                ARender::Orden(Orden::Bloquear(d)) => bloquear = Some(d),
                ARender::Texto(nombre, valor) => match escena.textos.iter().position(|t| t.0 == nombre) {
                    Some(i) => textos[i] = valor,
                    None => eprintln!("render · no conozco el texto «{nombre}»"),
                },
                ARender::Hecho(nombre, v) => match escena.hechos.iter().position(|h| h.0 == nombre) {
                    Some(i) => hechos[i] = v,
                    None => eprintln!("render · no conozco el hecho «{nombre}»"),
                },
                ARender::Suceso(nombre) => match escena.sucesos.iter().position(|s| s.0 == nombre) {
                    Some(i) => sucesos.push((i, true, None)),
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
                    // Un hecho que cambia una regla se le cuenta a la lógica, que si no
                    // se quedaría creyendo lo que ella misma dijo la última vez.
                    Efecto::Hecho(h, v) => {
                        hechos[h.0 as usize] = v;
                        let _ = a_logica.send(Evento::Hecho(escena.hechos[h.0 as usize].0, v));
                    }
                    Efecto::Alternar(h) => {
                        let v = if hechos[h.0 as usize] > 0.5 { 0.0 } else { 1.0 };
                        hechos[h.0 as usize] = v;
                        let _ = a_logica.send(Evento::Hecho(escena.hechos[h.0 as usize].0, v));
                    }
                    Efecto::Suceso(s, carga) => {
                        let v = carga.as_ref().map(|e| e.evaluar(Ctx { props: &props, hechos: &hechos }));
                        sucesos.push((s.0 as usize, false, v));
                    }
                    Efecto::Impulso(p, v) => props[p.0 as usize].v += v,
                    Efecto::Gesto(g) => gestos_pedidos.push(g.0 as usize),
                }
            }
            for (s, de_la_logica, carga) in std::mem::take(&mut sucesos) {
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
                    let _ = a_logica.send(Evento::Suceso(nombre, carga));
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
                gesto = Some(Reproduccion::empezar(g, nuevo, &escena.pose, Ctx { props: &props, hechos: &hechos }, ahora, op.reducido));
            }
        }

        pendientes.retain(|(cuando, t)| {
            if *cuando <= ahora {
                // El destino se calcula ahora, no cuando se declaró.
                let destino = t.a.evaluar(Ctx { props: &props, hechos: &hechos });
                let a = &mut props[t.prop.0 as usize];
                a.objetivo = destino;
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
                // Recién cargada, una capa se posa donde toca. Recargada en caliente
                // no: si el fichero cambió un destino, se va hacia él con su muelle.
                let primera_vez = est.gana.is_none() && !est.en_caliente;
                let callada = est.gana.is_none();
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
                        let v = t.a.evaluar(Ctx { props: &props, hechos: &hechos });
                        props[t.prop.0 as usize].fijar(v);
                    } else {
                        pendientes.push((ahora + t.retraso, t.clone()));
                    }
                }
                if !primera_vez && !callada {
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
                Comportamiento::Sigue { prop, a } => {
                    let v = a.evaluar(Ctx { props: &props, hechos: &hechos });
                    props[prop.0 as usize].objetivo = v;
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
                gesto = Some(Reproduccion::empezar(k, &escena.gestos[k], &escena.pose, Ctx { props: &props, hechos: &hechos }, ahora, op.reducido));
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
                    r.desde[k] = r.destino(r.fotograma, *p, &props);
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
                let de = r.quieta.unwrap_or(r.fotograma);
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
        let c = Ctx { props: &props, hechos: &hechos };
        // El texto y las imágenes se pintan a la escala de la lámina más fina; las
        // demás lo ven reducido, que se nota mucho menos que ampliado.
        let fina = laminas.iter().map(|l| l.escala).fold(1.0f32, f32::max);
        if atlas_por_rehacer || (fina - letras.escala()).abs() > 0.001 {
            letras.empezar_de_cero(fina, &escena.imagenes);
            atlas_por_rehacer = false;
        }
        // Se compone aunque aún no haya dónde pintar: así el taller va haciendo
        // los textos y las imágenes mientras la GPU y las ventanas arrancan.
        dibujo.componer(&escena.instrs, c, &textos, &mut letras, tam, op.hud);
        let Some(g) = &gpu else {
            // Aún no hay dónde: el tiempo corre igual, pero sin prisa.
            std::thread::sleep(Duration::from_millis(8));
            continue;
        };
        g.subir_atlas(&mut letras.por_subir);
        g.subir(&dibujo);

        // Por dónde entra el ratón: las zonas activas, y nada más. Lo demás de
        // la superficie es transparente también para el clic.
        let cajas: Vec<[i32; 4]> = escena
            .zonas
            .iter()
            .filter(|z| z.activa.es_verdad(c))
            .filter_map(|z| z.caja(c))
            .map(|b| [(b[0] - 3.0).floor() as i32, (b[1] - 3.0).floor() as i32, (b[2] + 3.0).ceil() as i32, (b[3] + 3.0).ceil() as i32])
            .collect();
        let cambia_la_region = cajas != region;
        if cambia_la_region {
            for l in &laminas {
                l.region_de_entrada(&cajas);
            }
            region = cajas;
        }

        uniformes[..8].copy_from_slice(&[tam.0, tam.1, t_total, 1.0, if op.hud { 1.0 } else { 0.0 }, periodo_ms, if bloqueada { 1.0 } else { 0.0 }, 0.0]);
        uniformes[8..].copy_from_slice(&historial);
        // La que marca el ritmo va la última: es la que espera a la pantalla.
        laminas.sort_by_key(|l| l.marca_el_ritmo);
        let mut pintadas = 0;
        for l in &mut laminas {
            pintadas += g.pintar(l, &dibujo, &uniformes) as u32;
        }
        if pintadas == 0 {
            std::thread::sleep(Duration::from_millis(8));
        } else if primer_frame {
            primer_frame = false;
            println!("render · primer frame a los {} ms de arrancar", op.arranque.elapsed().as_millis());
        }

        // Lo que midieron los textos pasa a sus propiedades: el frame que viene,
        // la caja que dependa de ellas ya lo sabe.
        let mut cambio_de_medida = false;
        for (p, v) in &dibujo.medidas {
            let a = &mut props[p.0 as usize];
            if (a.x - v).abs() > 0.01 {
                a.fijar(*v);
                cambio_de_medida = true;
            }
        }

        // ── 4. medir ────────────────────────────────────────────
        let ms = dt * 1000.0;
        // Un chivato permanente: cualquier frame que se pase de dos periodos, con su hora.
        if ms > periodo_ms * 2.4 && !primer_frame && ciclo.dts.len() > 1 && !op.sin_vsync && !op.ingenuo {
            println!("render · frame lento: {ms:.0} ms a los {t_total:.2} s");
        }
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
        if !op.sin_vsync && !vivo && !bloqueada && !cambia_la_region && !cambio_de_medida && props.iter().all(Animada::quieta) {
            for a in &mut props {
                a.posar();
            }
            ciclo.cerrar();
            proxima_cita = citas.iter().min().copied();
            en_reposo = true;
        }
    }
}

/// Una sola lámina espera a su pantalla —la del monitor más rápido— y las demás
/// presentan sin bloquear. Si esperasen todas, un monitor a 60 Hz frenaría a
/// otro a 165.
fn repartir_el_ritmo(g: &Gpu, laminas: &mut [Lamina], tam: (f32, f32), sin_vsync: bool) {
    let rapida = laminas.iter().max_by_key(|l| l.mhz).map(|l| l.id);
    for l in laminas.iter_mut() {
        let marca = !sin_vsync && Some(l.id) == rapida;
        if l.marca_el_ritmo != marca {
            l.marca_el_ritmo = marca;
            g.configurar(l, tam);
        }
    }
}

struct EstadoCapa {
    en_caliente: bool,
    gana: Option<usize>,
    hasta: Vec<Option<Instant>>,
    encendida: Vec<bool>,
}

impl EstadoCapa {
    fn nueva(n: usize, en_caliente: bool) -> Self {
        EstadoCapa { en_caliente, gana: None, hasta: vec![None; n], encendida: vec![false; n] }
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
    valores: Vec<Vec<(PropId, f32)>>,
}

impl Reproduccion {
    fn empezar(k: usize, g: &Gesto, pose: &[PropId], c: Ctx, ahora: Instant, reducido: bool) -> Self {
        let props = c.props;
        // Los valores de cada fotograma se fijan al empezar.
        let valores: Vec<Vec<(PropId, f32)>> = g.fotogramas.iter().map(|f| f.valores.iter().map(|(p, e)| (*p, e.evaluar(c))).collect()).collect();
        // La cara quieta de un gesto es el fotograma que más se aparta de la base.
        let quieta = reducido.then(|| {
            let puntos = |f: &Vec<(PropId, f32)>| -> f32 { f.iter().map(|(p, v)| (v - props[p.0 as usize].objetivo).abs() / props[p.0 as usize].objetivo.abs().max(1.0)).sum() };
            (0..valores.len()).max_by(|a, b| puntos(&valores[*a]).total_cmp(&puntos(&valores[*b]))).unwrap_or(0)
        });
        Reproduccion { gesto: k, fotograma: 0, inicio: ahora, desde: pose.iter().map(|p| props[p.0 as usize].x).collect(), quieta, valores }
    }

    /// Adónde lleva un fotograma a una propiedad: a lo que diga, o a su base.
    fn destino(&self, fotograma: usize, p: PropId, props: &[Animada]) -> f32 {
        self.valores[fotograma].iter().find(|(q, _)| *q == p).map_or(props[p.0 as usize].objetivo, |(_, v)| *v)
    }
}
