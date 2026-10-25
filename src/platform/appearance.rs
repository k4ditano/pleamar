//! What the system says about how things should move and look, through the
//! desktop portal (`org.freedesktop.portal.Settings`, the namespace
//! `org.freedesktop.appearance`): whether to reduce motion (`reduced-motion`:
//! 0 no preference, 1 reduce) and the colour scheme (`color-scheme`: 0 no
//! preference, 1 dark, 2 light). A backend that does not say `reduced-motion`
//! —GTK's, which most desktops use for settings, says GNOME's own instead— is
//! read through GNOME's `enable-animations` (`org.gnome.desktop.interface`),
//! the other way round. Only read: nothing here changes a setting.
//!
//! They reach the scene as the facts `motion.reduced` and `color.scheme`, as if
//! said from outside (`--say … fact`): at the start, and each time the portal
//! says one changed (`SettingChanged`). Nothing polls: a thread waits on the
//! bus. With no portal on the bus, nothing happens and both stay 0.

use crate::scene::{intern, ToRender};
use std::sync::mpsc::Sender;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::zvariant::Value;
use zbus::MatchRule;

const APPEARANCE: &str = "org.freedesktop.appearance";
const GNOME: &str = "org.gnome.desktop.interface";

/// A setting as one of the two facts, and what it is worth there; `None` if it
/// is none of them. `gnome` is whether GNOME's `enable-animations` is the one
/// that says it (the portal had no `reduced-motion`).
pub fn as_fact(namespace: &str, key: &str, value: &Value, gnome: bool) -> Option<(&'static str, u32)> {
    match (namespace, key) {
        (APPEARANCE, "reduced-motion") => Some(("motion.reduced", (number(value)? > 0) as u32)),
        (APPEARANCE, "color-scheme") => Some(("color.scheme", number(value)?.min(2))),
        (GNOME, "enable-animations") if gnome => Some(("motion.reduced", (number(value)? == 0) as u32)),
        _ => None,
    }
}

/// A number, or a yes as 1, however deep in variants it comes (`Read`, the old
/// call, wraps it twice).
fn number(v: &Value) -> Option<u32> {
    match v {
        Value::U32(n) => Some(*n),
        Value::I32(n) => u32::try_from(*n).ok(),
        Value::U8(n) => Some(u32::from(*n)),
        Value::Bool(b) => Some(u32::from(*b)),
        Value::Value(inner) => number(inner),
        _ => None,
    }
}

/// From now on, what the system says reaches the scene. Once per process.
pub fn watch(to_render: Sender<ToRender>) {
    let _ = std::thread::Builder::new().name("appearance".into()).spawn(move || {
        let Ok(c) = Connection::session() else { return };
        let Ok(p) = Proxy::new(&c, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Settings") else { return };
        let read = |ns: &str, key: &str| -> Option<zbus::zvariant::OwnedValue> {
            p.call("ReadOne", &(ns, key)).or_else(|_| p.call("Read", &(ns, key))).ok()
        };
        let tell = |(name, v): (&'static str, u32)| to_render.send(ToRender::ExternalFact(intern(name), v.to_string())).is_ok();
        let scheme = read(APPEARANCE, "color-scheme");
        let reduced = read(APPEARANCE, "reduced-motion");
        // GNOME's says it only where the portal's own does not.
        let gnome = reduced.is_none();
        let animations = gnome.then(|| read(GNOME, "enable-animations")).flatten();
        let said = [(APPEARANCE, "color-scheme", scheme), (APPEARANCE, "reduced-motion", reduced), (GNOME, "enable-animations", animations)];
        for (ns, key, v) in said {
            if let Some(f) = v.and_then(|v| as_fact(ns, key, &v, gnome)) {
                if !tell(f) {
                    return;
                }
            }
        }
        let Ok(rule) = MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.freedesktop.portal.Settings")
            .and_then(|b| b.member("SettingChanged"))
            .map(|b| b.build())
        else {
            return;
        };
        let Ok(messages) = MessageIterator::for_match_rule(rule, &c, Some(16)) else { return };
        for m in messages.flatten() {
            let Ok((ns, key, v)) = m.body().deserialize::<(String, String, zbus::zvariant::OwnedValue)>() else { continue };
            if let Some(f) = as_fact(&ns, &key, &v, gnome) {
                // The scene went away: so does this.
                if !tell(f) {
                    return;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_portal_says_as_facts() {
        assert_eq!(as_fact(APPEARANCE, "reduced-motion", &Value::U32(1), true), Some(("motion.reduced", 1)));
        assert_eq!(as_fact(APPEARANCE, "reduced-motion", &Value::U32(0), false), Some(("motion.reduced", 0)));
        assert_eq!(as_fact(APPEARANCE, "color-scheme", &Value::U32(1), false), Some(("color.scheme", 1)));
        assert_eq!(as_fact(APPEARANCE, "color-scheme", &Value::U32(2), false), Some(("color.scheme", 2)));
        // A value the spec does not have yet is no scheme.
        assert_eq!(as_fact(APPEARANCE, "color-scheme", &Value::U32(7), false), Some(("color.scheme", 2)));
        // `Read` wraps it in another variant.
        assert_eq!(as_fact(APPEARANCE, "color-scheme", &Value::Value(Box::new(Value::U32(1))), false), Some(("color.scheme", 1)));
        // GNOME's, the other way round, and only when the portal has no `reduced-motion`.
        assert_eq!(as_fact(GNOME, "enable-animations", &Value::Bool(false), true), Some(("motion.reduced", 1)));
        assert_eq!(as_fact(GNOME, "enable-animations", &Value::Bool(true), true), Some(("motion.reduced", 0)));
        assert_eq!(as_fact(GNOME, "enable-animations", &Value::Bool(false), false), None);
        // Anything else is not ours.
        assert_eq!(as_fact(APPEARANCE, "accent-color", &Value::U32(1), false), None);
        assert_eq!(as_fact(APPEARANCE, "reduced-motion", &Value::Str("yes".into()), false), None);
    }
}
