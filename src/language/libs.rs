//! Installed scene libraries: `import "lib:name/file"`, looked for along a search path.
//!
//! A library is a folder `name/` with a manifest, `name/pleamar-lib`, that says its name,
//! its version and the language it needs:
//!
//! ```text
//! name orilla
//! version 1.3.0
//! language 0.11
//! ```
//!
//! Inside, ordinary `library Name { … }` files (and their `.luau`, if they have logic).
//! `import "lib:orilla/tokens"` is `orilla/tokens.plm`, and `version "1.2"` after it asks
//! for 1.2 or later, without changing the first number (`0.3` asks for 0.3.x: while the
//! first is 0, the second is the one that breaks). pleamar knows no library by name: it
//! only looks where it is told, in this order:
//!
//! 1. `--lib DIR`, as many as said, in their order;
//! 2. `PLEAMAR_LIB_PATH`, folders separated by `:`;
//! 3. `lib/` next to the main scene (the copy an app carries with it);
//! 4. `$XDG_DATA_HOME/pleamar/lib` (by default `~/.local/share/pleamar/lib`);
//! 5. each `$XDG_DATA_DIRS/pleamar/lib` (by default `/usr/local/share` and `/usr/share`).
//!
//! The first folder that has `name/` is the one used, whatever its version: a library is
//! not chosen among several by version; the one found is checked, and its error says where
//! it was found, so it is clear which copy won.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The manifest's file name.
pub const MANIFEST: &str = "pleamar-lib";

/// `--lib DIR`, in the order given on the command line.
static FROM_COMMAND_LINE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// `--lib DIR`: looked in before anywhere else.
pub fn add_dir(dir: impl Into<PathBuf>) {
    FROM_COMMAND_LINE.lock().unwrap().push(dir.into());
}

/// Where libraries are looked for, in order (see the module), for a scene whose main file
/// is in `scene_dir`. Each with where it comes from, for the errors.
pub fn search_path(scene_dir: &Path) -> Vec<(PathBuf, &'static str)> {
    search_path_with(&FROM_COMMAND_LINE.lock().unwrap(), scene_dir)
}

