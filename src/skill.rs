//! What pleamar teaches the AI agents installed on this machine, and the
//! documentation it carries with it.
//!
//! `pleamar --install-skill` writes its skill (`skill/` in the source) for each
//! agent it finds —Claude Code, Codex, OpenCode— in their own folder of skills.
//! Every run of pleamar looks, quietly, whether a skill it wrote is of another
//! version and writes it again: the skill always speaks of the pleamar that is
//! installed. A skill in that place that pleamar did not write is left alone.

const SKILL: &str = include_str!("../skill/SKILL.md");
const MEASURING: &str = include_str!("../skill/measuring.md");
/// And a second one, for using the desktop: pleamar-wm's agent hands
/// (`pleamar-wm agent …`). Apart, so an agent asked to do something in a
/// browser finds it, and one asked to write a scene finds the other.
const DESKTOP: &str = include_str!("../skill/desktop/SKILL.md");

/// The documentation of this version, by topic.
const DOCS: &[(&str, &str, &str)] = &[
    ("reference", "the language: grammar, every element and property, rules, layers, gestures, services, files", include_str!("../docs/11-language-reference.md")),
    ("guide", "from zero to a bar, six steps", include_str!("../docs/guide.md")),
    ("recipes", "whole scenes that work", include_str!("../docs/recipes.md")),
    ("logic", "what the .luau logic can and cannot do", include_str!("../docs/10-luau-logic.md")),
    ("measuring", "measuring what a scene costs and how long its animations last", MEASURING),
];

/// Which skill this is: pleamar's version and a fingerprint of what the skill
/// says, so a change to it counts even without a new version.
fn stamp() -> String {
    let h = SKILL.bytes().chain(MEASURING.bytes()).chain(DESKTOP.bytes()).fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    format!("{}+{:08x}", env!("CARGO_PKG_VERSION"), h as u32)
}

/// The mark that says a skill is pleamar's own, and of which one.
fn mark() -> String {
    format!("<!-- pleamar skill {}.", stamp())
}

pub fn docs(topic: &str) -> i32 {
    match DOCS.iter().find(|(name, _, _)| *name == topic) {
        Some((_, _, text)) => {
            // (Into a pipe that closes early —`| head`— without a fuss.)
            let _ = std::io::Write::write_all(&mut std::io::stdout(), text.as_bytes());
            0
        }
        None => {
            if !topic.is_empty() {
                eprintln!("there is no «{topic}»:");
            }
            for (name, what, _) in DOCS {
                println!("  pleamar --docs {name:<10} {what}");
            }
            i32::from(!topic.is_empty())
        }
    }
}

/// Where each agent keeps its skills, if the agent is installed (its folder exists).
fn places() -> Vec<(&'static str, std::path::PathBuf)> {
    let home = std::env::var_os("HOME").filter(|v| !v.is_empty());
    #[cfg(target_os = "windows")]
    let home = home.or_else(|| std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()));
    let Some(home) = home.map(std::path::PathBuf::from) else { return Vec::new() };
    let config = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".config"));
    [
        ("Claude Code", home.join(".claude"), "skills"),
        ("Codex", home.join(".codex"), "skills"),
        ("OpenCode", config.join("opencode"), "skill"),
    ]
    .into_iter()
    .filter(|(_, root, _)| root.is_dir())
    .map(|(agent, root, sub)| (agent, root.join(sub).join("pleamar")))
    .collect()
}

/// Writes the skill for every agent installed. `loud`: says what it did, and
/// writes it even where it is up to date. Never over a skill pleamar did not write.
pub fn install(loud: bool) -> i32 {
    let skill = SKILL.replace("{VERSION}", &stamp());
    let places = places();
    if places.is_empty() && loud {
        println!("skill · no agent found (Claude Code, Codex, OpenCode)");
    }
    for (agent, dir) in places {
        let file = dir.join("SKILL.md");
        let before = std::fs::read_to_string(&file).ok();
        match &before {
            Some(text) if !text.contains("<!-- pleamar skill ") => {
                if loud {
                    println!("skill · {agent}: {} is someone else's: left as it is", file.display());
                }
                continue;
            }
            Some(text) if text.contains(&mark()) && !loud => continue,
            _ => {}
        }
        let written = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&file, &skill)).and_then(|_| std::fs::write(dir.join("measuring.md"), MEASURING));
        match written {
            Ok(()) if loud || before.is_some() => println!("skill · {agent}: {}", file.display()),
            Ok(()) => {}
            Err(e) => eprintln!("skill · {agent}: {}: {e}", file.display()),
        }
        // The desktop protocol needs pleamar-wm's independent Wayland seat.
        // Do not advertise it as a native Windows capability.
        if !cfg!(target_os = "linux") { continue; }
        // The desktop one, beside it, under the same rule: never over one
        // pleamar did not write.
        let Some(desktop_dir) = dir.parent().map(|p| p.join("pleamar-desktop")) else { continue };
        let desktop_file = desktop_dir.join("SKILL.md");
        if std::fs::read_to_string(&desktop_file).is_ok_and(|t| !t.contains("<!-- pleamar skill ")) {
            if loud {
                println!("skill · {agent}: {} is someone else's: left as it is", desktop_file.display());
            }
            continue;
        }
        let desktop = DESKTOP.replace("{VERSION}", &stamp());
        match std::fs::create_dir_all(&desktop_dir).and_then(|_| std::fs::write(&desktop_file, desktop)) {
            Ok(()) if loud || before.is_some() => println!("skill · {agent}: {}", desktop_file.display()),
            Ok(()) => {}
            Err(e) => eprintln!("skill · {agent}: {}: {e}", desktop_file.display()),
        }
    }
    0
}

/// On every start, off the way: a skill of another version is written again.
/// Only where pleamar already put one —installing is asked for once—.
pub fn refresh_quietly() {
    std::thread::spawn(|| {
        for (_, dir) in places() {
            let file = dir.join("SKILL.md");
            if let Ok(text) = std::fs::read_to_string(&file) {
                if text.contains("<!-- pleamar skill ") && !text.contains(&mark()) {
                    install(false);
                    return;
                }
            }
        }
    });
}
