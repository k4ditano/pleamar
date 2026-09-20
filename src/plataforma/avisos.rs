//! Las notificaciones del escritorio. Aquí no se escucha a nadie: **se es** el
//! servidor. Las aplicaciones llaman a `org.freedesktop.Notifications.Notify`,
//! y quien tenga ese nombre en el bus es quien las enseña. Solo puede ser uno:
//! si ya lo tiene otro (otra barra, un demonio), el servicio dice que no está.
//!
//! En Windows será `UserNotificationListener`; macOS no deja leer las de otros.
//!
//! `{ { id, app, title, body, icon, urgency, actions = { { key, label }, … } }, … }`,
//! la más nueva primero. `urgency`: 0 poca, 1 normal, 2 crítica.

use super::Valor;
use std::collections::HashMap;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use zbus::zvariant::OwnedValue;

const NOMBRE: &str = "org.freedesktop.Notifications";
const RUTA: &str = "/org/freedesktop/Notifications";
/// Lo que dura una que no dice cuánto quiere durar.
const POR_DEFECTO: Duration = Duration::from_secs(6);

struct Aviso {
    id: u32,
    app: String,
    titulo: String,
    cuerpo: String,
    icono: String,
    urgencia: u8,
    acciones: Vec<(String, String)>,
    caduca: Option<Instant>,
}

/// Lo que hay que contarle al bus, o hacer, fuera de la llamada que lo provocó.
enum Cosa {
    Cambio,
    Cerrada(u32, u32),
    Accion(u32, String),
}

#[derive(Default)]
struct Estado {
    avisos: Vec<Aviso>,
    siguiente: u32,
}

struct Central {
    estado: Mutex<Estado>,
    cosas: Mutex<Sender<Cosa>>,
}

static CENTRAL: OnceLock<Arc<Central>> = OnceLock::new();

impl Central {
    fn decir(&self, c: Cosa) {
        let _ = self.cosas.lock().unwrap().send(c);
    }

    /// Por qué se cierra: 1 caducó, 2 la cerró el usuario, 3 lo pidió la aplicación.
    fn cerrar(&self, id: u32, motivo: u32) -> bool {
        let mut e = self.estado.lock().unwrap();
        let antes = e.avisos.len();
        e.avisos.retain(|a| a.id != id);
        let estaba = e.avisos.len() != antes;
        drop(e);
        if estaba {
            self.decir(Cosa::Cerrada(id, motivo));
        }
        estaba
    }

    fn contar(&self) -> Valor {
        let texto = |s: &str| Valor::Texto(s.to_owned());
        Valor::Lista(self.estado.lock().unwrap().avisos.iter().rev().map(|a| Valor::Mapa(vec![
            ("id".into(), Valor::Num(a.id as f64)),
            ("app".into(), texto(&a.app)),
            ("title".into(), texto(&a.titulo)),
            ("body".into(), texto(&a.cuerpo)),
            ("icon".into(), texto(&a.icono)),
            ("urgency".into(), Valor::Num(a.urgencia as f64)),
            ("actions".into(), Valor::Lista(a.acciones.iter().map(|(k, l)| Valor::Mapa(vec![("key".into(), texto(k)), ("label".into(), texto(l))])).collect())),
        ])).collect())
    }
}

/// Decimos que no entendemos de etiquetas, pero hay quien las manda igual.
fn sin_etiquetas(s: &str) -> String {
    let mut fuera = String::with_capacity(s.len());
    let mut dentro = false;
    for c in s.chars() {
        match c {
            '<' => dentro = true,
            '>' if dentro => dentro = false,
            _ if !dentro => fuera.push(c),
            _ => {}
        }
    }
    fuera.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'")
}

struct Servidor(Arc<Central>);

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Servidor {
    #[allow(clippy::too_many_arguments)]
    fn notify(&self, app_name: &str, replaces_id: u32, app_icon: &str, summary: &str, body: &str, actions: Vec<String>, hints: HashMap<String, OwnedValue>, expire_timeout: i32) -> u32 {
        let pista = |k: &str| hints.get(k).and_then(|v| <&str>::try_from(v).ok()).map(str::to_owned);
        let icono = if app_icon.is_empty() { pista("image-path").or_else(|| pista("image_path")).unwrap_or_default() } else { app_icon.to_owned() };
        let urgencia = hints.get("urgency").and_then(|v| u8::try_from(v).ok()).unwrap_or(1);
        let mut e = self.0.estado.lock().unwrap();
        // Con `replaces_id`, la misma notificación que cambia: el volumen que sube, una descarga que avanza.
        let id = if replaces_id != 0 && e.avisos.iter().any(|a| a.id == replaces_id) {
            e.avisos.retain(|a| a.id != replaces_id);
            replaces_id
        } else {
            e.siguiente += 1;
            e.siguiente
        };
        let caduca = match expire_timeout {
            0 => None,
            // Las críticas no se van solas, digan lo que digan.
            _ if urgencia >= 2 => None,
            ms if ms > 0 => Some(Instant::now() + Duration::from_millis(ms as u64)),
            _ => Some(Instant::now() + POR_DEFECTO),
        };
        e.avisos.push(Aviso {
            id,
            app: app_name.to_owned(),
            titulo: sin_etiquetas(summary),
            cuerpo: sin_etiquetas(body),
            icono: icono.trim_start_matches("file://").to_owned(),
            urgencia,
            // Vienen de dos en dos: clave, etiqueta.
            acciones: actions.chunks(2).filter(|p| p.len() == 2).map(|p| (p[0].clone(), p[1].clone())).collect(),
            caduca,
        });
        drop(e);
        self.0.decir(Cosa::Cambio);
        id
    }

    fn close_notification(&self, id: u32) {
        if self.0.cerrar(id, 3) {
            self.0.decir(Cosa::Cambio);
        }
    }

    fn get_capabilities(&self) -> Vec<String> {
        vec!["body".into(), "actions".into(), "icon-static".into(), "persistence".into()]
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        ("pleamar".into(), "k4ditano".into(), env!("CARGO_PKG_VERSION").into(), "1.2".into())
    }
}