fn search_path_with(command_line: &[PathBuf], scene_dir: &Path) -> Vec<(PathBuf, &'static str)> {
    let mut dirs: Vec<(PathBuf, &'static str)> = command_line.iter().map(|d| (d.clone(), "--lib")).collect();
    if let Ok(v) = std::env::var("PLEAMAR_LIB_PATH") {
        dirs.extend(v.split(':').filter(|d| !d.is_empty()).map(|d| (PathBuf::from(d), "PLEAMAR_LIB_PATH")));
    }
    dirs.push((scene_dir.join("lib"), "next to the scene"));
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let data_home = env("XDG_DATA_HOME").map(PathBuf::from).or_else(|| env("HOME").map(|h| Path::new(&h).join(".local/share")));
    if let Some(d) = data_home {
        dirs.push((d.join("pleamar/lib"), "XDG_DATA_HOME"));
    }
    let data_dirs = env("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    dirs.extend(data_dirs.split(':').filter(|d| !d.is_empty()).map(|d| (Path::new(d).join("pleamar/lib"), "XDG_DATA_DIRS")));
    dirs
}

/// A version, `1.3.0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// `1.3.0`, `1.3` or `1` (what is missing is 0); `None` if it is not that.
pub fn parse_version(s: &str) -> Option<(Version, usize)> {
    let parts: Vec<&str> = s.trim().split('.').collect();
    if parts.is_empty() || parts.len() > 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let n = |i: usize| parts.get(i).map_or(Some(0), |p| p.parse().ok());
    Some((Version(n(0)?, n(1)?, n(2)?), parts.len()))
}

/// Whether `have` satisfies `version "asked"`: at least it, and the same «compatible» line
/// (the first number; while it is 0, the second; while both are 0, the third).
pub fn satisfies(have: Version, asked: Version, said: usize) -> bool {
    if have < asked {
        return false;
    }
    match (asked.0, asked.1) {
        (0, 0) if said == 3 => have.0 == 0 && have.1 == 0 && have.2 == asked.2,
        (0, 0) if said == 2 => have.0 == 0 && have.1 == 0,
        (0, _) if said >= 2 => have.0 == 0 && have.1 == asked.1,
        _ => have.0 == asked.0,
    }
}

/// What a manifest says.
#[derive(Debug, Clone)]
pub struct Manifest {
    pub name: String,
    pub version: Version,
    pub language: (u32, u32),
}

/// Reads `dir/pleamar-lib`; the error says what is wrong in it.
pub fn read_manifest(dir: &Path) -> Result<Manifest, String> {
    let file = dir.join(MANIFEST);
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{} has no manifest ({}): {e}. A library is a folder with a '{MANIFEST}' file that says `name`, `version` and `language`", dir.display(), file.display()))?;
    let (mut name, mut version, mut language) = (None, None, None);
    for (k, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let bad = |what: &str| format!("{}:{}: {what}", file.display(), k + 1);
        let (key, value) = line.split_once(char::is_whitespace).map(|(a, b)| (a, b.trim())).ok_or_else(|| bad("each line is `key value`: name, version or language"))?;
        match key {
            "name" => name = Some(value.to_owned()),
            "version" => version = Some(parse_version(value).filter(|(_, n)| *n == 3).map(|(v, _)| v).ok_or_else(|| bad("the version is three numbers, like `version 1.3.0`"))?),
            "language" => {
                let (v, n) = parse_version(value).ok_or_else(|| bad("the language is like `language 0.11`"))?;
                if n != 2 {
                    return Err(bad("the language is like `language 0.11`"));
                }
                language = Some((v.0, v.1));
            }
            other => return Err(bad(&format!("unknown key '{other}': a manifest says name, version and language"))),
        }
    }
    let missing = |k: &str| format!("{} does not say its {k}", file.display());
    Ok(Manifest { name: name.ok_or_else(|| missing("name"))?, version: version.ok_or_else(|| missing("version"))?, language: language.ok_or_else(|| missing("language"))? })
}

/// A library resolved: the file to read and the library it belongs to.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub file: PathBuf,
    pub manifest: Manifest,
}

/// `lib:orilla/tokens` (without `lib:`) with `version`, if said, for a scene in `scene_dir`.
pub fn resolve(spec: &str, version: Option<&str>, scene_dir: &Path, language: (u32, u32)) -> Result<Resolved, String> {
    let command_line = FROM_COMMAND_LINE.lock().unwrap().clone();
    resolve_with(&command_line, spec, version, scene_dir, language)
}

fn resolve_with(command_line: &[PathBuf], spec: &str, version: Option<&str>, scene_dir: &Path, language: (u32, u32)) -> Result<Resolved, String> {
    let (name, sub) = spec.split_once('/').ok_or_else(|| format!("say which file of the library: `lib:{spec}/tokens` is `{spec}/tokens.plm`"))?;
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
        return Err(format!("'{name}' is not a library name: lowercase letters, digits and '-'"));
    }
    // Inside the library and nowhere else: no way up, no absolute path, no empty step.
    if sub.is_empty() || sub.starts_with('/') || sub.contains('\\') || sub.split('/').any(|p| p.is_empty() || p == "." || p == "..") {
        return Err(format!("'lib:{spec}': the path inside a library goes down from it, without '..' or '/' at the start"));
    }
    if sub.ends_with(".plm") {
        return Err(format!("'lib:{spec}': without '.plm' (it is added): `lib:{name}/{}`", sub.trim_end_matches(".plm")));
    }
    let asked = match version {
        Some(v) => Some(parse_version(v).ok_or_else(|| format!("version \"{v}\": a version is like \"1.2\" or \"1.2.3\""))?),
        None => None,
    };
    let path = search_path_with(command_line, scene_dir);
    let Some((root, _)) = path.iter().map(|(d, from)| (d.join(name), *from)).find(|(d, _)| d.is_dir()) else {
        let looked: Vec<String> = path.iter().map(|(d, from)| format!("{} ({from})", d.display())).collect();
        return Err(format!("cannot find the library '{name}': looked in {}. Install it there, or say where with --lib DIR or PLEAMAR_LIB_PATH", looked.join(", ")));
    };
    let manifest = read_manifest(&root)?;
    if manifest.name != name {
        return Err(format!("{} says it is '{}', and it is imported as '{name}': the folder and the name in '{MANIFEST}' are the same", root.join(MANIFEST).display(), manifest.name));
    }
    if manifest.language.0 != language.0 || manifest.language.1 > language.1 {
        return Err(format!("{name} {} (in {}) needs the language {}.{}, and this pleamar understands {}.{}", manifest.version, root.display(), manifest.language.0, manifest.language.1, language.0, language.1));
    }
    if let Some((v, said)) = asked {
        if !satisfies(manifest.version, v, said) {
            let why = if manifest.version < v { "older: update it" } else { "of another line: it may not be compatible" };
            return Err(format!("the scene asks for {name} {} and the one in {} is {} ({why})", version.unwrap_or_default(), root.display(), manifest.version));
        }
    }
    let file = root.join(format!("{sub}.plm"));
    if !file.is_file() {
        return Err(format!("the library '{name}' {} (in {}) has no '{sub}.plm'", manifest.version, root.display()));
    }
    let file = file.canonicalize().map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(Resolved { file, manifest })
}

