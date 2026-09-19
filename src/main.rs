//! pleamar — prototipo de una shell donde animar no depende de la lógica.
//!
//! Tres hilos: este, que se lo queda la plataforma (las ventanas de cada
//! monitor, su escala y el ratón); el de lógica, que decide; y el de render, que es el
//! único que anima.

mod escena;
mod escenas;
mod formas;
mod gpu;
mod logica;
mod plataforma;
mod render;
mod texto;

use escena::*;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::Duration;

const AYUDA: &str = "pleamar [opciones]
  --escena NOMBRE     marea (por defecto), isla, cara, muestrario o enjambre (PLEAMAR_N formas)
  --pantalla NOMBRES  «todas», o monitores separados por comas (por defecto, lo que pida la escena).
                      Un nombre repetido da dos superficies en el mismo monitor.
  --bloqueo MS        lo que se bloquea la lógica tras cada decisión (600)
  --ingenuo           la lógica bloquea el hilo que pinta, como en QtQuick
  --demo              abre y cierra sola, sin ratón
  --raton GUION       ratón de mentira: «360,90@1000 pulsa@2500 fuera@4000» (ms)
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
    // Lo más lento del arranque es leer las fuentes del sistema. Que vaya yendo.
    let tipografo = std::thread::Builder::new().name("fuentes".into()).spawn(texto::Tipografo::nuevo).unwrap();
    let a = args();
    let bloqueada = Arc::new(AtomicBool::new(false));
    let mut guion: Box<dyn logica::Guion> = match a.escena.as_str() {
        "marea" => Box::<escenas::marea::Marea>::default(),
        "isla" => Box::<escenas::isla::Isla>::default(),
        "cara" => Box::<escenas::cara::Cara>::default(),
        "muestrario" => Box::<escenas::muestrario::Muestrario>::default(),
        "enjambre" => Box::<escenas::enjambre::Enjambre>::default(),
        otra => {
            eprintln!("no conozco la escena «{otra}»\n{AYUDA}");
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

    let (a_render, de_render) = channel();
    let (a_logica, de_logica) = channel();
    let instancia = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::PRIMARY, ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let pide = escena.superficie.clone();

    println!(
        "pleamar · modo {} · la lógica se bloquea {} ms tras cada decisión",
        if a.ingenuo { "INGENUO (un solo hilo)" } else { "separado (render independiente)" },
        a.bloqueo
    );
    let _ = a_render.send(ARender::Escena(escena));
    let render = {
        let bloqueada = bloqueada.clone();
        let op = render::Opciones { hud: a.hud, ingenuo: a.ingenuo, reducido: a.reducido, sin_vsync: a.sin_vsync, arranque };
        let instancia = instancia.clone();
        std::thread::Builder::new()
            .name("render".into())
            .spawn(move || render::hilo(instancia, de_render, tipografo, a_logica, bloqueada, op))
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

    if let Some(guion) = a.raton.clone() {
        // Para ensayar las zonas sin quitarle el ratón a nadie.
        let tx = a_render.clone();
        std::thread::spawn(move || {
            let inicio = std::time::Instant::now();
            for paso in guion.split_whitespace() {
                let (que, cuando) = paso.split_once('@').expect("--raton: falta @ms");
                let cuando = Duration::from_millis(cuando.parse().expect("--raton: ms"));
                std::thread::sleep(cuando.saturating_sub(inicio.elapsed()));
                let _ = match que {
                    "pulsa" => tx.send(ARender::Pulsar),
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
            std::process::exit(0);
        });
    }

    // A partir de aquí este hilo es de la plataforma: pone las ventanas y atiende
    // al sistema hasta que alguien cierre.
    let alto_extra = if a.hud { gpu::ALTO_INSTRUMENTOS as u32 } else { 0 };
    plataforma::atender(pide, alto_extra, instancia, a_render.clone());
    let _ = a_render.send(ARender::Salir);
    let _ = render.join();
    // El proceso se va entero: el orden de destrucción no merece código en un prototipo.
    std::process::exit(0);
}
