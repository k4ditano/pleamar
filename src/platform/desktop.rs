//! What a freedesktop desktop knows about itself without asking any
//! compositor: which applications are installed, and how to launch one.

use super::SysValue;
use std::collections::BTreeMap;

/// One `.desktop` file, as the `apps` service reports it.
#[derive(Debug, Default, PartialEq)]
struct App {
    name: String,
    exec: String,
    icon: String,
    id: String,
    wmclass: String,
    local_name: String,
    categories: String,
    generic_name: String,
    comment: String,
    keywords: String,
    terminal: bool,
    actions: Vec<Action>,
}

/// One of its `[Desktop Action <id>]` groups: Firefox's «New Private Window».
#[derive(Debug, Default, PartialEq)]
struct Action {
    id: String,
    name: String,
    local_name: String,
    exec: String,
    icon: String,
}

/// The `.desktop` files of every applications folder, without the hidden ones.
pub fn applications() -> SysValue {
    SysValue::List(installed().into_iter().map(App::value).collect())
}

/// Every applications folder, the user's first.
fn folders() -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    let data_home = std::env::var("XDG_DATA_HOME").ok().filter(|d| !d.is_empty()).unwrap_or(format!("{home}/.local/share"));
    let mut folders = vec![format!("{data_home}/applications")];
    folders.extend(data_dirs.split(':').map(|d| format!("{d}/applications")));
    folders
}

/// What changes when an app is installed, removed or its file rewritten: each
/// `.desktop` file's name, size and time, without reading any of them.
pub fn signature() -> u64 {
    signature_of(&folders())
}

fn signature_of(folders: &[String]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for folder in folders {
        folder.hash(&mut h);
        let Ok(dir) = std::fs::read_dir(folder) else { continue };
        let mut files: Vec<_> = dir
            .filter_map(Result::ok)
            .filter(|f| f.path().extension().is_some_and(|e| e == "desktop"))
            .map(|f| {
                let m = std::fs::metadata(f.path()).ok();
                (f.file_name(), m.as_ref().map(|m| m.len()), m.and_then(|m| m.modified().ok()))
            })
            .collect();
        files.sort();
        files.hash(&mut h);
    }
    h.finish()
}

/// The same, in alphabetical order, as `launch` checks against.
fn installed() -> Vec<App> {
    let folders = folders();
    // `es_ES.UTF-8` → `Name[es_ES]`, then `Name[es]`.
    let lang = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|v| std::env::var(v).ok()).find(|v| !v.is_empty()).unwrap_or_default();
    let lang = lang.split(['.', '@']).next().unwrap_or("").to_owned();
    // By name, and the first folder wins: the user's overrides the system's.
    let mut apps: BTreeMap<String, App> = BTreeMap::new();
    for folder in folders {
        let Ok(dir) = std::fs::read_dir(&folder) else { continue };
        for f in dir.filter_map(Result::ok).filter(|f| f.path().extension().is_some_and(|e| e == "desktop")) {
            let Ok(text) = std::fs::read_to_string(f.path()) else { continue };
            // `org.gnome.Calculator.desktop` is `org.gnome.Calculator`: the
            // app id its windows say, so a window can be told whose it is.
            let id = f.path().file_stem().and_then(|s| s.to_str()).unwrap_or("").to_owned();
            if let Some(a) = parse(&text, &id, &lang) {
                apps.entry(a.name.clone()).or_insert(a);
            }
        }
    }
    apps.into_values().collect()
}

