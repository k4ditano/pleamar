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
    /// Qué apuntar en cada frame: propiedades, hechos o textos, por su nombre.
    /// Es como se comprueba que una animación dura lo que dice que dura.
    pub registrar: Vec<String>,
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
    /// Lo que se va en LEER la escena —evaluar sus expresiones y apuntar lo que
    /// hay que pintar—, frente a lo que se va en dibujarla. Se mide siempre:
    /// son dos relojes por frame, y es la única manera de que quien escribe una
    /// escena se entere de que la ha hecho demasiado cara.
    componer: f32,
    lento_dicho: bool,
}

impl Ciclo {
    /// Una escena que no da los frames que la pantalla pide no se nota en el
    /// log —son frames lentos sueltos, uno detrás de otro— y desde fuera parece
    /// que el runtime se ha vuelto lento. Se dice UNA vez, con el reparto, en
    /// cuanto hay con qué decirlo: cuatro segundos de ir por detrás.
    fn vigilar(&mut self, periodo_ms: f32) {
        if self.lento_dicho || self.dts.len() < 240 {
            return;
        }
        let media = self.dts.iter().sum::<f32>() / self.dts.len() as f32;
        if media <= periodo_ms * 1.35 {
            return;
        }
        self.lento_dicho = true;
        let leer = self.componer / self.dts.len() as f32;
        eprintln!(
            "render · this scene does not keep up: {media:.1} ms a frame against the {periodo_ms:.1} the screen gives, and {leer:.1} of those go in reading the scene, not in drawing it. Something in it is too dear to work out sixty times a second"
        );
    }

