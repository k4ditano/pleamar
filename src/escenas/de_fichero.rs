//! Una escena que viene de un fichero del lenguaje, y que se recarga sola
//! cuando el fichero cambia. La lógica que la acompaña es la mínima: escucha.

use crate::escena::*;
use crate::logica::{Contexto, Guion};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// Lee y levanta una escena. El fallo viene ya con su fichero, su línea y su flecha.
pub fn leer(ruta: &str) -> Result<Escena, String> {
    crate::lenguaje::leer_fichero(ruta).map(|(e, _)| e)
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
            Evento::Texto(n, v) => println!("logic  · '{n}' now says: {v:?}"),
            Evento::Envia(n, v) => println!("logic  · enter in '{n}': {v:?}"),
            Evento::Recibido(z, tipo, d) => println!("logic  · something was dropped on '{z}', a {tipo}: {d:?}"),
            Evento::Suceso(s, None) => println!("logic  · '{s}' happened"),
            Evento::Suceso(s, Some(v)) => println!("logic  · '{s}' happened, with {v:.2}"),
            _ => {}
        }
    }
}

/// El guion de una escena de fichero: si a su lado hay un `.luau` con el mismo
/// nombre, esa es su lógica; si no, una que solo escucha.
pub fn guion_para(ruta: &str, tx: Sender<ARender>, a_logica: Sender<Evento>, bloqueada: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Box<dyn Guion> {
    let logica = logica_de(ruta);
    // Hay lógica si la escena tiene su `.luau`, o si alguno de sus plugins tiene el suyo.
    #[cfg(feature = "luau")]
    if std::path::Path::new(&logica).is_file() || leer(ruta).is_ok_and(|e| !e.plugins.is_empty()) {
        return Box::new(crate::logica_luau::GuionLuau::nuevo(ruta, &logica, tx, a_logica, bloqueada));
    }
    let _ = (tx, a_logica, bloqueada, logica);
    Box::new(DeFichero::nueva(ruta))
}

fn logica_de(ruta: &str) -> String {
    std::path::Path::new(ruta).with_extension("luau").to_string_lossy().into_owned()
}

/// Recarga en caliente: mira la fecha del fichero cuatro veces por segundo —que
/// funciona igual en cualquier sistema— y, si cambió, lo vuelve a leer. Si está
/// bien, la escena nueva sustituye a la vieja sin perder lo que se movía; si no,
/// dice por qué y la vieja sigue.
pub fn vigilar(ruta: String, al_render: Sender<ARender>, a_logica: Sender<Evento>) {
    // La lógica también se recarga: mismo truco, otro fichero.
    let logica = logica_de(&ruta);
    let a_la_logica = a_logica.clone();
    let escena_de_la_logica = ruta.clone();
    std::thread::Builder::new()
        .name("reload-logic".into())
        .spawn(move || {
            // La de la escena y la de sus plugins: tocar cualquiera las recarga.
            let todas = |escena: &str| -> Vec<String> {
                let mut v = vec![logica.clone()];
                if let Ok(e) = leer(escena) {
                    v.extend(e.plugins.iter().map(|p| p.logica.to_string_lossy().into_owned()));
                }
                v
            };
            let fecha = |v: &[String]| v.iter().map(|r| std::fs::metadata(r).and_then(|m| m.modified()).ok()).collect::<Vec<_>>();
            let mut vigiladas = todas(&escena_de_la_logica);
            let mut ultima = fecha(&vigiladas);
            loop {
                std::thread::sleep(Duration::from_millis(250));
                let ahora = fecha(&vigiladas);
                if ahora != ultima && ahora.iter().any(Option::is_some) {
                    std::thread::sleep(Duration::from_millis(80));
                    vigiladas = todas(&escena_de_la_logica);
                    ultima = fecha(&vigiladas);
                    if a_logica.send(Evento::RecargarLogica).is_err() {
                        return;
                    }
                }
            }
        })
        .unwrap();
    std::thread::Builder::new()
        .name("recarga".into())
        .spawn(move || {
            // La escena y lo que importe: tocar una biblioteca también la recarga.
            let mut vigilados: Vec<std::path::PathBuf> = crate::lenguaje::leer_fichero(&ruta).map_or_else(|_| vec![ruta.clone().into()], |(_, f)| f);
            let fecha = |v: &[std::path::PathBuf]| v.iter().map(|r| std::fs::metadata(r).and_then(|m| m.modified()).ok()).collect::<Vec<_>>();
            let mut ultima = fecha(&vigilados);
            loop {
                std::thread::sleep(Duration::from_millis(250));
                let mut ahora = fecha(&vigilados);
                if ahora == ultima || ahora.iter().all(Option::is_none) {
                    continue;
                }
                // Un editor guarda en varios tiempos —trunca, escribe, a veces renombra—:
                // se espera a que el fichero lleve un momento quieto y no esté vacío.
                loop {
                    std::thread::sleep(Duration::from_millis(60));
                    let despues = fecha(&vigilados);
                    let vacio = vigilados.iter().any(|r| std::fs::metadata(r).map_or(true, |m| m.len() == 0));
                    if despues == ahora && !vacio {
                        break;
                    }
                    ahora = despues;
                }
                let t0 = std::time::Instant::now();
                match crate::lenguaje::leer_fichero(&ruta) {
                    Ok((e, ficheros)) => {
                        println!("reload · {ruta} read in {:.1} ms{}", t0.elapsed().as_secs_f32() * 1000.0, if ficheros.len() > 1 { format!(" · with {} libraries", ficheros.len() - 1) } else { String::new() });
                        // Puede que ahora importe otras cosas.
                        vigilados = ficheros;
                        let _ = a_la_logica.send(Evento::EscenaNueva(e.hechos.clone(), e.textos.clone(), e.permisos.clone(), e.modelos.clone(), e.tipos.clone(), e.plugins.clone(), e.sucesos.iter().map(|s| s.0).collect()));
                        if al_render.send(ARender::Escena(e)).is_err() {
                            return;
                        }
                    }
                    Err(m) => eprintln!("reload · the scene stays as it was:\n{m}"),
                }
                ultima = fecha(&vigilados);
            }
        })
        .unwrap();
}