/// The library a file belongs to, if it is in one: the nearest folder above it with a
/// manifest. For `--check` (what was resolved and from where) and `pleamar lib vendor`.
pub fn library_of(file: &Path) -> Option<(PathBuf, Manifest)> {
    let mut dir = file.parent();
    while let Some(d) = dir {
        if d.join(MANIFEST).is_file() {
            return read_manifest(d).ok().map(|m| (d.to_owned(), m));
        }
        dir = d.parent();
    }
    None
}

/// The libraries among the files a compilation read, once each, in the order they were read.
pub fn libraries_read(files: &[PathBuf]) -> Vec<(PathBuf, Manifest)> {
    let mut out: Vec<(PathBuf, Manifest)> = Vec::new();
    for f in files {
        if let Some((root, m)) = library_of(f) {
            if !out.iter().any(|(r, _)| *r == root) {
                out.push((root, m));
            }
        }
    }
    out
}

/// `pleamar lib vendor SCENE --to DIR`: copies each library the scene resolves, whole, to
/// `DIR/name/`, replacing what was there. What it copied, with versions.
pub fn vendor(files: &[PathBuf], to: &Path) -> Result<Vec<(String, Version)>, String> {
    let mut done = Vec::new();
    for (root, m) in libraries_read(files) {
        let target = to.join(&m.name);
        if target.canonicalize().ok() == root.canonicalize().ok() {
            done.push((m.name, m.version));
            continue;
        }
        if target.exists() {
            if !target.join(MANIFEST).is_file() {
                return Err(format!("{} exists and is not a library (no '{MANIFEST}'): not overwritten", target.display()));
            }
            std::fs::remove_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
        }
        copy_dir(&root, &target).map_err(|e| format!("copying {} to {}: {e}", root.display(), target.display()))?;
        done.push((m.name, m.version));
    }
    Ok(done)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let name = e.file_name();
        // Not what a repository or an editor leaves.
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let kind = std::fs::metadata(e.path())?;
        if kind.is_dir() {
            copy_dir(&e.path(), &to.join(&name))?;
        } else {
            std::fs::copy(e.path(), to.join(&name))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(dir: &Path, name: &str, version: &str, language: &str) -> PathBuf {
        let root = dir.join(name);
        std::fs::create_dir_all(root.join("parts")).unwrap();
        std::fs::write(root.join(MANIFEST), format!("# a test library\nname {name}\nversion {version}\nlanguage {language}\n")).unwrap();
        std::fs::write(root.join("tokens.plm"), "library Tokens {\n  let ink = #112233\n}\n").unwrap();
        std::fs::write(root.join("parts/button.plm"), "library Button {\n  let r = 8\n}\n").unwrap();
        root
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pleamar-libs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn versions_are_compatible_within_their_line() {
        let v = |s: &str| parse_version(s).unwrap();
        let ok = |have: &str, asked: &str| { let (a, n) = v(asked); satisfies(v(have).0, a, n) };
        assert!(ok("1.3.0", "1.2") && ok("1.2.0", "1.2") && ok("1.9.9", "1"));
        assert!(!ok("1.1.9", "1.2"), "older");
        assert!(!ok("2.0.0", "1.2"), "another first number");
        assert!(ok("0.3.5", "0.3") && !ok("0.4.0", "0.3"), "while the first is 0, the second breaks");
        assert!(ok("0.0.4", "0.0.4") && !ok("0.0.5", "0.0.4"));
        assert!(parse_version("1.a").is_none() && parse_version("1.2.3.4").is_none() && parse_version("").is_none());
    }

    #[test]
    fn a_library_is_found_along_the_path_in_its_order() {
        let d = tmp("order");
        let (first, second, scene) = (d.join("first"), d.join("second"), d.join("app"));
        library(&first, "tide", "1.0.0", "0.11");
        library(&second, "tide", "2.0.0", "0.11");
        library(&scene.join("lib"), "shore", "0.2.1", "0.11");
        let cli = [first.clone(), second.clone()];
        // `--lib` before a `lib/` next to the scene, and the first `--lib` before the next.
        let r = resolve_with(&cli, "tide/tokens", None, &scene, (0, 11)).unwrap();
        assert_eq!(r.manifest.version, Version(1, 0, 0));
        assert!(r.file.ends_with("first/tide/tokens.plm"));
        let r = resolve_with(&cli, "shore/parts/button", Some("0.2"), &scene, (0, 11)).unwrap();
        assert!(r.file.ends_with("app/lib/shore/parts/button.plm"), "{:?}", r.file);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn what_is_wrong_is_said() {
        let d = tmp("errors");
        let scene = d.join("app");
        library(&scene.join("lib"), "tide", "1.1.0", "0.11");
        library(&scene.join("lib"), "future", "1.0.0", "0.12");
        let err = |spec: &str, v: Option<&str>| resolve_with(&[], spec, v, &scene, (0, 11)).unwrap_err();
        assert!(err("nowhere/tokens", None).contains("cannot find the library 'nowhere': looked in"));
        assert!(err("tide/missing", None).contains("has no 'missing.plm'"));
        assert!(err("tide/tokens", Some("1.2")).contains("asks for tide 1.2") && err("tide/tokens", Some("1.2")).contains("update it"));
        assert!(err("tide/tokens", Some("2")).contains("1.1.0"));
        assert!(err("future/tokens", None).contains("needs the language 0.12"));
        assert!(err("tide/tokens", Some("x")).contains("a version is like"));
        // Nothing outside the library.
        for bad in ["tide/../secret", "tide/./tokens", "tide//tokens", "tide/a\\b", "/etc/passwd", "../tide/tokens", "Tide/tokens", "tide"] {
            assert!(resolve_with(&[], bad, None, &scene, (0, 11)).is_err(), "{bad} should be refused");
        }
        assert!(err("tide/../tokens", None).contains("without '..'"));
        // A manifest that does not say its name, or says another.
        std::fs::write(scene.join("lib/tide").join(MANIFEST), "name wave\nversion 1.1.0\nlanguage 0.11\n").unwrap();
        assert!(err("tide/tokens", None).contains("says it is 'wave'"));
        std::fs::write(scene.join("lib/tide").join(MANIFEST), "version 1.1\n").unwrap();
        assert!(err("tide/tokens", None).contains("three numbers"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn vendor_copies_whole_libraries_and_nothing_else() {
        let d = tmp("vendor");
        let root = library(&d.join("installed"), "tide", "1.4.0", "0.11");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git/HEAD"), "x").unwrap();
        let to = d.join("app/lib");
        let got = vendor(&[d.join("scene.plm"), root.join("tokens.plm")], &to).unwrap();
        assert_eq!(got, vec![("tide".to_owned(), Version(1, 4, 0))]);
        assert!(to.join("tide/parts/button.plm").is_file() && to.join("tide").join(MANIFEST).is_file());
        assert!(!to.join("tide/.git").exists(), "not what a repository leaves");
        // A folder in the way that is not a library is not overwritten.
        std::fs::create_dir_all(d.join("app2/lib/tide")).unwrap();
        assert!(vendor(&[root.join("tokens.plm")], &d.join("app2/lib")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
