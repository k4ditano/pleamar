//! La bandeja del sistema: los iconos que dejan las aplicaciones que siguen
//! vivas sin ventana (StatusNotifierItem, el acuerdo de KDE que ya usan todos).
//!
//! Hay dos papeles. El **vigía** (`StatusNotifierWatcher`) es donde las
//! aplicaciones se apuntan, y solo puede haber uno. El **anfitrión** es quien
//! enseña los iconos, y puede haber varios. Si nadie es vigía, lo somos; si ya
//! lo es otro (otra barra), le preguntamos a él: la bandeja sale igual.
//!
//! `{ { key, id, title, status, icon, menu }, … }`. `icon` es un nombre de icono
//! o una ruta: vale tal cual para `image … = from`. `status`: Active, Passive,
//! NeedsAttention. En Windows será `Shell_NotifyIcon`; en macOS, `NSStatusItem`
//! no deja ver los de otros.

use super::Valor;
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::proxy::CacheProperties;
use zbus::MatchRule;

const VIGIA: &str = "org.kde.StatusNotifierWatcher";
const RUTA_VIGIA: &str = "/StatusNotifierWatcher";
const ELEMENTO: &str = "org.kde.StatusNotifierItem";

/// Los apuntados, cuando el vigía somos nosotros: «servicio/ruta».
#[derive(Default)]
struct Apuntados {
    elementos: Vec<String>,
    /// A quién avisar de que hay que volver a mirar, y qué señal dar.
    avisos: Option<Sender<Novedad>>,
}

enum Novedad {
    Mirar,
    Apuntado(String),
}

struct Vigia(Arc<Mutex<Apuntados>>);

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Vigia {
    fn register_status_notifier_item(&self, service: &str, #[zbus(header)] cabecera: zbus::message::Header<'_>) {
        // Unos dan su nombre en el bus; otros, solo la ruta del objeto, y el nombre es quien llama.
        let quien = cabecera.sender().map(|s| s.to_string()).unwrap_or_default();
        let entero = if service.starts_with('/') { format!("{quien}{service}") } else { format!("{service}/StatusNotifierItem") };
        let mut a = self.0.lock().unwrap();
        if !a.elementos.contains(&entero) {
            a.elementos.push(entero.clone());
            if let Some(tx) = &a.avisos {
                let _ = tx.send(Novedad::Apuntado(entero));
            }
        }
    }

    fn register_status_notifier_host(&self, _service: &str) {}

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.0.lock().unwrap().elementos.clone()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }
}

static CONEXION: OnceLock<Connection> = OnceLock::new();

fn partir(entero: &str) -> (&str, String) {
    match entero.find('/') {
        Some(k) => (&entero[..k], entero[k..].to_owned()),
        None => (entero, "/StatusNotifierItem".to_owned()),
    }
}

fn elemento(c: &Connection, entero: &str) -> Option<Proxy<'static>> {
    let (servicio, ruta) = partir(entero);
    // Sin caché: estos no avisan con `PropertiesChanged`, sino con `NewIcon` y compañía.
    zbus::blocking::proxy::Builder::new(c).destination(servicio.to_owned()).ok()?.path(ruta).ok()?.interface(ELEMENTO).ok()?.cache_properties(CacheProperties::No).build().ok()
}

/// Muchos —Electron, Telegram— no dan el nombre de un icono sino sus píxeles.
/// Se guardan como PNG, con el nombre según lo que contienen: si el icono
/// cambia, la ruta cambia, y quien lo pinta se entera.
fn de_pixeles(mapas: Vec<(i32, i32, Vec<u8>)>) -> Option<String> {
    // El más grande que haya: luego se reduce a lo que pida la escena.
    let (w, h, argb) = mapas.into_iter().filter(|(w, h, d)| *w > 0 && *h > 0 && d.len() == (*w * *h * 4) as usize).max_by_key(|(w, _, _)| *w)?;
    let mut sello = std::collections::hash_map::DefaultHasher::new();
    argb.hash(&mut sello);
    let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
    let carpeta = std::path::Path::new(&base).join("pleamar").join("tray");
    let ruta = carpeta.join(format!("{:016x}.png", sello.finish()));
    if !ruta.exists() {
        std::fs::create_dir_all(&carpeta).ok()?;
        // Llegan como ARGB, con el byte alto primero.
        let rgba: Vec<u8> = argb.chunks_exact(4).flat_map(|p| [p[1], p[2], p[3], p[0]]).collect();
        image::RgbaImage::from_raw(w as u32, h as u32, rgba)?.save(&ruta).ok()?;
    }
    Some(ruta.to_string_lossy().into_owned())
}

