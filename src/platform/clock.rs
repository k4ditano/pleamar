//! The time, without calling `date` every second. A thread that sleeps until the next
//! minute —or the next second, if someone wants them— and reports the local time.
//!
//! `{ hour, minute, second, day, month, year, weekday, time, date }`. The last two
//! come ready-made ("10:41", "Sun 20 Sep") for whatever only wants to show them.

use super::SysValue;
use chrono::{Datelike, Local, Timelike};
use std::time::Duration;

fn now() -> SysValue {
    let t = Local::now();
    let field = |k: &str, v: u32| (k.to_owned(), SysValue::Num(v as f64));
    SysValue::Map(vec![
        field("hour", t.hour()),
        field("minute", t.minute()),
        field("second", t.second()),
        field("day", t.day()),
        field("month", t.month()),
        ("year".into(), SysValue::Num(t.year() as f64)),
        // 0 is Sunday, as almost everywhere.
        field("weekday", t.weekday().num_days_from_sunday()),
        ("time".into(), SysValue::Text(t.format("%H:%M").to_string())),
        ("date".into(), SysValue::Text(t.format("%a %d %b").to_string())),
    ])
}

/// `clock` reports when the minute changes; `clock.seconds`, every second. Both start
/// by reporting the current time, and then sleep exactly until the next change: no
/// waking up four times a second to check whether it's time yet.
pub fn service(name: &str, notify: Box<dyn Fn(SysValue) + Send>) -> bool {
    let every_second = name == "clock.seconds";
    std::thread::Builder::new()
        .name("clock".into())
        .spawn(move || loop {
            notify(now());
            let t = Local::now();
            let remaining = if every_second {
                1_000 - (t.timestamp_subsec_millis() as u64 % 1_000)
            } else {
                let into_minute = t.second() as u64 * 1_000 + t.timestamp_subsec_millis() as u64;
                60_000 - into_minute
            };
            std::thread::sleep(Duration::from_millis(remaining.max(10)));
        })
        .is_ok()
}
