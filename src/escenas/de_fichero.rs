//! Una escena que viene de un fichero del lenguaje, y que se recarga sola
//! cuando el fichero cambia. La lógica que la acompaña es la mínima: escucha.

use crate::escena::*;
use crate::logica::{Contexto, Guion};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// Lee y levanta una escena. El fallo viene ya con su línea y su flecha.
pub fn leer(ruta: &str) -> Result<Escena, String> {
    let fuente = std::fs::read_to_string(ruta).map_err(|e| format!("{ruta}: {e}"))?;
    crate::lenguaje::leer(&fuente).map_err(|f| f.con_fuente(ruta, &fuente))
}

pub struct DeFichero {
    ruta: String,
}

impl DeFichero {
    pub fn nueva(ruta: &str) -> Self {
        DeFichero { ruta: ruta.to_owned() }
    }
}

impl Guion for DeFichero {
    fn escena(&mut self) -> Escena {
        leer(&self.ruta).unwrap_or_else(|m| {
            eprintln!("{m}");
            std::process::exit(1)
        })
    }

    fn evento(&mut self, e: Evento, c: &mut Contexto) {
        match e {
            // Sin ratón, `--demo` dispara el suceso «demo»: la escena dirá qué hace con él.
            Evento::Demo => c.suceso("demo"),
            Evento::Capa(_, _) => c.trabajar(),
            Evento::Suceso(s) => println!("lógica · ha pasado «{s}»"),
            _ => {}
        }
    }
}

/// Recarga en caliente: mira la fecha del fichero cuatro veces por segundo —que
/// funciona igual en cualquier sistema— y, si cambió, lo vuelve a leer. Si está
/// bien, la escena nueva sustituye a la vieja sin perder lo que se movía; si no,
/// dice por qué y la vieja sigue.
pub fn vigilar(ruta: String, al_render: Sender<ARender>) {
    std::thread::Builder::new()
        .name("recarga".into())
        .spawn(move || {
            let fecha = |r: &str| std::fs::metadata(r).and_then(|m| m.modified()).ok();
            let mut ultima = fecha(&ruta);
            loop {
                std::thread::sleep(Duration::from_millis(250));
                let mut ahora = fecha(&ruta);
                if ahora == ultima || ahora.is_none() {
                    continue;
                }
                // Un editor guarda en varios tiempos —trunca, escribe, a veces renombra—:
                // se espera a que el fichero lleve un momento quieto y no esté vacío.
                loop {
                    std::thread::sleep(Duration::from_millis(60));
                    let despues = fecha(&ruta);
                    let vacio = std::fs::metadata(&ruta).map_or(true, |m| m.len() == 0);
                    if despues == ahora && !vacio {
                        break;
                    }
                    ahora = despues;
                }
                ultima = ahora;
                let t0 = std::time::Instant::now();
                match leer(&ruta) {
                    Ok(e) => {
                        println!("recarga · {ruta} leída en {:.1} ms", t0.elapsed().as_secs_f32() * 1000.0);
                        if al_render.send(ARender::Escena(e)).is_err() {
                            return;
                        }
                    }
                    Err(m) => eprintln!("recarga · la escena sigue como estaba:\n{m}"),
                }
            }
        })
        .unwrap();
}