pub fn servicio(avisar: Box<dyn Fn(Valor) + Send>) -> bool {
    let (tx, cosas) = channel();
    let central = Arc::new(Central { estado: Mutex::default(), cosas: Mutex::new(tx) });
    // Sin hacer cola ni quitárselo a nadie: si el nombre tiene dueño, aquí no hay servicio.
    let conexion = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.serve_at(RUTA, Servidor(central.clone())))
        .and_then(|b| b.build())
        .and_then(|c| c.request_name_with_flags(NOMBRE, zbus::fdo::RequestNameFlags::DoNotQueue.into()).map(|_| c));
    let conexion = match conexion {
        Ok(c) => c,
        Err(e) => {
            eprintln!("avisos · no puedo ser quien recibe las notificaciones ({e}): ¿las tiene ya otro programa?");
            return false;
        }
    };
    let _ = CENTRAL.set(central.clone());
    std::thread::Builder::new().name("avisos".into()).spawn(move || {
        // Solo se cuenta si la lista es otra: descartar lo que ya no está no es noticia.
        let ultimo = std::cell::RefCell::new(String::new());
        let avisar = |v: Valor| {
            let huella = format!("{v:?}");
            if *ultimo.borrow() != huella {
                *ultimo.borrow_mut() = huella;
                avisar(v);
            }
        };
        avisar(central.contar());
        loop {
            // Se duerme hasta que pase algo o le toque caducar a la siguiente.
            let ahora = Instant::now();
            let espera = central.estado.lock().unwrap().avisos.iter().filter_map(|a| a.caduca).min().map(|c| c.saturating_duration_since(ahora));
            let cosa = match espera {
                Some(d) => cosas.recv_timeout(d),
                None => cosas.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match cosa {
                Ok(Cosa::Cambio) => avisar(central.contar()),
                Ok(Cosa::Cerrada(id, motivo)) => {
                    let _ = conexion.emit_signal(None::<&str>, RUTA, NOMBRE, "NotificationClosed", &(id, motivo));
                }
                Ok(Cosa::Accion(id, clave)) => {
                    let _ = conexion.emit_signal(None::<&str>, RUTA, NOMBRE, "ActionInvoked", &(id, clave.as_str()));
                }
                Err(RecvTimeoutError::Timeout) => {
                    let ahora = Instant::now();
                    let caducadas: Vec<u32> = central.estado.lock().unwrap().avisos.iter().filter(|a| a.caduca.is_some_and(|c| c <= ahora)).map(|a| a.id).collect();
                    for id in caducadas {
                        central.cerrar(id, 1);
                    }
                    avisar(central.contar());
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }).is_ok()
}

/// `notifications.dismiss(id)`, `notifications.invoke(id, "default")`, `notifications.clear()`.
pub fn orden(que: &str, args: &[Valor]) -> Result<(), String> {
    let central = CENTRAL.get().ok_or("the notifications service is not running: sys.watch(\"notifications\", …) is missing")?;
    match (que, args) {
        ("notifications.dismiss", [Valor::Num(id)]) => {
            central.cerrar(*id as u32, 2);
        }
        ("notifications.invoke", [Valor::Num(id), Valor::Texto(clave)]) => {
            // La aplicación se entera de qué botón fue, y la notificación ya ha cumplido.
            central.decir(Cosa::Accion(*id as u32, clave.clone()));
            central.cerrar(*id as u32, 2);
        }
        ("notifications.clear", []) => {
            let ids: Vec<u32> = central.estado.lock().unwrap().avisos.iter().map(|a| a.id).collect();
            ids.into_iter().for_each(|id| { central.cerrar(id, 2); });
        }
        _ => return Err(format!("'{que}' is not asked like that: notifications.dismiss(id), notifications.invoke(id, key), notifications.clear()")),
    }
    central.decir(Cosa::Cambio);
    Ok(())
}
