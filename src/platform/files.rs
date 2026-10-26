//! Saving and remembering: what a scene needs so its settings survive closing
//! it. Each scene —and each plugin— has **its own folder**, and cannot get out
//! of it: no absolute paths, no `..`, no reading anybody's `.ssh`. It is the same
//! thing `require` does with modules.
//!
//! On Linux, `~/.local/share/pleamar/<who>/`; on Windows, `%APPDATA%`; on macOS,
//! `~/Library/Application Support`.

use super::SysValue;
use std::path::{Path, PathBuf};

/// Where each scene or plugin keeps its things.
fn dir_for(owner: &str) -> PathBuf {
    let var = |v: &str| std::env::var_os(v).filter(|x| !x.is_empty()).map(PathBuf::from);
    let base = if cfg!(target_os = "windows") {
        var("APPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        var("XDG_DATA_HOME").or_else(|| var("HOME").map(|h| h.join(".local/share")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("pleamar").join(owner)
}

/// A file name, not a path: letters, digits, `_`, `-`, `.` and folders inside.
fn path_in(owner: &str, name: &str) -> Result<PathBuf, String> {
    let clean = !name.is_empty()
        && !name.starts_with('/')
        && name.split('/').all(|t| !t.is_empty() && t != ".." && t != "." && t.chars().all(|c| c.is_alphanumeric() || "_-.".contains(c)));
    if !clean {
        return Err(format!("'{name}' is not a name inside this scene's folder: no `..`, no full paths"));
    }
    Ok(dir_for(owner).join(name))
}

/// From JSON to what the logic sees: tables, numbers and texts.
pub(crate) fn from_json(v: serde_json::Value) -> SysValue {
    use serde_json::Value as J;
    match v {
        J::Null => SysValue::Null,
        J::Bool(b) => SysValue::Bool(b),
        J::Number(n) => SysValue::Num(n.as_f64().unwrap_or(0.0)),
        J::String(s) => SysValue::Text(s),
        J::Array(l) => SysValue::List(l.into_iter().map(from_json).collect()),
        J::Object(m) => SysValue::Map(m.into_iter().map(|(k, v)| (k, from_json(v))).collect()),
    }
}

/// And the other way round, to save it.
pub(crate) fn to_json(v: &SysValue) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        SysValue::Null => J::Null,
        SysValue::Bool(b) => J::Bool(*b),
        // An integer is written without a decimal point: inside everything is a
        // floating-point number, but people read this.
        SysValue::Num(n) if n.fract() == 0.0 && n.abs() < 9e15 => J::Number((*n as i64).into()),
        SysValue::Num(n) => serde_json::Number::from_f64(*n).map_or(J::Null, J::Number),
        SysValue::Text(s) => J::String(s.clone()),
        SysValue::List(l) => J::Array(l.iter().map(to_json).collect()),
        SysValue::Map(m) => J::Object(m.iter().map(|(k, v)| (k.clone(), to_json(v))).collect()),
    }
}