    fn cerrar(&mut self) {
        if self.dts.len() < 8 {
            self.dts.clear();
            return;
        }
        let mut o = self.dts.clone();
        o.sort_by(|a, b| a.total_cmp(b));
        let leer = self.componer / o.len() as f32;
        let media = o.iter().sum::<f32>() / o.len() as f32;
        let p99 = o[((o.len() as f32 * 0.99) as usize).min(o.len() - 1)];
        println!(
            "cycle  · {:>4} frames · mean {:>5.2} ms · p99 {:>6.2} ms · max {:>6.2} ms · reading the scene {:.2} ms · with the logic blocked: {} frames, max {:.2} ms",
            o.len(), media, p99, o[o.len() - 1], leer, self.frames_bloqueada, self.max_bloqueada
        );
        let dicho = self.lento_dicho;
        *self = Ciclo::default();
        self.lento_dicho = dicho;
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
    let mut repeticion: Option<(u32, u32)> = Some((400, 33));
    // Cada emergente de la escena: si está abierta, dónde y con qué tamaño.
    let mut emergentes: Vec<Option<[i32; 4]>> = Vec::new();
    let mut uniformes = [0f32; N_UNIFORMES];
    let mut tam = (720.0f32, 224.0f32);
    // Si se pidió `--registrar`, la cabecera se escribe una vez.
    let mut registro_dicho = false;
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
    // Lo que se está arrastrando: qué zona, y dónde estaba el ratón al pulsar.
    let mut arrastre: Option<(usize, (f32, f32), Instant)> = None;
    let mut cursor_puesto = Cursor::Normal;
    let mut teclado_pedido = false;
    // El campo donde se está escribiendo, y la tecla que se ha quedado pulsada.
    let mut edicion: Option<Edicion> = None;
    let mut repite: Option<(String, Option<String>, Mods, Instant)> = None;
    let mut ultima_tecla = Instant::now();
    let mut ultimo_puntero: Option<(f32, f32)> = None;
    let mut ultima_actividad = Instant::now();
    let mut azar = Azar(0x9E3779B97F4A7C15);
    let inicio = Instant::now();

    let mut historial = [0f32; 120];
    let mut ciclo = Ciclo::default();
    let mut ultimo = Instant::now();
    let mut en_reposo = false;
    let mut proxima_cita: Option<Instant> = None;
    let mut periodo_ms = 16.7f32;
    let mut ultimo_presentado = Instant::now();
    // A qué borde está pegada cada superficie ahora mismo, para no pedirlo dos veces.
    let mut anclas_puestas: Vec<crate::escena::Ancla> = Vec::new();
    // La banda de «esto no compila», y la escena buena con ella encima.
    let mut aviso: Option<Vec<Instr>> = None;
    let mut con_aviso: Vec<Instr> = Vec::new();
    let mut proximo_frame = Instant::now();
    let mut frames_para_periodo = 0u32;

    loop {
        // ── 1. lo que ha llegado ────────────────────────────────
        let mut bloquear = None;
        // (qué botón, si baja o sube), las muescas de rueda y las teclas de este frame.
        let mut botones: Vec<(u8, bool)> = Vec::new();
        let mut rueda = 0.0f32;
        let mut teclas: Vec<String> = Vec::new();
        let mut pulsaciones: Vec<(String, Option<String>, Mods)> = Vec::new();
        let mut cambios_de_foco: Vec<bool> = Vec::new();
        let mut soltados: Vec<(String, String)> = Vec::new();
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
                // Una recarga que no cuela se enseña en la superficie, no solo en
                // una consola que puede que nadie esté mirando. Se compone una
                // vez, al llegar: mientras dure, se pinta la escena buena con la
                // banda encima.
                ARender::FalloDeRecarga(que) => {
                    // `size: full` no dice su ancho hasta que el compositor la
                    // configura: entonces vale el de la pantalla, que es `screen.width`.
                    let ancho = match escena.superficie().ancho {
                        0 => hechos.first().copied().unwrap_or(900.0),
                        w => w as f32,
                    };
                    aviso = que.map(|m| banda_de_fallo(&m, ancho));
                    con_aviso.clear();
                    if let Some(banda) = &aviso {
                        con_aviso.extend(escena.instrs.iter().cloned());
                        con_aviso.extend(banda.iter().cloned());
                    }
                }
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
                    aviso = None;
                    con_aviso.clear();
                    anclas_puestas = nueva.superficies.iter().map(|s| s.ancla).collect();
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
                    tam = (nueva.superficie().ancho as f32, nueva.superficie().alto as f32 + if op.hud { ALTO_INSTRUMENTOS } else { 0.0 });
                    textos = nueva.textos.iter().map(|(n, inicial)| escena.textos.iter().position(|t| t.0 == *n).map_or_else(|| inicial.clone(), |k| textos[k].clone())).collect();
                    atlas_por_rehacer = true;
                    println!(
                        "render · scene: {} properties, {} instructions, {} facts, {} layers, {} gestures, {} rules, {} zones",
                        nueva.props.len(), nueva.instrs.len(), nueva.hechos.len(), nueva.capas.len(),
                        nueva.gestos.len(), nueva.reglas.len(), nueva.zonas.len()
                    );
                    escena = nueva;
                }
                ARender::Lamina(n) => {
                    let g = gpu.get_or_insert_with(|| Gpu::nueva(&instancia, &n.superficie));
                    // Una escena que pide «todo el ancho» mide lo que mida su monitor, y lo
                    // puede saber: `screen.width`.
                    // Una ventana mide lo que el compositor le haya dado, y puede cambiar.
                    let es_ventana = escena.superficie().ventana.is_some();
                    let suya = n.vista.superficie == 0 && n.vista.emergente.is_none();
                    if (escena.superficie().ancho == 0 || es_ventana) && suya {
                        tam.0 = n.tam.0 as f32;
                    }
                    if es_ventana && suya {
                        tam.1 = n.tam.1 as f32;
                    }
                    let alto = if es_ventana { n.tam.1 as f32 } else { escena.superficie().alto as f32 };
                    for (k, (nombre, _)) in escena.hechos.iter().enumerate() {
                        match *nombre {
                            "screen.width" => hechos[k] = tam.0,
                            "screen.height" => hechos[k] = alto,
                            _ => {}
                        }
                    }
                    // Con `screens: each`, cada copia sabe de qué monitor es: su nombre y
                    // lo que mide. Es lo que le deja enseñar lo suyo y no lo de la otra.
                    if let Some(suya) = escena.superficies.get(n.vista.superficie) {
                        let k = suya.instancia;
                        if let Some(i) = escena.textos.iter().position(|t| t.0 == format!("screen.{k}.name")) {
                            textos.resize(escena.textos.len().max(textos.len()), String::new());
                            textos[i] = n.nombre.clone();
                            let _ = a_logica.send(Evento::Texto(escena.textos[i].0, n.nombre.clone()));
                        }
                        for (parte, v) in [("width", n.tam.0 as f32), ("height", n.tam.1 as f32)] {
                            if let Some(i) = escena.hechos.iter().position(|h| h.0 == format!("screen.{k}.{parte}")) {
                                hechos[i] = v;
                            }
                        }
                    }
                    {
                        // Qué superficie de la escena es, no solo el número de lámina: con
                        // `screens: each` hay varias iguales y conviene saber cuál cayó dónde.
                        let cual = escena.superficies.get(n.vista.superficie).map_or(String::new(), |s| match (s.nombre.as_str(), s.instancia) {
                            ("", 0) => String::new(),
                            ("", k) => format!(" · copy {k}"),
                            (nombre, 0) => format!(" · {nombre}"),
                            (nombre, k) => format!(" · {nombre} copy {k}"),
                        });
                        println!("render · surface {} on {} · {}×{} · scale {} · {:.0} Hz{cual}", n.id, n.nombre, n.tam.0, n.tam.1, n.escala, n.mhz as f32 / 1000.0);
                    }
                    laminas.push(g.lamina(*n, tam));
                    // Cuántos monitores están enseñando algo ahora mismo.
                    if let Some(i) = escena.hechos.iter().position(|h| h.0 == "screens.count") {
                        let cuantos = laminas.iter().filter(|l| l.vista.emergente.is_none()).map(|l| l.vista.superficie).collect::<std::collections::HashSet<_>>().len();
                        hechos[i] = cuantos as f32;
                    }
                    repartir_el_ritmo(g, &mut laminas, tam, op.sin_vsync);
                    region = vec![[i32::MIN; 4]];
                }
                ARender::LaminaFuera(id) => {
                    laminas.retain(|l| l.id != id);
                    println!("render · surface {id} gone; {} left", laminas.len());
                    if let Some(g) = &gpu {
                        repartir_el_ritmo(g, &mut laminas, tam, op.sin_vsync);
                    }
                }
                ARender::Taller(p) => letras.recibir(*p),
                // Una ventana que alguien estira: la lámina cambia, y con ella lo que la
                // escena lee en `screen.width` y `screen.height`.
                ARender::TamLamina(id, nuevo) => {
                    if let (Some(g), Some(l)) = (&gpu, laminas.iter_mut().find(|l| l.id == id)) {
                        if l.vista.tam != nuevo {
                            l.vista.tam = nuevo;
                            g.configurar(l, tam);
                            if l.vista.superficie == 0 && l.vista.emergente.is_none() && escena.superficie().ventana.is_some() {
                                // Lo que se pinta fuera de la superficie no existe: si la
                                // ventana crece, el marco tiene que crecer con ella.
                                tam = nuevo;
                                for (k, (nombre, _)) in escena.hechos.iter().enumerate() {
                                    match *nombre {
                                        "screen.width" => hechos[k] = nuevo.0,
                                        "screen.height" => hechos[k] = nuevo.1,
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }
                ARender::Escala(id, e) => {
                    if let (Some(g), Some(l)) = (&gpu, laminas.iter_mut().find(|l| l.id == id)) {
                        if (l.escala - e).abs() > 0.001 {
                            println!("render · surface {id}: scale {} → {e}", l.escala);
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
                    None => eprintln!("render · I don't know the text '{nombre}'"),
                },
                ARender::Hecho(nombre, v) => match escena.hechos.iter().position(|h| h.0 == nombre) {
                    Some(i) => hechos[i] = v,
                    None => eprintln!("render · I don't know the fact '{nombre}'"),
                },
                ARender::EmergenteCerrada(k) => {
                    // Han pulsado fuera: primero se suelta lo que pintaba en ella, luego ella.
                    laminas.retain(|l| l.vista.emergente != Some(k));
                    crate::plataforma::emergente(k, None);
                    if let Some(g) = &gpu {
                        repartir_el_ritmo(g, &mut laminas, tam, op.sin_vsync);
                    }
                    if let Some(abierta) = emergentes.get_mut(k) {
                        *abierta = None;
                    }
                    if let Some(em) = escena.emergentes.get(k) {
                        let h = em.abierta.0 as usize;
                        hechos[h] = 0.0;
                        let _ = a_logica.send(Evento::Hecho(escena.hechos[h].0, 0.0));
                    }
                }
                ARender::HechoDeFuera(nombre, escrito) => match escena.hechos.iter().position(|h| h.0 == nombre) {
                    Some(i) => {
                        let tipo = escena.tipos.iter().find(|(n, _)| n == nombre).map(|(_, t)| t);
                        let v = tipo.and_then(|t| t.de_texto(&escrito)).or_else(|| escrito.parse().ok()).or(match escrito.as_str() { "true" => Some(1.0), "false" => Some(0.0), _ => None });
                        match v {
                            Some(v) => {
                                hechos[i] = v;
                                // No lo ha puesto la lógica: que se entere.
                                let _ = a_logica.send(Evento::Hecho(nombre, v));
                            }
                            None => eprintln!("render · '{nombre}' cannot be '{escrito}'"),
                        }
                    }
                    None => eprintln!("render · I don't know the fact '{nombre}'"),
                },
                ARender::Pregunta(nombre, a_quien) => {
                    let numero = |v: f32| if v.fract() == 0.0 { format!("{}", v as i64) } else { format!("{v}") };
                    let r = if let Some(i) = escena.hechos.iter().position(|h| h.0 == nombre) {
                        // Como se escribiría: `true`, `critical`, o el número.
                        match escena.tipos.iter().find(|(n, _)| n == nombre) {
                            Some((_, t)) => t.como_texto(hechos[i]),
                            None => numero(hechos[i]),
                        }
                    } else if let Some(i) = escena.textos.iter().position(|t| t.0 == nombre) {
                        textos.get(i).cloned().unwrap_or_default()
                    } else if let Some(i) = escena.props.iter().position(|p| p.0 == nombre) {
                        numero(props[i].x)
                    } else {
                        format!("? I don't know '{nombre}'")
                    };
                    let _ = a_quien.send(r);
                }
                ARender::Suceso(nombre) => match escena.sucesos.iter().position(|s| s.0 == nombre) {
                    Some(i) => sucesos.push((i, true, None)),
                    None => eprintln!("render · I don't know the event '{nombre}'"),
                },
                ARender::SucesoDeFuera(nombre, carga) => match escena.sucesos.iter().position(|s| s.0 == nombre) {
                    Some(i) => sucesos.push((i, false, carga)),
                    None => eprintln!("render · I don't know the event '{nombre}'"),
                },
                ARender::Gesto(nombre) => match escena.gestos.iter().position(|g| g.nombre == nombre) {
                    Some(i) => gestos_pedidos.push(i),
                    None => eprintln!("render · I don't know the gesture '{nombre}'"),
                },
                ARender::Puntero(p) => {
                    puntero = p;
                    ultima_actividad = Instant::now();
                }
                ARender::Boton(b, abajo) => {
                    botones.push((b, abajo));
                    ultima_actividad = Instant::now();
                }
                ARender::Rueda(d) => {
                    rueda += d;
                    ultima_actividad = Instant::now();
                }
                ARender::Repeticion(r) => repeticion = r,
                ARender::Tecla(nombre, escribe, mods) => {
                    ultima_actividad = Instant::now();
                    // Si se queda pulsada, se repite, como este usuario lo tenga puesto.
                    repite = repeticion.map(|(espera, _)| (nombre.clone(), escribe.clone(), mods, Instant::now() + Duration::from_millis(espera as u64)));
                    pulsaciones.push((nombre, escribe, mods));
                }
                ARender::TeclaSuelta(nombre) => {
                    if repite.as_ref().is_some_and(|r| r.0 == nombre) {
                        repite = None;
                    }
                }
                ARender::FocoTeclado(si) => cambios_de_foco.push(si),
                ARender::Enfocar(nombre) => {
                    edicion = nombre.and_then(|n| escena.textos.iter().position(|t| t.0 == n)).map(|k| Edicion { campo: k, cursor: textos[k].len(), ancla: textos[k].len() });
                    ultima_tecla = Instant::now();
                }
                ARender::Soltado(tipo, datos) => soltados.push((tipo, datos)),
                ARender::Salir => {
                    ciclo.cerrar();
                    return;
                }
            }
        }
        // La tecla que sigue pulsada vuelve a contar.
        if let Some((nombre, escribe, mods, cuando)) = &mut repite {
            if Instant::now() >= *cuando {
                pulsaciones.push((nombre.clone(), escribe.clone(), *mods));
                *cuando = Instant::now() + Duration::from_millis(repeticion.map_or(33, |r| r.1) as u64);
            }
        }
        let mut enviados: Vec<usize> = Vec::new();
        for (nombre, escribe, mods) in pulsaciones {
            // Primero el campo: lo que sea escribir es suyo. Lo demás —Escape, un
            // atajo— sigue hacia las reglas y hacia la lógica.
            if let Some(ed) = &mut edicion {
                let k = ed.campo;
                match ed.tecla(&mut textos[k], &nombre, escribe.as_deref(), mods) {
                    Tecleo::Cambio => {
                        ultima_tecla = Instant::now();
                        let _ = a_logica.send(Evento::Texto(escena.textos[k].0, textos[k].clone()));
                        continue;
                    }
                    Tecleo::Movio => {
                        ultima_tecla = Instant::now();
                        continue;
                    }
                    Tecleo::Envio => {
                        enviados.push(k);
                        let _ = a_logica.send(Evento::Envia(escena.textos[k].0, textos[k].clone()));
                        continue;
                    }
                    Tecleo::NoEsMio => {}
                }
            }
            let mut combo = String::new();
            for (si, prefijo) in [(mods.ctrl, "Ctrl+"), (mods.alt, "Alt+"), (mods.logo, "Super+")] {
                if si {
                    combo.push_str(prefijo);
                }
            }
            combo.push_str(&nombre);
            let _ = a_logica.send(Evento::Tecla(combo.clone(), escribe));
            teclas.push(combo);
        }
        for si in &cambios_de_foco {
            let _ = a_logica.send(Evento::Foco(*si));
            if *si {
                // Al ganar el teclado, si hay dónde escribir y nadie lo tiene, el primero.
                if edicion.is_none() {
                    edicion = escena.instrs.iter().find_map(|i| if let Instr::Campo { texto, .. } = i { Some(texto.0 as usize) } else { None }).map(|k| Edicion { campo: k, cursor: textos[k].len(), ancla: textos[k].len() });
                }
            } else {
                edicion = None;
                repite = None;
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
        let mut cambio_de_teclado = false;
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
        let encima = dentro.iter().rposition(|d| *d);
        let (mut pulsada, mut pulsada_con, mut soltada) = (None, None, None);
        for (boton, abajo) in &botones {
            match (*boton, *abajo) {
                (0, true) => {
                    pulsada = encima;
                    if let (Some(k), Some(p)) = (encima, puntero) {
                        // Pulsar un campo lo enfoca, con el cursor donde cayó el clic.
                        let id = escena.zonas[k].id;
                        if let Some(puesto) = dibujo.campos.iter().find(|c| c.zona == id) {
                            let local = escena.zonas[k].a_local(Ctx { props: &props, hechos: &hechos }, p.0, p.1);
                            let b = puesto.maqueta.as_ref().map_or(0, |m| m.byte_en(local.0 - puesto.x0 + puesto.corrido));
                            let b = b.min(textos[puesto.texto].len());
                            edicion = Some(Edicion { campo: puesto.texto, cursor: b, ancla: b });
                            ultima_tecla = ahora;
                        }
                        arrastre = Some((k, p, ahora));
                        let _ = a_logica.send(Evento::Pulsa(escena.zonas[k].id));
                    }
                }
                (0, false) => {
                    if let Some((k, _, _)) = arrastre.take() {
                        soltada = Some(k);
                        if let Some(z) = escena.zonas.get(k) {
                            let _ = a_logica.send(Evento::Suelta(z.id));
                        }
                    }
                }
                (b, true) => pulsada_con = encima.map(|k| (k, b)),
                _ => {}
            }
        }
        // La salida de emergencia de un prototipo —el botón derecho cierra— no
        // puede pisarle el botón a una escena que lo usa. Quien sabe si ha caído
        // encima de algo es esto, que es lo que mira las zonas: si debajo del
        // puntero no había ninguna, cierra; si había, el clic es de la escena.
        // La plataforma ya cierra por su cuenta cuando NINGUNA superficie usa el
        // derecho, y entonces esto ni se ejecuta.
        if escena.superficies.iter().any(|s| !s.derecho_cierra) && encima.is_none()
            && botones.iter().any(|(b, abajo)| *b == 1 && *abajo)
        {
            crate::plataforma::pedir_salir();
        }
        let arrastrada = match (arrastre, puntero) {
            (Some((k, _, _)), Some(p)) if ultimo_puntero != Some(p) => Some(k),
            _ => None,
        };
        ultimo_puntero = puntero;
        // La rueda no es solo de la zona de más arriba: vale para cualquiera que tenga
        // debajo, que una píldora entera quiere la rueda aunque dentro haya un botón.
        if rueda != 0.0 {
            if let Some(k) = encima {
                let _ = a_logica.send(Evento::Rueda(escena.zonas[k].id, rueda));
            }
        }

        // Lo que una regla puede leer del ratón: dónde está, dónde dentro de la zona
        // que tiene entre manos, cuánto lleva arrastrado y cuánto ha girado la rueda.
        {
            let foco = arrastre.map(|a| a.0).or(encima);
            let (px, py) = puntero.unwrap_or((0.0, 0.0));
            let local = foco.and_then(|k| escena.zonas.get(k)).map_or((px, py), |z| z.a_local(Ctx { props: &props, hechos: &hechos }, px, py));
            let (dx, dy) = arrastre.map_or((0.0, 0.0), |(_, o, _)| (px - o.0, py - o.1));
            for (k, (nombre, _)) in escena.hechos.iter().enumerate() {
                match *nombre {
                    "pointer.x" => hechos[k] = px,
                    "pointer.y" => hechos[k] = py,
                    "local.x" => hechos[k] = local.0,
                    "local.y" => hechos[k] = local.1,
                    "drag.dx" => hechos[k] = dx,
                    "drag.dy" => hechos[k] = dy,
                    "wheel" => hechos[k] = rueda,
                    _ => {}
                }
            }
        }

        for (tipo, datos) in &soltados {
            if let Some(k) = encima {
                let _ = a_logica.send(Evento::Recibido(escena.zonas[k].id, tipo.clone(), datos.clone()));
            }
        }

        // El teclado, solo mientras la escena lo quiera: un lanzador cerrado no
        // puede quedarse con él.
        if let Some(cuando) = &escena.teclado_mientras {
            let quiere = cuando.es_verdad(Ctx { props: &props, hechos: &hechos });
            if quiere != teclado_pedido {
                teclado_pedido = quiere;
                for l in &laminas {
                    l.teclado(if quiere { escena.superficie().teclado } else { Teclado::Nunca });
                }
                cambio_de_teclado = true;
            }
        }

        // El cursor, el de la zona que tenga encima.
        let quiere = arrastre.map(|a| a.0).or(encima).and_then(|k| escena.zonas.get(k)).map_or(Cursor::Normal, |z| z.cursor);
        if quiere != cursor_puesto {
            cursor_puesto = quiere;
            for l in &laminas {
                l.cursor(quiere);
            }
        }

        // Reglas: todo esto ocurre aquí, esté la lógica como esté.
        {
            let c = Ctx { props: &props, hechos: &hechos };
            for (r, e) in escena.reglas.iter().zip(reglas.iter_mut()) {
                let dispara = match &r.cuando {
                    Disparador::Entra(z) => bordes.contains(&(true, z.0 as usize)),
                    Disparador::Sale(z) => bordes.contains(&(false, z.0 as usize)),
                    Disparador::Pulsa(z) => pulsada == Some(z.0 as usize),
                    Disparador::PulsaCon(z, b) => pulsada_con == Some((z.0 as usize, *b)),
                    Disparador::Suelta(z) => soltada == Some(z.0 as usize),
                    Disparador::Rueda(z) => rueda != 0.0 && dentro[z.0 as usize],
                    // Arrastrar no es solo de la zona de más arriba, como la rueda: vale
                    // para cualquiera que estuviera debajo cuando se pulsó. Así una lista
                    // se puede arrastrar agarrándola por una de sus filas.
                    Disparador::Arrastra(z) => {
                        arrastrada.is_some()
                            && match (escena.zonas.get(z.0 as usize), arrastre) {
                                (Some(zona), Some((_, o, _))) => zona.contiene(c, o.0, o.1),
                                _ => false,
                            }
                    }
                    Disparador::Mantiene { zona, durante } => {
                        let puesta = arrastre.is_some_and(|(k, _, _)| k == zona.0 as usize);
                        e.esperar(puesta, *durante, ahora, &mut citas)
                    }
                    // `on change floor(list.scroll / 34) { … }`: cuando eso cambie.
                    Disparador::Cambia(x) => {
                        let ahora = x.evaluar(c);
                        let antes = e.valia.replace(ahora);
                        antes.is_some_and(|v| (v - ahora).abs() > 0.001)
                    }
                    // `on still audio.volume for 1.1s { … }`: cuando eso lleve ese
                    // rato quieto. Cada cambio pone el reloj a cero, así que seis
                    // toques seguidos a la tecla del volumen son una sola espera.
                    Disparador::Quieta { que, durante } => {
                        let valor = que.evaluar(c);
                        let antes = e.valia.replace(valor);
                        if antes.is_some_and(|v| (v - valor).abs() > 0.001) {
                            e.armada = true;
                            e.proxima = Some(ahora + *durante);
                        }
                        match e.proxima.filter(|_| e.armada) {
                            Some(p) if ahora >= p => {
                                e.armada = false;
                                true
                            }
                            Some(p) => {
                                citas.push(p);
                                false
                            }
                            None => false,
                        }
                    }
                    Disparador::Tecla(t) => teclas.iter().any(|x| x == t),
                    Disparador::Envia(t) => enviados.contains(&(t.0 as usize)),
                    Disparador::GanaFoco => cambios_de_foco.contains(&true),
                    Disparador::PierdeFoco => cambios_de_foco.contains(&false),
                    Disparador::Recibe(z) => !soltados.is_empty() && dentro[z.0 as usize],
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
                        // Mientras la condición no se cumple, la espera NO cuenta: el
                        // reloj se pone entero cada frame, así que empieza a contar
                        // cuando empieza a ser verdad. Antes corría por su cuenta y
                        // `every 1s while cuenta` daba su primer paso a los 300 ms —lo
                        // que quedara del reloj de antes—, que en una cuenta atrás es
                        // un segundo que no existe.
                        if !mientras.es_verdad(c) {
                            e.proxima = Some(ahora + Duration::from_secs_f32(azar.entre(entre.0, entre.1)));
                            false
                        } else {
                            let toca = e.proxima.is_some_and(|p| ahora >= p);
                            if toca {
                                e.proxima = Some(ahora + Duration::from_secs_f32(azar.entre(entre.0, entre.1)));
                            }
                            if let Some(p) = e.proxima {
                                citas.push(p);
                            }
                            toca
                        }
                    }
                    Disparador::Al(_) => false, // se atienden con los sucesos, abajo
                };
                if dispara && r.si.as_ref().is_none_or(|si| si.es_verdad(c)) {
                    efectos.extend(r.efectos.iter().cloned());
                }
            }
        }

        // Efectos y sucesos, hasta que no quede ninguno (con tope: una regla
        // que se dispara a sí misma no cuelga al render).
        //
        // Y si ha pasado algo, no se duerme todavía: las reglas se miran todas
        // antes de aplicar nada, así que un `on change` no puede ver en el mismo
        // frame lo que otra regla acaba de cambiar. Lo ve al siguiente, y sin
        // esto «el siguiente» podía ser un segundo más tarde —el render dormido
        // hasta la próxima cita—, que es como una cuenta atrás se saltaba su
        // final.
        let paso_algo = !efectos.is_empty() || !sucesos.is_empty() || !gestos_pedidos.is_empty();
        let mut tope = 8;
        while (!efectos.is_empty() || !sucesos.is_empty() || !gestos_pedidos.is_empty()) && tope > 0 {
            tope -= 1;
            for ef in std::mem::take(&mut efectos) {
                match ef {
                    Efecto::Animar(t) => pendientes.push((ahora + t.retraso, t)),
                    // Un hecho que cambia una regla se le cuenta a la lógica, que si no
                    // se quedaría creyendo lo que ella misma dijo la última vez.
                    Efecto::Hecho(h, v) => {
                        let v = v.evaluar(Ctx { props: &props, hechos: &hechos });
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
                    Efecto::Impulso(p, v) => {
                        let v = v.evaluar(Ctx { props: &props, hechos: &hechos });
                        props[p.0 as usize].v += v;
                    }
                    Efecto::Gesto(g) => gestos_pedidos.push(g.0 as usize),
                    Efecto::Enfocar(t) => {
                        edicion = t.map(|t| t.0 as usize).map(|k| Edicion { campo: k, cursor: textos[k].len(), ancla: textos[k].len() });
                        ultima_tecla = ahora;
                    }
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
                    if matches!(&r.cuando, Disparador::Al(x) if *x == id) && r.si.as_ref().is_none_or(|si| si.es_verdad(Ctx { props: &props, hechos: &hechos })) {
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

        // Comportamientos: la vida propia de la escena. Lo que un gesto esté
        // moviendo ahora mismo no lo tocan: un gesto manda sobre lo ambiental.
        let de_un_gesto: Vec<PropId> = gesto
            .as_ref()
            .map(|r| escena.gestos[r.gesto].fotogramas.iter().flat_map(|f| f.valores.iter().map(|(p, _)| *p)).collect())
            .unwrap_or_default();
        let mut vivo = false;
        let mut n_parpadeo = 0;
        for comp in &escena.comportamientos {
            match comp {
                Comportamiento::Parpadeo { prop, cada, dura } => {
                    let (proximo, desde) = &mut parpadeos[n_parpadeo];
                    n_parpadeo += 1;
                    // Mientras un gesto lleve esta misma pose de la mano, lo ambiental
                    // calla: el gesto ya dice qué hacen los párpados. Y su reloj se
                    // para con él, para que al acabar no dispare de golpe lo que le
                    // tocaba a mitad del gesto.
                    if de_un_gesto.contains(prop) {
                        *proximo += Duration::from_secs_f32(dt);
                        *desde = None;
                        continue;
                    }
                    // Con movimiento reducido lo ambiental calla: un parpadeo se
                    // queda con el ojo abierto. Los muelles se posan y los gestos
                    // enseñan su cara quieta; lo que va solo y en bucle es justo
                    // lo que no debe seguir dando vueltas.
                    if op.reducido {
                        *desde = None;
                        props[prop.0 as usize].fijar(1.0);
                        continue;
                    }
                    if desde.is_none() && ahora >= *proximo {
                        *desde = Some(ahora);
                    }
                    if let Some(d) = *desde {
                        let t = (ahora - d).as_secs_f32() / dura;
                        if t >= 1.0 {
                            *desde = None;
                            // El periodo se cuenta de comienzo a comienzo: «cada 5,2 s» es
                            // eso, no 5,2 s **después** de cerrar el ojo.
                            let siguiente = d + Duration::from_secs_f32(azar.entre(cada.0, cada.1));
                            *proximo = siguiente.max(ahora);
                            props[prop.0 as usize].fijar(1.0);
                        } else {
                            props[prop.0 as usize].fijar((2.0 * t - 1.0).abs().powf(1.6));
                            vivo = true;
                        }
                    }
                    citas.push(*proximo);
                }
                Comportamiento::Onda { prop, frecuencia, amplitud } => {
                    if de_un_gesto.contains(prop) {
                        continue;
                    }
                    // Y una onda se queda en su descanso —la mitad del viaje—, que
                    // es donde estaba al empezar.
                    if op.reducido {
                        props[prop.0 as usize].fijar(0.0);
                        continue;
                    }
                    let a = amplitud.evaluar(Ctx { props: &props, hechos: &hechos });
                    props[prop.0 as usize].fijar(a * (t_total * frecuencia).sin());
                    vivo |= a.abs() > 0.01;
                }
                Comportamiento::Sigue { prop, a } => {
                    let v = a.evaluar(Ctx { props: &props, hechos: &hechos });
                    props[prop.0 as usize].objetivo = v;
                }
                Comportamiento::Es { prop, a } => {
                    let v = a.evaluar(Ctx { props: &props, hechos: &hechos });
                    let p = &mut props[prop.0 as usize];
                    vivo |= (p.x - v).abs() > 1e-3;
                    p.fijar(v);
                }
                Comportamiento::Avance { prop, por_segundo } => {
                    // Un giro no tiene descanso al que volver: se queda donde va.
                    if op.reducido {
                        continue;
                    }
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

        // Las emergentes: abiertas mientras su hecho sea verdad, donde y como digan
        // sus expresiones. Si cambian de sitio o de tamaño estando abiertas, se rehacen.
        emergentes.resize(escena.emergentes.len(), None);
        // Lo que se dibuja existe si cae en alguna superficie viva, o en alguna emergente
        // abierta. Una superficie cerrada no aporta la suya: así todo lo suyo se descarta al
        // componer y su siguiente frame sale vacío, que es lo que la hace desaparecer.
        let abierta = |k: usize| escena.superficies.get(k).and_then(|s| s.abierta.as_ref()).is_none_or(|e| e.es_verdad(Ctx { props: &props, hechos: &hechos }));
        dibujo.vistas.clear();
        dibujo.vistas.extend(laminas.iter().filter(|l| abierta(l.vista.superficie)).map(|l| l.vista.caja()));
        for (k, em) in escena.emergentes.iter().enumerate() {
            let c = Ctx { props: &props, hechos: &hechos };
            let quiere = (hechos[em.abierta.0 as usize] > 0.5 && !laminas.is_empty()).then(|| {
                [em.en.0.evaluar(c).round() as i32, em.en.1.evaluar(c).round() as i32, em.tam.0.evaluar(c).round().max(1.0) as i32, em.tam.1.evaluar(c).round().max(1.0) as i32]
            });
            if quiere != emergentes[k] {
                if emergentes[k].is_some() {
                    laminas.retain(|l| l.vista.emergente != Some(k));
                    crate::plataforma::emergente(k, None);
                    if let Some(g) = &gpu {
                        repartir_el_ritmo(g, &mut laminas, tam, op.sin_vsync);
                    }
                }
                if let Some(g) = quiere {
                    crate::plataforma::emergente(k, Some((g, em.origen)));
                }
                emergentes[k] = quiere;
            }
            if let Some(g) = emergentes[k] {
                dibujo.vistas.push([em.origen.0, em.origen.1, em.origen.0 + g[2] as f32, em.origen.1 + g[3] as f32]);
            }
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
        // El cursor de texto parpadea: medio segundo sí, medio no, y siempre sí justo
        // después de teclear. Entre parpadeos no hace falta pintar.
        let vista = edicion.as_ref().map(|e| {
            let t = (ahora - ultima_tecla).as_secs_f32();
            citas.push(ahora + Duration::from_secs_f32(0.53 - t % 0.53 + 0.001));
            crate::gpu::VistaDeCampo { texto: e.campo, cursor: e.cursor, ancla: e.ancla, se_ve: (t % 1.06) < 0.53 }
        });
        if let Some((_, _, _, cuando)) = &repite {
            citas.push(*cuando);
        }
        let a_pintar: &[Instr] = if aviso.is_some() { &con_aviso } else { &escena.instrs };
        dibujo.pegada_a(escena.superficie().ancla.pegada());
        let leyendo = Instant::now();
        dibujo.componer(a_pintar, c, &textos, &mut letras, vista, tam, op.hud);
        ciclo.componer += leyendo.elapsed().as_secs_f32() * 1000.0;
        let Some(g) = &mut gpu else {
            // Aún no hay dónde: el tiempo corre igual, pero sin prisa.
            std::thread::sleep(Duration::from_millis(8));
            continue;
        };
        g.subir_atlas(&mut letras.por_subir);
        g.subir(&dibujo);

        // Por dónde entra el ratón: las zonas activas, y nada más. Lo demás de
        // la superficie es transparente también para el clic.
        // De una superficie cerrada no se puede pulsar nada.
        let cerradas: Vec<[f32; 4]> = escena
            .superficies
            .iter()
            .filter(|s| s.abierta.as_ref().is_some_and(|e| !e.es_verdad(c)))
            .map(|s| [s.origen.0, s.origen.1, s.origen.0 + s.ancho.max(1) as f32, s.origen.1 + s.alto as f32])
            .collect();
        let cajas: Vec<[i32; 4]> = escena
            .zonas
            .iter()
            .filter(|z| z.activa.es_verdad(c))
            .filter_map(|z| z.caja(c))
            .filter(|b| !cerradas.iter().any(|v| b[0] < v[2] && b[2] > v[0] && b[1] < v[3] && b[3] > v[1]))
            .map(|b| [(b[0] - 3.0).floor() as i32, (b[1] - 3.0).floor() as i32, (b[2] + 3.0).ceil() as i32, (b[3] + 3.0).ceil() as i32])
            .collect();
        let cambia_la_region = cajas != region;
        if cambia_la_region {
            // Cada superficie recibe las zonas que caen en SU trozo del plano, en sus
            // coordenadas. Una emergente es toda suya, y no lleva región.
            for l in laminas.iter().filter(|l| l.vista.emergente.is_none()) {
                let v = l.vista.caja();
                let (dx, dy) = (l.vista.origen.0 as i32, l.vista.origen.1 as i32);
                let suyas: Vec<[i32; 4]> = cajas
                    .iter()
                    .filter(|b| (b[0] as f32) < v[2] && (b[2] as f32) > v[0] && (b[1] as f32) < v[3] && (b[3] as f32) > v[1])
                    .map(|b| [b[0] - dx, b[1] - dy, b[2] - dx, b[3] - dy])
                    .collect();
                l.region_de_entrada(&suyas);
            }
            region = cajas;
        }

        // El 4 y el 7 son el origen de la vista: cero en la principal; cada emergente pone el suyo.
        uniformes[..8].copy_from_slice(&[tam.0, tam.1, t_total, 1.0, 0.0, periodo_ms, if bloqueada { 1.0 } else { 0.0 }, 0.0]);
        uniformes[8..].copy_from_slice(&historial);
        // La que marca el ritmo va la última: es la que espera a la pantalla.
        laminas.sort_by_key(|l| l.marca_el_ritmo);
        // ¿Alguna superficie ha decidido pegarse a otro borde? La esquina que
        // elige la cara de grabar de Marea es un hecho, y layer-shell deja
        // cambiarla sin volver a crear nada.
        if anclas_puestas.len() != escena.superficies.len() {
            anclas_puestas = escena.superficies.iter().map(|s| s.ancla).collect();
        }
        for (k, sup) in escena.superficies.iter().enumerate() {
            let Some((hecho, anclas)) = &sup.ancla_de else { continue };
            let quiere = anclas.get(hechos[hecho.0 as usize].round().max(0.0) as usize).copied();
            if let Some(a) = quiere.filter(|a| anclas_puestas[k] != *a) {
                anclas_puestas[k] = a;
                crate::plataforma::anclar(k, a);
            }
        }

        // Quién está abierta ahora. Si cambia, se reparte el ritmo otra vez: una
        // superficie cerrada que lo marcase esperaría con vsync un frame que el
        // compositor no le va a dar —a lo que no se ve no se le dan—, y con ella
        // se paraba todo: 300 ms en mitad de una animación de otra ventana.
        let mut cambio_de_abiertas = false;
        for l in &mut laminas {
            let ab = l.vista.emergente.is_some() || abierta(l.vista.superficie);
            cambio_de_abiertas |= ab != l.abierta;
            if ab {
                l.vaciada = false;
            }
            l.abierta = ab;
        }
        if cambio_de_abiertas {
            repartir_el_ritmo(g, &mut laminas, tam, op.sin_vsync);
            laminas.sort_by_key(|l| l.marca_el_ritmo);
        }
        // El paso. Con buzón lo marca el render: un plazo absoluto por periodo del
        // monitor que marca el ritmo, para que el error de cada espera no se
        // acumule; si se llega tarde —un reposo, un frame largo— se empieza de
        // nuevo desde ahora en vez de correr a alcanzarlo. Con vsync de cola lo
        // marca la pantalla, pero solo cuando su cola está llena: al despertar
        // aceptaba dos o tres frames sin esperar y el bucle los presentaba en dos
        // milisegundos; ahí, nunca más de un frame por periodo.
        if !op.sin_vsync && !op.ingenuo {
            let ahora_mismo = Instant::now();
            if g.con_buzon() {
                let mhz = laminas.iter().find(|l| l.marca_el_ritmo).map_or(60_000, |l| l.mhz.max(1));
                let periodo = Duration::from_secs_f64(1000.0 / mhz as f64);
                if proximo_frame > ahora_mismo {
                    std::thread::sleep(proximo_frame - ahora_mismo);
                    proximo_frame += periodo;
                } else {
                    proximo_frame = ahora_mismo + periodo;
                }
            } else {
                let minimo = Duration::from_secs_f32(periodo_ms * 0.8 / 1000.0);
                let desde = ahora_mismo - ultimo_presentado;
                if desde < minimo {
                    std::thread::sleep(minimo - desde);
                }
            }
        }
        ultimo_presentado = Instant::now();
        let mut pintadas = 0;
        for l in &mut laminas {
            // Cerrada se pinta una vez, vacía, y ya.
            if !l.abierta && std::mem::replace(&mut l.vaciada, true) {
                continue;
            }
            pintadas += g.pintar(l, &dibujo, &uniformes) as u32;
        }
        if pintadas == 0 {
            std::thread::sleep(Duration::from_millis(8));
        } else if primer_frame {
            primer_frame = false;
            println!("render · first frame {} ms after starting", op.arranque.elapsed().as_millis());
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
        //  Lo que se pidió apuntar, frame a frame: es como se comprueba que una
        //  animación dura lo que su contrato dice que dura.
        if !op.registrar.is_empty() {
            if !registro_dicho {
                registro_dicho = true;
                // Un nombre que no existe se apuntaba como «?» durante toda la
                // medición, y no había manera de saber si estaba mal escrito o
                // es que la escena no lo movía. Ahora se dice, una vez, con los
                // que se le parecen: dentro de una copia los nombres llevan su
                // marca (`px#Hat1`), y eso no hay quien lo adivine.
                for n in &op.registrar {
                    let n = n.as_str();
                    let hay = escena.props.iter().any(|p| p.0 == n) || escena.hechos.iter().any(|h| h.0 == n) || escena.textos.iter().any(|t| t.0 == n);
                    if !hay {
                        let mut cerca: Vec<&str> = escena
                            .props
                            .iter()
                            .map(|p| p.0)
                            .chain(escena.hechos.iter().map(|h| h.0))
                            .chain(escena.textos.iter().map(|t| t.0))
                            .filter(|c| c.contains(n) || n.contains(*c) || c.split('#').next() == Some(n))
                            .collect();
                        cerca.sort_unstable();
                        cerca.dedup();
                        cerca.truncate(6);
                        let pista = if cerca.is_empty() { String::new() } else { format!(" · there is {}", cerca.join(", ")) };
                        println!("record · there is nothing called '{n}'{pista}");
                    }
                }
                println!("ms\t{}", op.registrar.join("\t"));
            }
            let valores: Vec<String> = op
                .registrar
                .iter()
                .map(|n| {
                    let n = n.as_str();
                    if let Some(i) = escena.props.iter().position(|p| p.0 == n) {
                        format!("{:.4}", props[i].x)
                    } else if let Some(i) = escena.hechos.iter().position(|h| h.0 == n) {
                        format!("{:.4}", hechos[i])
                    } else if let Some(i) = escena.textos.iter().position(|t| t.0 == n) {
                        textos.get(i).cloned().unwrap_or_default()
                    } else {
                        "?".into()
                    }
                })
                .collect();
            println!("{:.1}\t{}", t_total * 1000.0, valores.join("\t"));
        }
        let ms = dt * 1000.0;
        // Un chivato permanente: cualquier frame que se pase de dos periodos, con su hora.
        if ms > periodo_ms * 2.4 && !primer_frame && ciclo.dts.len() > 1 && !op.sin_vsync && !op.ingenuo {
            println!("render · slow frame: {ms:.0} ms at {t_total:.2} s");
        }
        historial.copy_within(1.., 0);
        historial[119] = if bloqueada { -ms } else { ms };
        ciclo.dts.push(ms);
        if !op.sin_vsync && !op.ingenuo && !bloqueada {
            // El periodo APRENDIDO no vale aquí: una escena que siempre va
            // tarde le enseña que la pantalla da 28 ms y entonces nunca llega
            // tarde. Se compara con el refresco de verdad del monitor.
            let de_verdad = laminas.iter().find(|l| l.marca_el_ritmo).map_or(16.7, |l| 1_000_000.0 / l.mhz.max(1) as f32);
            ciclo.vigilar(de_verdad);
        }
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
        if !op.sin_vsync && !vivo && !paso_algo && !bloqueada && !cambia_la_region && !cambio_de_medida && !cambio_de_teclado && props.iter().all(Animada::quieta) {
            for a in &mut props {
                a.posar();
            }
            ciclo.cerrar();
            proxima_cita = citas.iter().min().copied();
            en_reposo = true;
            // Quieta ya: si algo se salía, esto es lo que se queda mirando.
            dibujo.decir_lo_pendiente();
        }
    }
}

/// La banda que dice que lo que acabas de guardar no compila, con el fichero, la
/// línea y el mensaje. Va arriba de la superficie principal, que es donde estás
/// mirando, y se quita sola cuando la escena vuelve a estar bien. Debajo sigue
/// corriendo la última escena buena: esto no para nada, solo lo cuenta.
fn banda_de_fallo(mensaje: &str, ancho: f32) -> Vec<Instr> {
    let ancho = if ancho > 0.0 { ancho } else { 900.0 };
    // La primera línea es la que dice qué pasa; el resto es el dedo señalando.
    let dicho = mensaje.lines().next().unwrap_or(mensaje).trim().to_string();
    let alto = 52.0;
    let caja = |x: f32, w: f32, r: f32| Forma::Caja {
        centro: ((x + w * 0.5).into(), (alto * 0.5).into()),
        mitad: ((w * 0.5).into(), (alto * 0.5).into()),
        radio: r.into(),
    };
    vec![
        Instr::Grupo { sombra: Some(Sombra { desplazada: (0.0.into(), 6.0.into()), difusa: 18.0.into(), alfa: 0.45.into(), color: None }) },
        Instr::Forma { forma: caja(8.0, ancho - 16.0, 10.0), fusion: 0.0.into() },
        Instr::Relleno { pintura: color(0.18, 0.05, 0.06).into(), alfa: 1.0.into(), filo: 0.05, luz: None, borde: None },
        // Una pestaña del color de los errores, a la izquierda: se lee antes que el texto.
        Instr::Grupo { sombra: None },
        Instr::Forma { forma: caja(8.0, 5.0, 2.5), fusion: 0.0.into() },
        Instr::Relleno { pintura: color(0.99, 0.41, 0.33).into(), alfa: 1.0.into(), filo: 0.05, luz: None, borde: None },
        Instr::Texto {
            contenido: Contenido::Fijo("this does not compile — the last good scene is still running".into()),
            en: (26.0.into(), 17.0.into()),
            ancla: (0.0, 0.5),
            ancho: Some((ancho - 52.0).into()),
            estilo: Estilo::de(10.5, color(0.99, 0.41, 0.33)).peso(600).lineas(1),
            alfa: 1.0.into(),
            mide: None,
        },
        Instr::Texto {
            contenido: Contenido::Fijo(dicho),
            en: (26.0.into(), 34.0.into()),
            ancla: (0.0, 0.5),
            ancho: Some((ancho - 52.0).into()),
            estilo: Estilo::de(12.5, color(0.96, 0.96, 0.96)).lineas(1),
            alfa: 1.0.into(),
            mide: None,
        },
    ]
}

/// Una sola lámina espera a su pantalla —la del monitor más rápido— y las demás
/// presentan sin bloquear. Si esperasen todas, un monitor a 60 Hz frenaría a
/// otro a 165.
fn repartir_el_ritmo(g: &Gpu, laminas: &mut [Lamina], tam: (f32, f32), sin_vsync: bool) {
    // Una emergente nunca marca el ritmo: viene y va, y puede estar tapada.
    // Ni una cerrada. Y entre iguales, la primera: `max_by_key` se queda con la
    // última, que con dos superficies a 60 Hz era la de la esquina, cerrada.
    let mut rapida: Option<(i32, _)> = None;
    for l in laminas.iter().filter(|l| l.vista.emergente.is_none() && l.abierta) {
        if rapida.is_none_or(|(mhz, _)| l.mhz > mhz) {
            rapida = Some((l.mhz, l.id));
        }
    }
    let rapida = rapida.map(|(_, id)| id);
    for l in laminas.iter_mut() {
        let marca = !sin_vsync && Some(l.id) == rapida;
        if l.marca_el_ritmo != marca {
            l.marca_el_ritmo = marca;
            g.configurar(l, tam);
        }
    }
}

/// El campo donde se escribe: qué texto, dónde está el cursor y desde dónde se
/// seleccionó. En bytes, siempre en el borde de una letra.
struct Edicion {
    campo: usize,
    cursor: usize,
    ancla: usize,
}

enum Tecleo {
    Cambio,
    Movio,
    Envio,
    NoEsMio,
}

impl Edicion {
    fn seleccion(&self) -> (usize, usize) {
        (self.cursor.min(self.ancla), self.cursor.max(self.ancla))
    }

    fn borrar_seleccion(&mut self, t: &mut String) -> bool {
        let (a, b) = self.seleccion();
        t.replace_range(a..b, "");
        self.cursor = a;
        self.ancla = a;
        b > a
    }

    fn tecla(&mut self, t: &mut String, nombre: &str, escribe: Option<&str>, m: Mods) -> Tecleo {
        self.cursor = self.cursor.min(t.len());
        self.ancla = self.ancla.min(t.len());
        let antes = |t: &str, b: usize| t[..b].char_indices().next_back().map_or(0, |c| c.0);
        let despues = |t: &str, b: usize| t[b..].chars().next().map_or(t.len(), |c| b + c.len_utf8());
        let mover = |yo: &mut Self, a: usize| {
            yo.cursor = a;
            if !m.mayus {
                yo.ancla = a;
            }
            Tecleo::Movio
        };
        match nombre {
            "a" if m.ctrl => {
                self.ancla = 0;
                self.cursor = t.len();
                Tecleo::Movio
            }
            "c" | "x" if m.ctrl => {
                let (a, b) = self.seleccion();
                if b > a {
                    crate::plataforma::portapapeles_escribir(&t[a..b]);
                }
                if nombre == "x" && self.borrar_seleccion(t) { Tecleo::Cambio } else { Tecleo::Movio }
            }
            "v" if m.ctrl => {
                let Some(pegado) = crate::plataforma::portapapeles_leer() else { return Tecleo::Movio };
                // Un campo es de una línea: lo que venga con saltos, sin ellos.
                let pegado: String = pegado.chars().filter(|c| !c.is_control()).collect();
                self.borrar_seleccion(t);
                t.insert_str(self.cursor, &pegado);
                self.cursor += pegado.len();
                self.ancla = self.cursor;
                Tecleo::Cambio
            }
            "BackSpace" | "Delete" => {
                if !self.borrar_seleccion(t) {
                    let (a, b) = if nombre == "BackSpace" { (antes(t, self.cursor), self.cursor) } else { (self.cursor, despues(t, self.cursor)) };
                    t.replace_range(a..b, "");
                    self.cursor = a;
                    self.ancla = a;
                }
                Tecleo::Cambio
            }
            "Left" => {
                let a = antes(t, self.cursor);
                mover(self, a)
            }
            "Right" => {
                let a = despues(t, self.cursor);
                mover(self, a)
            }
            "Home" => mover(self, 0),
            "End" => {
                let a = t.len();
                mover(self, a)
            }
            "Return" | "KP_Enter" => Tecleo::Envio,
            _ => match escribe {
                Some(letras) if !m.ctrl && !m.alt && !m.logo => {
                    self.borrar_seleccion(t);
                    t.insert_str(self.cursor, letras);
                    self.cursor += letras.len();
                    self.ancla = self.cursor;
                    Tecleo::Cambio
                }
                _ => Tecleo::NoEsMio,
            },
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
    /// Lo que valía la última vez la cuenta de un `on change`.
    valia: Option<f32>,
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