fn icono_de(p: &Proxy) -> String {
    let nombre: String = p.get_property("IconName").unwrap_or_default();
    if !nombre.is_empty() {
        // Algunos traen su propia carpeta de iconos, fuera del tema.
        let carpeta: String = p.get_property("IconThemePath").unwrap_or_default();
        if !carpeta.is_empty() {
            for ext in ["svg", "png"] {
                let ruta = std::path::Path::new(&carpeta).join(format!("{nombre}.{ext}"));
                if ruta.exists() {
                    return ruta.to_string_lossy().into_owned();
                }
            }
        }
        return nombre;
    }
    p.get_property::<Vec<(i32, i32, Vec<u8>)>>("IconPixmap").ok().and_then(de_pixeles).unwrap_or_default()
}

fn lista(c: &Connection, propios: Option<&Mutex<Apuntados>>) -> Vec<String> {
    match propios {
        Some(a) => a.lock().unwrap().elementos.clone(),
        None => Proxy::new(c, VIGIA, RUTA_VIGIA, VIGIA).ok().and_then(|p| p.get_property::<Vec<String>>("RegisteredStatusNotifierItems").ok()).unwrap_or_default(),
    }
}

fn contar(c: &Connection, propios: Option<&Mutex<Apuntados>>) -> Valor {
    let mut idos = Vec::new();
    let v = lista(c, propios).into_iter().filter_map(|entero| {
        let p = elemento(c, &entero)?;
        // Si ni su nombre contesta, es que la aplicación se ha ido.
        let Ok(id) = p.get_property::<String>("Id") else {
            idos.push(entero);
            return None;
        };
        let titulo: String = p.get_property("Title").unwrap_or_default();
        let menu = p.get_property::<zbus::zvariant::OwnedObjectPath>("Menu").is_ok_and(|m| m.as_str() != "/" && !m.as_str().is_empty());
        Some(Valor::Mapa(vec![
            ("key".into(), Valor::Texto(entero)),
            ("title".into(), Valor::Texto(if titulo.is_empty() { id.clone() } else { titulo })),
            ("id".into(), Valor::Texto(id)),
            ("status".into(), Valor::Texto(p.get_property("Status").unwrap_or_else(|_| "Active".to_owned()))),
            ("icon".into(), Valor::Texto(icono_de(&p))),
            ("menu".into(), Valor::Si(menu)),
        ]))
    }).collect();
    if let Some(a) = propios {
        a.lock().unwrap().elementos.retain(|e| !idos.contains(e));
    }
    Valor::Lista(v)
}

