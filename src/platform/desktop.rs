//! What a freedesktop desktop knows about itself without asking any
//! compositor: which applications are installed, and how to launch one.

use super::SysValue;
use std::collections::BTreeMap;

/// The `.desktop` files of every applications folder, without the hidden ones.
pub fn applications() -> SysValue {
    let home = std::env::var("HOME").unwrap_or_default();
    let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    let mut folders = vec![format!("{home}/.local/share/applications")];
    folders.extend(data_dirs.split(':').map(|d| format!("{d}/applications")));
    // By name, and the first folder wins: the user's overrides the system's.
    let mut apps: BTreeMap<String, (String, String)> = BTreeMap::new();
    for folder in folders {
        let Ok(dir) = std::fs::read_dir(&folder) else { continue };
        for f in dir.filter_map(Result::ok).filter(|f| f.path().extension().is_some_and(|e| e == "desktop")) {
            let Ok(text) = std::fs::read_to_string(f.path()) else { continue };
            let (mut name, mut command, mut icon, mut hidden, mut inside) = (None, None, String::new(), false, false);
            for l in text.lines() {
                if l.starts_with('[') {
                    inside = l == "[Desktop Entry]";
                } else if inside {
                    match l.split_once('=') {
                        Some(("Name", v)) => name = name.or(Some(v.to_owned())),
                        Some(("Exec", v)) => command = Some(v.split_whitespace().filter(|p| !p.starts_with('%')).collect::<Vec<_>>().join(" ")),
                        Some(("Icon", v)) => icon = v.to_owned(),
                        Some(("NoDisplay" | "Hidden", "true")) => hidden = true,
                        Some(("Type", v)) if v != "Application" => hidden = true,
                        _ => {}
                    }
                }
            }
            if let (Some(n), Some(o), false) = (name, command, hidden) {
                apps.entry(n).or_insert((o, icon));
            }
        }
    }
    SysValue::List(apps.into_iter().map(|(n, (o, i))| SysValue::Map(vec![("name".into(), SysValue::Text(n)), ("exec".into(), SysValue::Text(o)), ("icon".into(), SysValue::Text(i))])).collect())
}

/// Launches a command and forgets about it: it is not a child of the logic, and does not die with us.
///
/// Only what really is an installed application: otherwise, `apps.launch`
/// would be a way to run anything with the permission to "see the apps".
pub fn launch(command: &str) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let SysValue::List(apps) = applications() else { return Err("I don't know what applications there are".into()) };
    let known = apps.iter().any(|a| matches!(a, SysValue::Map(m) if m.iter().any(|(k, v)| k == "exec" && matches!(v, SysValue::Text(o) if o == command))));
    if !known {
        return Err(format!("'{command}' is none of the installed applications: apps.launch only launches what the `apps` service has reported"));
    }
    let mut child = Command::new("setsid").args(["-f", "sh", "-c", command]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