/// One file's text, read in the user's language (`es_ES`, or "" for none);
/// `None` when it is not an application to show.
fn parse(text: &str, id: &str, lang: &str) -> Option<App> {
    // Every group's keys, the first of each: `[Desktop Entry]` and the `[Desktop Action <id>]`s.
    let mut groups: BTreeMap<&str, BTreeMap<&str, &str>> = BTreeMap::new();
    let mut group = "";
    for l in text.lines() {
        let l = l.trim_end();
        if let Some(g) = l.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            group = g;
        } else if let Some((k, v)) = l.split_once('=').filter(|_| !l.starts_with('#')) {
            groups.entry(group).or_default().entry(k.trim_end()).or_insert(v.trim_start());
        }
    }
    let short = lang.split('_').next().unwrap_or("");
    // `Name[es_ES]`, then `Name[es]`, then `Name`.
    let local = |g: &BTreeMap<&str, &str>, key: &str| {
        [format!("{key}[{lang}]"), format!("{key}[{short}]")].iter().filter(|_| !lang.is_empty()).find_map(|k| g.get(k.as_str())).or(g.get(key)).map(|v| v.to_string())
    };
    let get = |g: &BTreeMap<&str, &str>, key: &str| g.get(key).map_or(String::new(), |v| v.to_string());
    let e = groups.get("Desktop Entry")?;
    if e.get("Type").is_some_and(|t| *t != "Application") || e.get("NoDisplay") == Some(&"true") || e.get("Hidden") == Some(&"true") {
        return None;
    }
    // Only the ones `Actions=` lists, in its order, and only with something to run.
    let actions = get(e, "Actions")
        .split(';')
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .filter_map(|a| {
            let g = groups.get(format!("Desktop Action {a}").as_str())?;
            Some(Action { id: a.to_owned(), name: g.get("Name")?.to_string(), local_name: local(g, "Name").unwrap_or_default(), exec: command(g.get("Exec")?), icon: get(g, "Icon") })
        })
        .collect();
    Some(App {
        name: e.get("Name")?.to_string(),
        exec: command(e.get("Exec")?),
        icon: get(e, "Icon"),
        id: id.to_owned(),
        wmclass: get(e, "StartupWMClass"),
        local_name: local(e, "Name").unwrap_or_default(),
        categories: get(e, "Categories"),
        generic_name: local(e, "GenericName").unwrap_or_default(),
        comment: local(e, "Comment").unwrap_or_default(),
        keywords: local(e, "Keywords").unwrap_or_default(),
        terminal: e.get("Terminal") == Some(&"true"),
        actions,
    })
}

/// `Exec=firefox %u` → `firefox`: the field codes are for a file manager, not for us.
fn command(exec: &str) -> String {
    exec.split_whitespace().filter(|p| !p.starts_with('%')).collect::<Vec<_>>().join(" ")
}

impl App {
    fn value(self) -> SysValue {
        let text = |s: String| SysValue::Text(s);
        SysValue::Map(vec![
            ("name".into(), text(self.name)),
            ("exec".into(), text(self.exec)),
            ("icon".into(), text(self.icon)),
            ("id".into(), text(self.id)),
            ("wmclass".into(), text(self.wmclass)),
            // `Name[es]`, as the user's language says it (the name itself if it has none).
            ("local_name".into(), text(self.local_name)),
            // `Categories=`, as written: `Utility;Calculator;`.
            ("categories".into(), text(self.categories)),
            // These three in the user's language, like `local_name`, and "" when it has none:
            // what to search besides the name («Web Browser», `Internet;WWW;Browser;Web;`).
            ("generic_name".into(), text(self.generic_name)),
            ("comment".into(), text(self.comment)),
            ("keywords".into(), text(self.keywords)),
            // `Terminal=true`: it runs in a terminal (`btop`), and `apps.launch` opens one for it.
            ("terminal".into(), SysValue::Bool(self.terminal)),
            ("actions".into(), SysValue::List(
                self.actions
                    .into_iter()
                    .map(|a| SysValue::Map(vec![
                        ("id".into(), text(a.id)),
                        ("name".into(), text(a.name)),
                        ("local_name".into(), text(a.local_name)),
                        ("exec".into(), text(a.exec)),
                        ("icon".into(), text(a.icon)),
                    ]))
                    .collect(),
            )),
        ])
    }
}