pub fn servicio(avisar: Box<dyn Fn(Valor) + Send>) -> bool {
    let apuntados = Arc::new(Mutex::new(Apuntados::default()));
    let Ok(c) = zbus::blocking::connection::Builder::session().and_then(|b| b.serve_at(RUTA_VIGIA, Vigia(apuntados.clone()))).and_then(|b| b.build()) else { return false };
    // ¿Hay ya vigía? Si no, nosotros. Sin hacer cola: o se es o no se es.
    let somos = c.request_name_with_flags(VIGIA, zbus::fdo::RequestNameFlags::DoNotQueue.into()).is_ok();
    if somos {
        println!("tray   · there was no watcher: applications register here");
    } else {
        // Anfitrión de la bandeja de otro: hay que tener un nombre y decírselo.
        let _ = c.object_server().remove::<Vigia, _>(RUTA_VIGIA);
        let mio = format!("org.kde.StatusNotifierHost-{}-pleamar", std::process::id());
        let _ = c.request_name(mio.as_str());
        let Ok(vigia) = Proxy::new(&c, VIGIA, RUTA_VIGIA, VIGIA) else { return false };
        if vigia.call_method("RegisterStatusNotifierHost", &(mio.as_str(),)).is_err() {
            return false;
        }
    }
    let _ = CONEXION.set(c.clone());
    let (tx, novedades) = channel();
    apuntados.lock().unwrap().avisos = Some(tx.clone());

    // Tres cosas hacen que haya que volver a mirar: que un icono cambie, que el
    // vigía (si es otro) apunte o borre a alguien, y que alguien se vaya del bus.
    let senal = |interfaz: &'static str, miembro: Option<&'static str>| {
        let b = MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(interfaz).ok()?;
        Some(match miembro { Some(m) => b.member(m).ok()?, None => b }.build())
    };
    for regla in [senal(ELEMENTO, None), senal(VIGIA, None), senal("org.freedesktop.DBus", Some("NameOwnerChanged"))].into_iter().flatten() {
        let (c, tx) = (c.clone(), tx.clone());
        std::thread::spawn(move || {
            let Ok(mensajes) = MessageIterator::for_match_rule(regla, &c, Some(32)) else { return };
            for _ in mensajes {
                if tx.send(Novedad::Mirar).is_err() { break }
            }
        });
    }

    std::thread::Builder::new().name("bandeja".into()).spawn(move || {
        let propios = somos.then_some(&*apuntados);
        let mut ultimo = String::new();
        let mut mirar = |c: &Connection| {
            let v = contar(c, propios);
            let huella = format!("{v:?}");
            if huella != ultimo {
                ultimo = huella;
                avisar(v);
            }
        };
        mirar(&c);
        while let Ok(n) = novedades.recv() {
            if let Novedad::Apuntado(entero) = &n {
                let _ = c.emit_signal(None::<&str>, RUTA_VIGIA, VIGIA, "StatusNotifierItemRegistered", &(entero.as_str(),));
            }
            // Un cambio suelen ser varias señales seguidas: se mira una vez.
            std::thread::sleep(std::time::Duration::from_millis(60));
            while novedades.try_recv().is_ok() {}
            mirar(&c);
        }
    }).is_ok()
}

// ── los menús ─────────────────────────────────────────────────────
//
// Otro acuerdo, `com.canonical.dbusmenu`: la aplicación publica su menú como un
// árbol y se le dice qué se ha pulsado. Aquí solo se lee y se avisa; pintarlo es
// cosa de la escena, que para eso tiene `popup`.

const MENU: &str = "com.canonical.dbusmenu";

fn menu_de(c: &Connection, clave: &str) -> Result<Proxy<'static>, String> {
    let p = elemento(c, clave).ok_or("that icon is gone")?;
    let ruta: zbus::zvariant::OwnedObjectPath = p.get_property("Menu").map_err(|_| "that icon has no menu")?;
    let (servicio, _) = partir(clave);
    zbus::blocking::proxy::Builder::new(c).destination(servicio.to_owned()).map_err(|e| e.to_string())?.path(ruta).map_err(|e| e.to_string())?.interface(MENU).map_err(|e| e.to_string())?.cache_properties(CacheProperties::No).build().map_err(|e| e.to_string())
}