/// `sys.ask("files.read", "settings.json")` · `("files.read", n, "json")` ·
/// `("files.list")` · `("files.exists", n)`
pub fn query(owner: &str, what: &str, args: &[SysValue]) -> Result<SysValue, String> {
    match (what, args) {
        // With `"json"` after it, what comes out is a table and not a text. A
        // half-written file is an error with its location, not half a table.
        ("files.read", [SysValue::Text(name), SysValue::Text(format)]) if format == "json" => match std::fs::read_to_string(path_in(owner, name)?) {
            Ok(t) => serde_json::from_str(&t).map(from_json).map_err(|e| format!("'{name}' is not valid JSON: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SysValue::Null),
            Err(e) => Err(format!("cannot read '{name}': {e}")),
        },
        ("files.read", [_, SysValue::Text(other)]) => Err(format!("'{other}' is not a format: the only one is \"json\", and without it you get the text")),
        ("files.read", [SysValue::Text(name)]) => match std::fs::read_to_string(path_in(owner, name)?) {
            Ok(t) => Ok(SysValue::Text(t)),
            // Not being there yet is not an error: it is the first time.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SysValue::Null),
            Err(e) => Err(format!("cannot read '{name}': {e}")),
        },
        ("files.exists", [SysValue::Text(name)]) => Ok(SysValue::Bool(path_in(owner, name)?.exists())),
        ("files.list", []) => {
            let mut names: Vec<String> = std::fs::read_dir(dir_for(owner))
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter(|e| e.path().is_file())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            Ok(SysValue::List(names.into_iter().map(SysValue::Text).collect()))
        }
        ("files.folder", []) => Ok(SysValue::Text(dir_for(owner).to_string_lossy().into_owned())),
        _ => Err(format!("'{what}' is not asked like that: files.read(name[, \"json\"]), files.exists(name), files.list(), files.folder()")),
    }
}

/// Writes `path` (inside `folder`) so that only its owner can read it: on Unix the
/// scene's folder is 0700 and each file 0600. A scene keeps tokens and passwords
/// here; this keeps other users of the machine out, not other programs of the same one.
fn write_private(folder: &Path, path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    // Up to the folder's parent (`~/.local/share/pleamar`), as the system makes them;
    // the folder and whatever is inside it, private.
    if let Some(above) = folder.parent() {
        std::fs::create_dir_all(above)?;
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(path.parent().unwrap_or(folder))?;
    // A folder made before this was private gets closed too: otherwise whoever
    // already had a token there would stay just as exposed.
    #[cfg(unix)]
    if std::fs::metadata(folder)?.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))?;
    }
    // First next to it, then in its place: if the power goes out halfway, the old one
    // is still there. The mode goes on before the rename, so it is never readable.
    let partial = PathBuf::from(format!("{}.new", path.display()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&partial)?;
    // A `.new` left over from a crash keeps its old mode when opened: set it anyway.
    #[cfg(unix)]
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(text.as_bytes())?;
    drop(file);
    std::fs::rename(&partial, path)
}

/// `sys.call("files.write", "settings.json", text)` · `("files.remove", name)`
pub fn command(owner: &str, what: &str, args: &[SysValue]) -> Result<(), String> {
    match (what, args) {
        ("files.write", [SysValue::Text(name), contents]) => {
            let path = path_in(owner, name)?;
            // A table is saved as JSON, with its line breaks: what gets saved
            // also gets read by hand now and then.
            let text = match contents {
                SysValue::Text(t) => t.clone(),
                SysValue::Num(n) => n.to_string(),
                SysValue::Bool(b) => b.to_string(),
                SysValue::Null => String::new(),
                table => serde_json::to_string_pretty(&to_json(table)).map(|t| t + "\n").map_err(|e| format!("cannot write '{name}': {e}"))?,
            };
            write_private(&dir_for(owner), &path, &text).map_err(|e| format!("cannot write '{name}': {e}"))
        }
        ("files.remove", [SysValue::Text(name)]) => match std::fs::remove_file(path_in(owner, name)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("cannot remove '{name}': {e}")),
        },
        _ => Err(format!("'{what}' is not asked like that: files.write(name, text), files.remove(name)")),
    }
}

/// What gets watched: the file changing underneath (another application touched it).
/// It checks its date four times a second, which works the same on all three systems.
pub fn watch(owner: &str, name: &str, dispatch: Box<dyn Fn(SysValue) + Send>) -> bool {
    let Ok(path) = path_in(owner, name) else { return false };
    std::thread::Builder::new()
        .name(format!("file {name}"))
        .spawn(move || {
            let modified = |r: &Path| std::fs::metadata(r).and_then(|m| m.modified()).ok();
            let mut last = None;
            loop {
                let now = modified(&path);
                if now != last {
                    last = now;
                    dispatch(match std::fs::read_to_string(&path) {
                        Ok(t) => SysValue::Text(t),
                        Err(_) => SysValue::Null,
                    });
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        })
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_file_and_its_folder_are_private() {
        let base = std::env::temp_dir().join(format!("pleamar-files-{}", std::process::id()));
        let folder = base.join("scene");
        let path = folder.join("deep/secret.json");
        write_private(&folder, &path, "token").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "token");
        assert!(!PathBuf::from(format!("{}.new", path.display())).exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&folder), 0o700);
            assert_eq!(mode(&folder.join("deep")), 0o700);
            assert_eq!(mode(&path), 0o600);
            // A folder that was already open gets closed, and rewriting keeps the file private.
            std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            write_private(&folder, &path, "other").unwrap();
            assert_eq!(mode(&folder), 0o700);
            assert_eq!(mode(&path), 0o600);
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "other");
        std::fs::remove_dir_all(&base).unwrap();
    }
}