/// Launches a command and forgets about it: it is not a child of the logic, and does not die with us.
///
/// Only what really is an installed application or one of its actions:
/// otherwise, `apps.launch` would be a way to run anything with the
/// permission to "see the apps".
pub fn launch(command: &str) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let apps: Vec<App> = installed().into_iter().filter(|a| a.exec == command || a.actions.iter().any(|x| x.exec == command)).collect();
    if apps.is_empty() {
        return Err(format!("'{command}' is none of the installed applications: apps.launch only launches what the `apps` service has reported"));
    }
    let mut launch = Command::new("setsid");
    crate::child_env(&mut launch);
    launch.arg("-f");
    // On its own it would have nothing to talk to (its stdin is /dev/null):
    // the user's terminal runs it, the one xdg-terminal-exec says. If two
    // entries run the same command and one wants a terminal, it gets one.
    if let Some(app) = apps.iter().find(|a| a.terminal) {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::var_os("PATH").unwrap_or_default();
        let runnable = |d: std::path::PathBuf| std::fs::metadata(d.join("xdg-terminal-exec")).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
        if !std::env::split_paths(&path).any(runnable) {
            return Err(format!("'{}' runs in a terminal, and there is no xdg-terminal-exec to open one", app.name));
        }
        launch.arg("xdg-terminal-exec");
    }
    let command = &with_accessibility(command);
    let mut child = launch.args(["sh", "-c", command]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Launchers of Electron apps that are scripts: the program they start is found
/// at run time, so the folder of the file named says nothing.
const ELECTRON_SCRIPTS: &[&str] = &["discord"];

/// Whether `program` (as an `Exec=` names it) is an Electron app: next to it,
/// or next to where it links, are the files every one ships with.
fn is_electron(program: &str) -> bool {
    let path = std::path::Path::new(program);
    let found = if program.contains('/') {
        Some(path.to_path_buf())
    } else {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).map(|d| d.join(program)).find(|p| p.is_file())
    };
    let Some(file) = found.and_then(|f| std::fs::canonicalize(f).ok()) else { return false };
    if file.file_name().and_then(|n| n.to_str()).is_some_and(|n| ELECTRON_SCRIPTS.contains(&n)) {
        return true;
    }
    let shipped = file.parent().is_some_and(|d| d.join("chrome_100_percent.pak").is_file() && d.join("libffmpeg.so").is_file());
    // A script that starts the Electron the distribution packages.
    let script = std::fs::read(&file).ok().filter(|t| t.starts_with(b"#!") && t.len() < 64 * 1024).is_some_and(|t| String::from_utf8_lossy(&t).contains("/usr/lib/electron"));
    shipped || script
}

