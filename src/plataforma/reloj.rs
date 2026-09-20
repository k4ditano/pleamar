//! La hora, sin llamar a `date` cada segundo. Un hilo que duerme hasta el siguiente
//! minuto —o el siguiente segundo, si alguien los quiere— y cuenta la hora de aquí.
//!
//! `{ hour, minute, second, day, month, year, weekday, time, date }`. Los dos últimos
//! vienen hechos («10:41», «Sun 20 Sep») para lo que solo quiere enseñarlos.

use super::Valor;
use chrono::{Datelike, Local, Timelike};
use std::time::Duration;

fn ahora() -> Valor {
    let t = Local::now();
    let campo = |k: &str, v: u32| (k.to_owned(), Valor::Num(v as f64));
    Valor::Mapa(vec![
        campo("hour", t.hour()),
        campo("minute", t.minute()),
        campo("second", t.second()),
        campo("day", t.day()),
        campo("month", t.month()),
        ("year".into(), Valor::Num(t.year() as f64)),
        // 0 es domingo, como en casi todas partes.
        campo("weekday", t.weekday().num_days_from_sunday()),
        ("time".into(), Valor::Texto(t.format("%H:%M").to_string())),
        ("date".into(), Valor::Texto(t.format("%a %d %b").to_string())),
    ])
}

/// `clock` avisa al cambiar el minuto; `clock.seconds`, cada segundo. Los dos empiezan
/// contando la hora de ahora, y luego duermen justo hasta el siguiente cambio: nada de
/// despertarse cuatro veces por segundo para mirar si ya toca.
pub fn servicio(nombre: &str, avisar: Box<dyn Fn(Valor) + Send>) -> bool {
    let cada_segundo = nombre == "clock.seconds";
    std::thread::Builder::new()
        .name("clock".into())
        .spawn(move || loop {
            avisar(ahora());
            let t = Local::now();
            let falta = if cada_segundo {
                1_000 - (t.timestamp_subsec_millis() as u64 % 1_000)
            } else {
                let en_el_minuto = t.second() as u64 * 1_000 + t.timestamp_subsec_millis() as u64;
                60_000 - en_el_minuto
            };
            std::thread::sleep(Duration::from_millis(falta.max(10)));
        })
        .is_ok()
}
