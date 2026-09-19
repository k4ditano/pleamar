//! pleamar — prototipo de una shell donde animar no depende de la lógica.
//!
//! Tres hilos: este, que se lo queda la plataforma (las ventanas de cada
//! monitor, su escala y el ratón); el de lógica, que decide; y el de render, que es el
//! único que anima.

mod escena;
mod escenas;
mod formas;
mod gpu;
mod lenguaje;
mod logica;
#[cfg(feature = "luau")]
mod logica_luau;
mod plataforma;
mod render;
mod texto;

use escena::*;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const AYUDA: &str = "pleamar [opciones]
  --escena NOMBRE     un fichero de escena (.plm), que se recarga solo al guardarlo; o una de las
                      escritas en Rust: marea (por defecto), isla, cara, muestrario, enjambre
  --comprobar FICHERO lee una escena, dice si está bien y sale
  --decir [ESCENA] ORDEN   le dice algo a una escena en marcha y sale. Órdenes:
                      «emit suceso [n]», «fact hecho valor», «text nombre lo que ponga», «focus campo», «get nombre» (contesta), «quit»
  --pantalla NOMBRES  «todas», o monitores separados por comas (por defecto, lo que pida la escena).
                      Un nombre repetido da dos superficies en el mismo monitor.
  --bloqueo MS        lo que se bloquea la lógica tras cada decisión (600)
  --ingenuo           la lógica bloquea el hilo que pinta, como en QtQuick
  --demo              abre y cierra sola, sin ratón
  --raton GUION       ratón de mentira: «360,90@1000 pulsa@2500 baja@… sube@… rueda+@… fuera@4000» (ms)
  --segundos N        salir sola al cabo de N segundos
  --margen PX         margen superior, en vez del de la escena
  --sin-hud           sin la gráfica de frames
  --sin-vsync         pintar sin esperar a la pantalla, para medir lo que cuesta un frame
  --movimiento-reducido  los muelles se posan y los gestos enseñan su cara quieta
Botón derecho sobre ella para cerrarla.";

struct Args {
    escena: String,
    pantalla: Option<String>,
    bloqueo: u64,
    ingenuo: bool,
    demo: bool,
    raton: Option<String>,
    segundos: Option<u64>,
    margen: Option<i32>,
    hud: bool,
    reducido: bool,
    sin_vsync: bool,
}

fn args() -> Args {
    let mut a = Args { escena: "marea".into(), pantalla: None, bloqueo: 600, ingenuo: false, demo: false, raton: None, segundos: None, margen: None, hud: true, reducido: false, sin_vsync: false };
    let mut it = std::env::args().skip(1);
    while let Some(op) = it.next() {
        let mut valor = || it.next().unwrap_or_else(|| { eprintln!("{AYUDA}"); std::process::exit(2) });
        match op.as_str() {
            "--escena" => a.escena = valor(),
            "--decir" => {
                let (a1, a2) = (valor(), it.next());
                let r = match &a2 {
                    Some(orden) => plataforma::decir(Some(&a1), orden),
                    None => plataforma::decir(None, &a1),
                };
                std::process::exit(match r {
                    Ok(()) => 0,
                    Err(e) => {
                        eprintln!("{e}");
                        1
                    }
                });
            }
            "--comprobar" => {
                let ruta = valor();
                std::process::exit(match escenas::de_fichero::leer(&ruta) {
                    Ok(e) => {
                        println!("{ruta}: bien · {} propiedades, {} instrucciones, {} capas, {} reglas, {} zonas, {} gestos", e.props.len(), e.instrs.len(), e.capas.len(), e.reglas.len(), e.zonas.len(), e.gestos.len());
                        0
                    }
                    Err(m) => {
                        eprintln!("{m}");
                        1
                    }
                });
            }
            "--pantalla" => a.pantalla = Some(valor()),
            "--bloqueo" => a.bloqueo = valor().parse().expect("--bloqueo quiere milisegundos"),
            "--raton" => a.raton = Some(valor()),
            "--segundos" => a.segundos = Some(valor().parse().expect("--segundos quiere un número")),
            "--margen" => a.margen = Some(valor().parse().expect("--margen quiere píxeles")),
            "--ingenuo" => a.ingenuo = true,
            "--demo" => a.demo = true,
            "--sin-hud" => a.hud = false,
            "--movimiento-reducido" => a.reducido = true,
            "--sin-vsync" => a.sin_vsync = true,
            _ => { eprintln!("{AYUDA}"); std::process::exit(2) }
        }
    }
    a
}

