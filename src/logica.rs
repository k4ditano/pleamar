//! El hilo que piensa. Aquí corre un guion: recibe eventos con nombre
//! («entra en orbe», «pulsa ver»), decide y declara transiciones. No anima
//! nada ni sabe de coordenadas.
//!
//! Después de cada decisión el guion puede «trabajar» —bloquearse a propósito—,
//! que es lo que hace una shell de verdad al abrir un panel: instanciar, leer,
//! parsear.

use crate::escena::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Opciones {
    pub bloqueo: Duration,
    pub demo: bool,
    pub ingenuo: bool,
    /// Contar por la salida cada evento que llega.
    pub eco: bool,
}

pub trait Guion: Send {
    fn escena(&mut self) -> Escena;
    fn evento(&mut self, e: Evento, c: &mut Contexto);
    /// Cuándo quiere que le despierten, si tiene algo a plazo.
    fn proxima(&self) -> Option<Instant> {
        None
    }
    /// Le ha llegado la hora.
    fn tic(&mut self, _: &mut Contexto) {}
}

pub struct Contexto {
    tx: Sender<ARender>,
    bloqueada: Arc<AtomicBool>,
    op: Opciones,
    alarmas: Vec<(Instant, &'static str)>,
}

impl Contexto {
    /// El de la lógica de un plugin, que corre en su propio hilo: ni demo, ni eco, ni bloqueo de ensayo.
    pub fn de_plugin(tx: Sender<ARender>, bloqueada: Arc<AtomicBool>) -> Contexto {
        Contexto { tx, bloqueada, op: Opciones { bloqueo: Duration::ZERO, demo: false, ingenuo: false, eco: false }, alarmas: Vec::new() }
    }
}

#[allow(dead_code)] // animar e impulso siguen ahí para guiones que aún manden intenciones sueltas
impl Contexto {
    pub fn animar(&self, prop: PropId, a: f32, muelle: Muelle, retraso_ms: u64) {
        let t = Transicion { prop, a: a.into(), muelle, retraso: Duration::from_millis(retraso_ms) };
        let _ = self.tx.send(ARender::Orden(Orden::Animar(t)));
    }

    /// La frontera con la escena: contar lo que es verdad…
    pub fn hecho(&self, nombre: &'static str, valor: bool) {
        let _ = self.tx.send(ARender::Hecho(nombre, valor as u8 as f32));
    }

    /// …cambiar lo que dice un texto…
    pub fn texto(&self, nombre: &'static str, valor: impl Into<String>) {
        let _ = self.tx.send(ARender::Texto(nombre, valor.into()));
    }

    /// …lo que acaba de pasar…
    pub fn suceso(&self, nombre: &'static str) {
        let _ = self.tx.send(ARender::Suceso(nombre));
    }

    /// …y pedir un gesto, que la escena concederá o no según su clase.
    pub fn gesto(&self, nombre: &'static str) {
        let _ = self.tx.send(ARender::Gesto(nombre));
    }

    pub fn impulso(&self, prop: PropId, velocidad: f32) {
        let _ = self.tx.send(ARender::Orden(Orden::Impulso { prop, velocidad }));
    }

    /// Una alarma con nombre; ponerla otra vez la retrasa.
    pub fn alarma(&mut self, nombre: &'static str, ms: u64) {
        self.cancelar(nombre);
        self.alarmas.push((Instant::now() + Duration::from_millis(ms), nombre));
    }

    pub fn cancelar(&mut self, nombre: &'static str) {
        self.alarmas.retain(|(_, n)| *n != nombre);
    }

    /// El trabajo pesado. En el modo ingenuo se lo endosa al hilo de render,
    /// porque ahí «lógica» y «pintado» son el mismo hilo.
    pub fn trabajar(&self) {
        self.trabajar_durante(self.op.bloqueo);
    }

    pub fn trabajar_durante(&self, cuanto: Duration) {
        if cuanto.is_zero() {
            return;
        }
        if self.op.ingenuo {
            let _ = self.tx.send(ARender::Orden(Orden::Bloquear(cuanto)));
            std::thread::sleep(cuanto);
            return;
        }
        self.bloqueada.store(true, Ordering::Relaxed);
        let fin = Instant::now() + cuanto;
        // Espera activa: un núcleo al 100 %, como un bucle de JS que no cede.
        while Instant::now() < fin {
            std::hint::spin_loop();
        }
        self.bloqueada.store(false, Ordering::Relaxed);
    }
}

pub fn hilo(mut guion: Box<dyn Guion>, rx: Receiver<Evento>, tx: Sender<ARender>, bloqueada: Arc<AtomicBool>, op: Opciones) {
    let demo = op.demo;
    let mut c = Contexto { tx, bloqueada, op, alarmas: Vec::new() };
    let mut siguiente_demo = Instant::now() + Duration::from_millis(1200);
    guion.evento(Evento::Alarma("inicio"), &mut c);

    loop {
        let mut hasta = Instant::now() + Duration::from_secs(3600);
        if demo {
            hasta = hasta.min(siguiente_demo);
        }
        for (cuando, _) in &c.alarmas {
            hasta = hasta.min(*cuando);
        }
        if let Some(p) = guion.proxima() {
            hasta = hasta.min(p);
        }
        match rx.recv_timeout(hasta.saturating_duration_since(Instant::now())) {
            // En la demo manda el reloj, no el ratón.
            Ok(e) if !(demo && matches!(e, Evento::Entra(_) | Evento::Sale(_) | Evento::Pulsa(_))) => {
                if c.op.eco {
                    // El eco del ratón de mentira cuenta lo que llega, no lo
                    // vuelca: la salida de un `run` pueden ser ochocientos
                    // kilobytes de rutas de tu casa, y eso ni se lee ni se
                    // quiere en un log.
                    // Y lo que se teclea en un campo secreto no se cuenta: ni el
                    // texto, ni las teclas mientras haya alguno en la escena.
                    let secretos = crate::escena::SECRETOS.lock().unwrap();
                    let dicho = match &e {
                        Evento::Texto(n, _) if secretos.contains(n) => format!("Texto({n:?}, «…»)"),
                        Evento::Envia(n, _) if secretos.contains(n) => format!("Envia({n:?}, «…»)"),
                        Evento::Tecla(..) if !secretos.is_empty() => "Tecla(«…»)".to_owned(),
                        _ => format!("{e:?}"),
                    };
                    drop(secretos);
                    match dicho.char_indices().nth(160) {
                        Some((k, _)) => println!("logic  · {}… ({} caracteres)", &dicho[..k], dicho.chars().count()),
                        None => println!("logic  · {dicho}"),
                    }
                }
                guion.evento(e, &mut c)
            }
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let ahora = Instant::now();
        let vencidas: Vec<&'static str> = c.alarmas.iter().filter(|(t, _)| *t <= ahora).map(|(_, n)| *n).collect();
        c.alarmas.retain(|(t, _)| *t > ahora);
        for n in vencidas {
            guion.evento(Evento::Alarma(n), &mut c);
        }
        if guion.proxima().is_some_and(|p| p <= Instant::now()) {
            guion.tic(&mut c);
        }
        if demo && ahora >= siguiente_demo {
            guion.evento(Evento::Demo, &mut c);
            siguiente_demo = Instant::now() + Duration::from_millis(2600);
        }
    }
}