/// Electron builds the tree of a page only if told to (it does not look at
/// the accessibility bus): with pleamar-wm's bus on, the apps it recognises
/// start with that said. What an `Exec=` writes after the program is kept.
fn with_accessibility(command: &str) -> String {
    const FLAG: &str = "--force-renderer-accessibility";
    // Only pleamar-wm's own bus: another desktop's, or none, is not ours to change.
    let ours = std::env::var("AT_SPI_BUS_ADDRESS").is_ok_and(|v| v.contains("pleamar-a11y"));
    if !ours || command.contains(FLAG) {
        return command.to_owned();
    }
    let (program, rest) = command.split_once(' ').unwrap_or((command, ""));
    if program.is_empty() || program.contains(['\'', '"', '=', '$']) || !is_electron(program) {
        return command.to_owned();
    }
    format!("{program} {FLAG} {rest}").trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installing_or_removing_an_app_changes_the_signature() {
        let dir = std::env::temp_dir().join(format!("pleamar-apps-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let folders = [dir.display().to_string()];
        let empty = signature_of(&folders);
        assert_eq!(signature_of(&folders), empty);
        std::fs::write(dir.join("notes.txt"), "not an app").unwrap();
        assert_eq!(signature_of(&folders), empty);
        std::fs::write(dir.join("new.desktop"), "[Desktop Entry]\nName=New\nExec=new\n").unwrap();
        let one = signature_of(&folders);
        assert_ne!(one, empty);
        std::fs::write(dir.join("new.desktop"), "[Desktop Entry]\nName=Newer\nExec=new\n").unwrap();
        assert_ne!(signature_of(&folders), one);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_ne!(signature_of(&folders), one);
    }

    #[test]
    fn electron_gets_its_flag_only_when_it_is_one() {
        let dir = std::env::temp_dir().join(format!("pleamar-electron-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["chrome_100_percent.pak", "libffmpeg.so", "app"] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        std::fs::write(dir.join("plain"), "x").unwrap();
        let app = dir.join("app");
        let plain = dir.join("plain");
        // SAFETY: only this test touches the variable.
        unsafe { std::env::set_var("AT_SPI_BUS_ADDRESS", "unix:path=/run/user/1/pleamar-a11y-x") };
        assert_eq!(with_accessibility(&format!("{} --url", app.display())), format!("{} --force-renderer-accessibility --url", app.display()));
        assert_eq!(with_accessibility(&app.display().to_string()), format!("{} --force-renderer-accessibility", app.display()));
        // Already said, or not Electron (a folder with another app's files has none).
        assert_eq!(with_accessibility(&format!("{} --force-renderer-accessibility", app.display())), format!("{} --force-renderer-accessibility", app.display()));
        std::fs::remove_file(dir.join("libffmpeg.so")).unwrap();
        assert_eq!(with_accessibility(&format!("{} a", app.display())), format!("{} a", app.display()));
        assert_eq!(with_accessibility(&plain.display().to_string()), plain.display().to_string());
        // Another desktop's bus is not ours.
        unsafe { std::env::set_var("AT_SPI_BUS_ADDRESS", "unix:path=/run/user/1/at-spi/bus") };
        assert_eq!(with_accessibility(&app.display().to_string()), app.display().to_string());
        // Off with the bus.
        unsafe { std::env::remove_var("AT_SPI_BUS_ADDRESS") };
        assert_eq!(with_accessibility(&app.display().to_string()), app.display().to_string());
        for f in ["chrome_100_percent.pak", "app", "plain"] {
            std::fs::remove_file(dir.join(f)).unwrap();
        }
        std::fs::remove_dir(&dir).unwrap();
    }

    const FIREFOX: &str = "[Desktop Entry]
Name=Firefox
Name[es]=Firefox
GenericName=Web Browser
GenericName[es]=Navegador web
Comment=Browse the World Wide Web
Keywords=Internet;WWW;Browser;Web;
Exec=/usr/lib/firefox/firefox %u
Icon=firefox
Type=Application
Categories=Network;WebBrowser;
Actions=new-window;new-private-window;

[Desktop Action new-private-window]
Name=New Private Window
Name[es]=Nueva ventana privada
Exec=/usr/lib/firefox/firefox --private-window %u

[Desktop Action new-window]
Name=New Window
Exec=/usr/lib/firefox/firefox --new-window %u

[Desktop Action profile-manager]
Name=Profile Manager
Exec=/usr/lib/firefox/firefox --ProfileManager
";

    #[test]
    fn an_entry_says_what_to_search_in_the_users_language() {
        let a = parse(FIREFOX, "firefox", "es_ES").unwrap();
        assert_eq!((a.name.as_str(), a.exec.as_str(), a.local_name.as_str()), ("Firefox", "/usr/lib/firefox/firefox", "Firefox"));
        // In the user's language where it has one, and the plain key where it does not.
        assert_eq!((a.generic_name.as_str(), a.comment.as_str(), a.keywords.as_str()), ("Navegador web", "Browse the World Wide Web", "Internet;WWW;Browser;Web;"));
        assert_eq!(parse(FIREFOX, "firefox", "").unwrap().generic_name, "Web Browser");
    }

    #[test]
    fn an_entry_brings_the_actions_it_lists_in_that_order() {
        let a = parse(FIREFOX, "firefox", "es_ES").unwrap();
        // A group `Actions=` does not list is not one.
        let ids: Vec<_> = a.actions.iter().map(|x| (x.id.as_str(), x.local_name.as_str(), x.exec.as_str())).collect();
        assert_eq!(ids, [("new-window", "New Window", "/usr/lib/firefox/firefox --new-window"), ("new-private-window", "Nueva ventana privada", "/usr/lib/firefox/firefox --private-window")]);
        assert_eq!(parse(FIREFOX, "firefox", "").unwrap().actions[1].local_name, "New Private Window");
    }

    #[test]
    fn a_terminal_entry_says_so() {
        assert!(!parse(FIREFOX, "firefox", "").unwrap().terminal);
        let btop = parse("[Desktop Entry]\nType=Application\nName=btop++\nExec=btop\nTerminal=true\n", "btop", "").unwrap();
        assert!(btop.terminal && btop.actions.is_empty() && btop.keywords.is_empty());
    }

    #[test]
    fn a_hidden_entry_or_a_link_is_no_app() {
        assert_eq!(parse("[Desktop Entry]\nName=Hidden\nExec=x\nNoDisplay=true\n", "x", ""), None);
        assert_eq!(parse("[Desktop Entry]\nName=A link\nType=Link\nURL=https://example.org\n", "x", ""), None);
    }

    #[test]
    fn a_header_with_spaces_after_it_still_starts_its_group() {
        let a = parse("[Desktop Entry]\nName=A\nExec=a\nActions=b;\n[Desktop Action b] \nName=B\nExec=a --b\n", "a", "").unwrap();
        assert_eq!((a.exec.as_str(), a.actions.len()), ("a", 1));
    }

    #[test]
    fn spaces_in_the_actions_list_do_not_lose_one() {
        let a = parse("[Desktop Entry]\nName=A\nExec=a\nActions=b; c;\n[Desktop Action b]\nName=B\nExec=a --b\n[Desktop Action c]\nName=C\nExec=a --c\n", "a", "").unwrap();
        assert_eq!(a.actions.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["b", "c"]);
    }
}