fn main() {
    let arranque = std::time::Instant::now();
    let a = args();
    let bloqueada = Arc::new(AtomicBool::new(false));
    let (a_render, de_render) = channel();
    let (a_logica, de_logica) = channel();
    let mut guion: Box<dyn logica::Guion> = match a.escena.as_str() {
        "marea" => Box::<escenas::marea::Marea>::default(),
        "isla" => Box::<escenas::isla::Isla>::default(),
        "cara" => Box::<escenas::cara::Cara>::default(),
        "muestrario" => Box::<escenas::muestrario::Muestrario>::default(),
        "enjambre" => Box::<escenas::enjambre::Enjambre>::default(),
        ruta if std::path::Path::new(ruta).is_file() => escenas::de_fichero::guion_para(ruta, a_render.clone(), a_logica.clone(), bloqueada.clone()),
        otra => {
            eprintln!("no conozco la escena «{otra}», ni es un fichero\n{AYUDA}");
            std::process::exit(2)
        }
    };
    // La escena dice qué superficie quiere; la línea de órdenes puede llevarle
    // la contraria.
    let mut escena = guion.escena();
    if let Some(p) = &a.pantalla {
        escena.superficie.pantallas = if p == "todas" { Pantallas::Todas } else { Pantallas::Estas(p.split(',').map(str::to_owned).collect()) };
    }
    if let Some(m) = a.margen {
        escena.superficie.margen[0] = m;
    }

    // El taller de texto e imágenes. Lo primero que hace es leer las fuentes del
    // sistema, que es lo más lento del arranque: que vaya yendo.
    let letras = texto::Textos::abrir(a_render.clone());
    let instancia = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::PRIMARY, ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let pide = escena.superficie.clone();

    println!(
        "pleamar · modo {} · la lógica se bloquea {} ms tras cada decisión",
        if a.ingenuo { "INGENUO (un solo hilo)" } else { "separado (render independiente)" },
        a.bloqueo
    );
    let _ = a_render.send(ARender::Escena(escena));
    if std::path::Path::new(&a.escena).is_file() {
        escenas::de_fichero::vigilar(a.escena.clone(), a_render.clone(), a_logica.clone());
    }
    let a_logica_para_ordenes = a_logica.clone();
    let render = {
        let bloqueada = bloqueada.clone();
        let op = render::Opciones { hud: a.hud, ingenuo: a.ingenuo, reducido: a.reducido, sin_vsync: a.sin_vsync, arranque };
        let instancia = instancia.clone();
        std::thread::Builder::new()
            .name("render".into())
            .spawn(move || render::hilo(instancia, de_render, letras, a_logica, bloqueada, op))
            .unwrap()
    };
    {
        let op = logica::Opciones { bloqueo: Duration::from_millis(a.bloqueo), demo: a.demo, ingenuo: a.ingenuo, eco: a.raton.is_some() };
        let tx = a_render.clone();
        std::thread::Builder::new()
            .name("logica".into())
            .spawn(move || logica::hilo(guion, de_logica, tx, bloqueada, op))
            .unwrap();
    }

    // Lo que se le diga desde fuera —`pleamar --decir "emit toggle"`, que es lo que
    // ejecuta un atajo global del compositor— entra como si lo dijera la lógica.
    {
        let nombre = std::path::Path::new(&a.escena).file_stem().map_or(a.escena.clone(), |n| n.to_string_lossy().into_owned());
        let tx = Mutex::new((a_render.clone(), a_logica_para_ordenes));
        plataforma::escuchar_ordenes(&nombre, Box::new(move |linea| {
            let guardia = tx.lock().unwrap();
            let (tx, a_logica) = &*guardia;
            let mut p = linea.trim().splitn(3, ' ');
            let (que, quien, resto) = (p.next().unwrap_or(""), p.next().unwrap_or(""), p.next().unwrap_or(""));
            if que == "get" {
                let (pregunta, respuesta) = std::sync::mpsc::channel();
                let _ = tx.send(ARender::Pregunta(escena::internar(quien), pregunta));
                return Some(respuesta.recv_timeout(std::time::Duration::from_secs(1)).unwrap_or_else(|_| "? el render no contesta".into()));
            }
            let _ = match que {
                "emit" => tx.send(ARender::SucesoDeFuera(escena::internar(quien), resto.parse().ok())),
                // Lo que se pone desde fuera, la lógica tiene que saberlo: no lo ha puesto ella.
                "fact" => {
                    let v = match resto { "true" => 1.0, "false" => 0.0, n => n.parse().unwrap_or(0.0) };
                    let _ = a_logica.send(Evento::Hecho(escena::internar(quien), v));
                    tx.send(ARender::Hecho(escena::internar(quien), v))
                }
                "text" => {
                    let _ = a_logica.send(Evento::Texto(escena::internar(quien), resto.to_owned()));
                    tx.send(ARender::Texto(escena::internar(quien), resto.to_owned()))
                }
                "focus" => tx.send(ARender::Enfocar(Some(escena::internar(quien)))),
                "quit" => salir(),
                _ => {
                    eprintln!("órdenes · no entiendo «{linea}»");
                    return Some(format!("? no entiendo «{}»: emit, fact, text, focus, get, quit", linea.trim()));
                }
            };
            None
        }));
    }
    if let Some(guion) = a.raton.clone() {
        // Para ensayar las zonas sin quitarle el ratón a nadie.
        let tx = a_render.clone();
        std::thread::spawn(move || {
            let inicio = std::time::Instant::now();
            for paso in guion.split_whitespace() {
                let (que, cuando) = paso.split_once('@').expect("--raton: falta @ms");
                let cuando = Duration::from_millis(cuando.parse().expect("--raton: ms"));
                std::thread::sleep(cuando.saturating_sub(inicio.elapsed()));
                // Cada paso se dice, para saber a qué responde lo que venga detrás.
                println!("ratón  · {que}");
                let _ = match que {
                    // Un clic entero: bajar y subir.
                    "pulsa" => tx.send(ARender::Boton(0, true)).and_then(|_| tx.send(ARender::Boton(0, false))),
                    "baja" => tx.send(ARender::Boton(0, true)),
                    "sube" => tx.send(ARender::Boton(0, false)),
                    "derecho" => tx.send(ARender::Boton(1, true)).and_then(|_| tx.send(ARender::Boton(1, false))),
                    // `tecla:Escape`, `tecla:Ctrl+a`: bajar y subir.
                    t if t.starts_with("tecla:") => {
                        let mut m = Mods::default();
                        let mut nombre = &t[6..];
                        for (prefijo, pone) in [("Ctrl+", 0), ("Alt+", 1), ("Shift+", 2), ("Super+", 3)] {
                            if let Some(resto) = nombre.strip_prefix(prefijo) {
                                nombre = resto;
                                match pone {
                                    0 => m.ctrl = true,
                                    1 => m.alt = true,
                                    2 => m.mayus = true,
                                    _ => m.logo = true,
                                }
                            }
                        }
                        tx.send(ARender::Tecla(nombre.to_owned(), None, m)).and_then(|_| tx.send(ARender::TeclaSuelta(nombre.to_owned())))
                    }
                    // `escribe:hola`: letra a letra, como un teclado. Un `_` es un espacio.
                    t if t.starts_with("escribe:") => {
                        for ch in t[8..].chars() {
                            let ch = if ch == '_' { ' ' } else { ch };
                            let _ = tx.send(ARender::Tecla(ch.to_string(), Some(ch.to_string()), Mods::default()));
                            let _ = tx.send(ARender::TeclaSuelta(ch.to_string()));
                        }
                        Ok(())
                    }
                    "foco+" => tx.send(ARender::FocoTeclado(true)),
                    "foco-" => tx.send(ARender::FocoTeclado(false)),
                    t if t.starts_with("suelta:") => tx.send(ARender::Soltado("text/plain".into(), t[7..].to_owned())),
                    "rueda+" => tx.send(ARender::Rueda(1.0)),
                    "rueda-" => tx.send(ARender::Rueda(-1.0)),
                    "fuera" => tx.send(ARender::Puntero(None)),
                    xy => {
                        let (x, y) = xy.split_once(',').expect("--raton: x,y");
                        tx.send(ARender::Puntero(Some((x.parse().unwrap(), y.parse().unwrap()))))
                    }
                };
            }
        });
    }
    if let Some(s) = a.segundos {
        let tx = a_render.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(s));
            // Salir pasa por el render para que cierre su último ciclo de medidas.
            let _ = tx.send(ARender::Salir);
            std::thread::sleep(Duration::from_millis(400));
            salir();
        });
    }

    // A partir de aquí este hilo es de la plataforma: pone las ventanas y atiende
    // al sistema hasta que alguien cierre.
    let alto_extra = if a.hud { gpu::ALTO_INSTRUMENTOS as u32 } else { 0 };
    plataforma::atender(pide, alto_extra, instancia, a_render.clone());
    let _ = a_render.send(ARender::Salir);
    let _ = render.join();
    salir();
}

/// El proceso se va entero —el orden de destrucción no merece código en un
/// prototipo—, pero no sin parar antes lo que la lógica dejó corriendo.
fn salir() -> ! {
    #[cfg(feature = "luau")]
    logica_luau::parar_hijos();
    std::process::exit(0)
}
