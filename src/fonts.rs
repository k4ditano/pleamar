//! The system's fonts, as cosmic-text's own `FontSystem::new` finds them, with the three generic
//! families named as `font_families.rs` says (pleamar#154): `FontSystem::new` puts Open Sans, DejaVu
//! Serif and Noto Sans Mono on sans-serif, serif and monospace whatever fontconfig says, installed
//! or not, and a machine without Open Sans got a sans-serif that is not there.

use cosmic_text::FontSystem;

/// A `FontSystem` with the system's fonts.
pub(crate) fn font_system() -> FontSystem {
    #[allow(unused_mut)]
    let mut fonts = FontSystem::new();
    #[cfg(target_os = "linux")]
    {
        let aliases = linux::aliases();
        crate::font_families::settle(fonts.db_mut(), &aliases);
    }
    fonts
}

#[cfg(target_os = "linux")]
mod linux {
    use crate::font_families::Aliases;
    use std::path::{Path, PathBuf};

    /// What fontconfig says for sans-serif, serif and monospace: the configuration files read as
    /// fontdb reads them (`FONTCONFIG_FILE`, else the user's, else the system's).
    pub(super) fn aliases() -> Aliases {
        let mut fontconfig = fontconfig_parser::FontConfig::default();
        if let Ok(config_file) = std::env::var("FONTCONFIG_FILE") {
            let _ = fontconfig.merge_config(Path::new(&config_file));
        } else {
            let xdg_config_home = if let Ok(val) = std::env::var("XDG_CONFIG_HOME") {
                Some(PathBuf::from(val))
            } else {
                std::env::var("HOME").ok().map(|home| Path::new(&home).join(".config"))
            };
            let read_global = match xdg_config_home {
                Some(p) => fontconfig.merge_config(&p.join("fontconfig/fonts.conf")).is_err(),
                None => true,
            };
            if read_global {
                let _ = fontconfig.merge_config(Path::new("/etc/fonts/local.conf"));
            }
            let _ = fontconfig.merge_config(Path::new("/etc/fonts/fonts.conf"));
        }
        let mut aliases = Aliases::default();
        for fontconfig_parser::Alias { alias, default, prefer, accept } in fontconfig.aliases {
            aliases.add(&alias, prefer.into_iter().chain(accept).chain(default));
        }
        aliases
    }
}