/// Un nodo del árbol llega como (id, propiedades, hijos), y cada hijo, envuelto
/// en un variante, es otro igual.
fn nodo(v: &zbus::zvariant::Value) -> Option<Valor> {
    use zbus::zvariant::Value;
    let v = if let Value::Value(dentro) = v { dentro } else { v };
    let Value::Structure(s) = v else { return None };
    let [Value::I32(id), Value::Dict(props), Value::Array(hijos)] = s.fields() else { return None };
    let texto = |k: &str| props.get::<&str, &str>(&k).ok().flatten().map(str::to_owned);
    let si = |k: &str| props.get::<&str, bool>(&k).ok().flatten();
    if si("visible") == Some(false) {
        return None;
    }
    let hijos: Vec<Valor> = hijos.iter().filter_map(nodo).collect();
    // `_Archivo` marca la letra del atajo; `__` es un guion bajo de verdad.
    let etiqueta = texto("label").unwrap_or_default().replace("__", "\u{1}").replace('_', "").replace('\u{1}', "_");
    let marca = texto("toggle-type").filter(|t| !t.is_empty());
    let mut m = vec![
        ("id".to_owned(), Valor::Num(*id as f64)),
        ("label".to_owned(), Valor::Texto(etiqueta)),
        ("enabled".to_owned(), Valor::Si(si("enabled").unwrap_or(true))),
        ("separator".to_owned(), Valor::Si(texto("type").as_deref() == Some("separator"))),
        ("children".to_owned(), Valor::Lista(hijos)),
    ];
    if marca.is_some() {
        m.push(("checked".to_owned(), Valor::Si(props.get::<&str, i32>(&"toggle-state").ok().flatten() == Some(1))));
    }
    Some(Valor::Mapa(m))
}

/// `sys.ask("tray.menu", key)` → `{ { id, label, enabled, separator, checked, children }, … }`
pub fn consulta(que: &str, args: &[Valor]) -> Result<Valor, String> {
    let c = CONEXION.get().ok_or("the tray is not running: sys.watch(\"tray\", …) is missing")?;
    let ("tray.menu", [Valor::Texto(clave)]) = (que, args) else { return Err(format!("'{que}' is not asked like that: tray.menu(key)")) };
    let menu = menu_de(c, clave)?;
    // Muchos no rellenan su menú hasta que se les dice que se va a enseñar.
    let _ = menu.call_method("AboutToShow", &(0i32,));
    let respuesta = menu.call_method("GetLayout", &(0i32, -1i32, Vec::<&str>::new())).map_err(|e| e.to_string())?;
    let cuerpo = respuesta.body();
    // De la raíz solo interesan sus hijos: ella misma no es nada que se pueda pulsar.
    type Raiz = (i32, std::collections::HashMap<String, zbus::zvariant::OwnedValue>, Vec<zbus::zvariant::OwnedValue>);
    let (_, (_, _, hijos)): (u32, Raiz) = cuerpo.deserialize().map_err(|e| format!("I don't understand the menu that application sent: {e}"))?;
    Ok(Valor::Lista(hijos.iter().filter_map(|h| nodo(h)).collect()))
}

/// `tray.activate(key)` —el clic de siempre—, `tray.secondary(key)` —el del
/// medio—, `tray.context(key)` —que la aplicación enseñe su menú, si sabe— y
/// `tray.scroll(key, muescas)`.
pub fn orden(que: &str, args: &[Valor]) -> Result<(), String> {
    let c = CONEXION.get().ok_or("the tray is not running: sys.watch(\"tray\", …) is missing")?;
    let [Valor::Texto(clave), resto @ ..] = args else { return Err(format!("'{que}' wants the icon's `key`")) };
    let p = elemento(c, clave).ok_or("that icon is gone")?;
    let hecho = match (que, resto) {
        // Dónde se pulsó, en la pantalla. No lo sabemos (Wayland no lo cuenta): 0, 0.
        ("tray.activate", []) => p.call_method("Activate", &(0i32, 0i32)),
        ("tray.secondary", []) => p.call_method("SecondaryActivate", &(0i32, 0i32)),
        ("tray.context", []) => p.call_method("ContextMenu", &(0i32, 0i32)),
        ("tray.scroll", [Valor::Num(d)]) => p.call_method("Scroll", &(*d as i32 * 120, "vertical")),
        // Han elegido algo de su menú: el `id` es el que venía en `tray.menu`.
        ("tray.menu_click", [Valor::Num(id)]) => {
            let menu = menu_de(c, clave)?;
            let ahora = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as u32);
            return menu.call_method("Event", &(*id as i32, "clicked", zbus::zvariant::Value::I32(0), ahora)).map(|_| ()).map_err(|e| e.to_string());
        }
        _ => return Err(format!("'{que}' does not exist: tray.activate, tray.secondary, tray.context, tray.scroll, tray.menu_click")),
    };
    hecho.map(|_| ()).map_err(|e| e.to_string())
}
