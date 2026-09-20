//! Quién ha dicho que sí. Un plugin declara en su fichero lo que su lógica quiere
//! tocar del sistema, pero declararlo no es tenerlo: **lo aprueba quien lo usa**, una
//! vez, y se guarda fuera del plugin, junto a la huella de su lógica y de lo que
//! pedía. Si el plugin cambia —su código, o lo que pide—, la aprobación ya no vale.
//!
//! La escena que uno mismo abre no pasa por aquí: abrirla ya es decidir, como
//! ejecutar un script. Lo que entra por un `import` es lo que hay que mirar.

use crate::escena::{Permisos, Plugin};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Lo que ejecuta cualquier cosa que se le diga: pedirlo es pedirlo todo.
const INTERPRETES: [&str; 12] = ["sh", "bash", "zsh", "fish", "dash", "python", "python3", "perl", "ruby", "node", "lua", "env"];

fn fichero() -> PathBuf {
    crate::plataforma::carpeta_de_ajustes().join("permisos.json")
}

/// FNV-1a, de 64 bits: pequeña, y la misma en todas partes y en todas las versiones.
fn fnv(bytes: impl Iterator<Item = u8>) -> u64 {
    bytes.fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

/// De qué depende una aprobación: del código del plugin y de lo que pide.
pub fn huella(p: &Plugin) -> String {
    let codigo = std::fs::read(&p.logica).unwrap_or_default();
    let pide = format!("|run:{}|services:{}", p.permisos.ordenes.join(","), p.permisos.servicios.join(","));
    format!("{:016x}", fnv(codigo.into_iter().chain(pide.bytes())))
}

fn aprobados() -> BTreeMap<String, String> {
    std::fs::read_to_string(fichero()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn clave(p: &Plugin) -> String {
    p.logica.canonicalize().unwrap_or_else(|_| p.logica.clone()).to_string_lossy().into_owned()
}

/// Un plugin que no pide nada no necesita que nadie le diga que sí.
pub fn aprobado(p: &Plugin) -> bool {
    p.permisos == Permisos::default() || aprobados().get(&clave(p)) == Some(&huella(p))
}

/// Los permisos con los que corre de verdad: los suyos si están aprobados; si no, ninguno.
pub fn los_que_valen(p: &Plugin) -> Permisos {
    if aprobado(p) { p.permisos.clone() } else { Permisos::default() }
}

pub fn aprobar(p: &Plugin) -> Result<(), String> {
    let mut todos = aprobados();
    todos.insert(clave(p), huella(p));
    let ruta = fichero();
    std::fs::create_dir_all(ruta.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&ruta, serde_json::to_string_pretty(&todos).unwrap()).map_err(|e| format!("{}: {e}", ruta.display()))
}

/// Lo que pide, dicho para que se entienda, y avisando de lo que es pedirlo todo.
pub fn en_claro(p: &Permisos) -> String {
    let lista = |l: &[String], marca: &dyn Fn(&str) -> bool| match l {
        [] => "ninguno".to_owned(),
        _ => l.iter().map(|x| if marca(x) { format!("{x} ⚠") } else { x.clone() }).collect::<Vec<_>>().join(", "),
    };
    let es_interprete = |o: &str| INTERPRETES.contains(&o.rsplit('/').next().unwrap_or(o));
    let mut s = format!("órdenes: {} · servicios: {}", lista(&p.ordenes, &es_interprete), lista(&p.servicios, &|_| false));
    if p.ordenes.iter().any(|o| es_interprete(o)) {
        s.push_str(" · ⚠ un intérprete ejecuta lo que se le diga: es pedirlo todo");
    }
    s
}

/// `pleamar --aprobar escena.plm`: enseña lo que pide cada plugin de esa escena y pregunta.
/// Con `si_a_todo`, no pregunta (para guiones; úsese sabiendo lo que se aprueba).
pub fn preguntar(escena: &str, si_a_todo: bool) -> i32 {
    let e = match crate::escenas::de_fichero::leer(escena) {
        Ok(e) => e,
        Err(m) => {
            eprintln!("{m}");
            return 1;
        }
    };
    let pendientes: Vec<&Plugin> = e.plugins.iter().filter(|p| !aprobado(p)).collect();
    if e.plugins.is_empty() {
        println!("{escena} no usa ningún plugin: no hay nada que aprobar.");
        return 0;
    }
    for p in e.plugins.iter().filter(|p| aprobado(p)) {
        println!("✓ «{}» · {} · {}", p.nombre, p.logica.display(), if p.permisos == Permisos::default() { "no pide nada".to_owned() } else { format!("ya aprobado · {}", en_claro(&p.permisos)) });
    }
    let mut negados = 0;
    for p in pendientes {
        println!("\n? El plugin «{}» ({}) quiere:\n    {}", p.nombre, p.logica.display(), en_claro(&p.permisos));
        let si = si_a_todo || {
            print!("  ¿Se lo apruebas? [s/N] ");
            let _ = std::io::Write::flush(&mut std::io::stdout());
            let mut linea = String::new();
            let _ = std::io::stdin().read_line(&mut linea);
            matches!(linea.trim().to_lowercase().as_str(), "s" | "si" | "sí" | "y" | "yes")
        };
        if si {
            match aprobar(p) {
                Ok(()) => println!("  aprobado. Si su lógica o lo que pide cambia, habrá que volver a aprobarlo."),
                Err(m) => {
                    eprintln!("  no he podido guardarlo: {m}");
                    return 1;
                }
            }
        } else {
            println!("  no aprobado: correrá, pero sin poder tocar nada del sistema.");
            negados += 1;
        }
    }
    if negados > 0 { 2 } else { 0 }
}
