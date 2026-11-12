//! The system's fonts, as cosmic-text's own `FontSystem::new` finds them, but
//! without reading every font file in every process (#147).
//!
//! `FontSystem::new` has fontdb open and parse each of the system's font files
//! —a thousand on a desktop with CJK and emoji, most of a gigabyte to go
//! through, seconds with a cold disk— in every process that writes a letter,
//! and the recorder did it once more. Here the same folders are walked the same
//! way, and what fontdb's own parser said of each file (its faces: families,
//! weight, style, width, whether monospaced) is kept in a small cache next to
//! the file's size and time. Another process, or the next start, takes it from
//! there and only parses what is new or has changed. Same faces, in the same
//! order, with the same names: which font a text gets, and its fallbacks (CJK,
//! emoji), do not change. The letters themselves are read from the file the
//! first time a face is used, as before.
//!
//! Once read, the list stays for the whole process: whoever else wants a
//! `FontSystem` (the recorder's keycaps) gets a copy of it.

use cosmic_text::FontSystem;

/// A `FontSystem` with the system's fonts, and how many there are.
pub(crate) fn font_system() -> FontSystem {
    #[cfg(target_os = "linux")]
    {
        let db = linux::database().clone();
        let locale = sys_locale::get_locale().unwrap_or_else(|| String::from("en-US"));
        FontSystem::new_with_locale_and_db(locale, db)
    }
    #[cfg(not(target_os = "linux"))]
    {
        FontSystem::new()
    }
}

/// What the first read found: (faces, files parsed now, files taken from the cache).
pub(crate) fn last_read() -> (usize, usize, usize) {
    #[cfg(target_os = "linux")]
    {
        *linux::READ.get().unwrap_or(&(0, 0, 0))
    }
    #[cfg(not(target_os = "linux"))]
    {
        (0, 0, 0)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use cosmic_text::fontdb::{self, Database, FaceInfo, Language, Source, Stretch, Style, Weight};
    use std::collections::{HashMap, HashSet};
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    pub(super) static READ: OnceLock<(usize, usize, usize)> = OnceLock::new();

    /// Bumped whenever what is kept changes shape (or fontdb's parser changes what it says).
    const VERSION: &str = "pleamar-fonts 1 fontdb-0.23";

    /// One face, as fontdb's parser read it.
    #[derive(Clone)]
    struct Face {
        index: u32,
        families: Vec<(String, bool)>,
        post_script_name: String,
        style: Style,
        weight: u16,
        stretch: Stretch,
        monospaced: bool,
    }

    /// A file's faces, and the size and time it had when they were read.
    #[derive(Clone)]
    struct Entry {
        size: u64,
        mtime: i128,
        faces: Vec<Face>,
    }

    pub(super) fn database() -> &'static Database {
        static DB: OnceLock<Database> = OnceLock::new();
        DB.get_or_init(read)
    }

    fn read() -> Database {
        let cache_path = cache_path();
        let old = cache_path.as_deref().map(load_cache).unwrap_or_default();
        let mut walk = Walk { db: Database::new(), old, new: HashMap::new(), parsed: 0, cached: 0 };
        walk.system_fonts();
        // As `FontSystem::new` does, after reading them (it overrides what fontconfig said for these three).
        walk.db.set_monospace_family("Noto Sans Mono");
        walk.db.set_sans_serif_family("Open Sans");
        walk.db.set_serif_family("DejaVu Serif");
        let changed = walk.parsed > 0 || walk.new.len() != walk.old.len();
        if changed && let Some(p) = &cache_path {
            save_cache(p, &walk.new);
        }
        let _ = READ.set((walk.db.len(), walk.parsed, walk.cached));
        walk.db
    }

    struct Walk {
        db: Database,
        old: HashMap<PathBuf, Entry>,
        new: HashMap<PathBuf, Entry>,
        parsed: usize,
        cached: usize,
    }

    impl Walk {
        /// fontdb's `load_system_fonts` on Linux with fontconfig: its aliases, then its folders.
        fn system_fonts(&mut self) {
            if !self.fontconfig() {
                self.no_fontconfig();
            }
        }

        fn no_fontconfig(&mut self) {
            let mut seen = HashSet::new();
            self.dir(Path::new("/usr/share/fonts/"), &mut seen);
            self.dir(Path::new("/usr/local/share/fonts/"), &mut seen);
            if let Ok(home) = std::env::var("HOME") {
                let home = Path::new(&home);
                self.dir(&home.join(".fonts"), &mut seen);
                self.dir(&home.join(".local/share/fonts"), &mut seen);
            }
        }

        fn fontconfig(&mut self) -> bool {
            let mut fontconfig = fontconfig_parser::FontConfig::default();
            let home = std::env::var("HOME");
            if let Ok(config_file) = std::env::var("FONTCONFIG_FILE") {
                let _ = fontconfig.merge_config(Path::new(&config_file));
            } else {
                let xdg_config_home = if let Ok(val) = std::env::var("XDG_CONFIG_HOME") {
                    Some(PathBuf::from(val))
                } else if let Ok(home) = &home {
                    Some(Path::new(home).join(".config"))
                } else {
                    None
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
            for fontconfig_parser::Alias { alias, default, prefer, accept } in fontconfig.aliases {
                let name = prefer.first().or_else(|| accept.first()).or_else(|| default.first());
                if let Some(name) = name {
                    match alias.to_lowercase().as_str() {
                        "serif" => self.db.set_serif_family(name),
                        "sans-serif" | "sans serif" => self.db.set_sans_serif_family(name),
                        "monospace" => self.db.set_monospace_family(name),
                        "cursive" => self.db.set_cursive_family(name),
                        "fantasy" => self.db.set_fantasy_family(name),
                        _ => {}
                    }
                }
            }
            if fontconfig.dirs.is_empty() {
                return false;
            }
            let mut seen = HashSet::new();
            for dir in fontconfig.dirs {
                let path = if dir.path.starts_with("~") {
                    match &home {
                        Ok(home) => Path::new(home).join(dir.path.strip_prefix("~").unwrap()),
                        Err(_) => continue,
                    }
                } else {
                    dir.path
                };
                self.dir(&path, &mut seen);
            }
            true
        }

        /// fontdb's `canonicalize`, to the letter: which entries it skips, and
        /// the paths it gives the files, decide which faces there are and in what order.
        fn canonicalize(&self, path: PathBuf, entry: std::fs::DirEntry, seen: &mut HashSet<PathBuf>) -> Option<(PathBuf, std::fs::FileType)> {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_symlink() {
                if !seen.is_empty() {
                    if seen.contains(&path) {
                        return None;
                    }
                    seen.insert(path.clone());
                }
                return Some((path, file_type));
            }
            if seen.is_empty() && file_type.is_dir() {
                for info in self.db.faces() {
                    if let Source::File(p) = &info.source {
                        seen.insert(p.clone());
                    }
                }
            }
            let stat = std::fs::metadata(&path).ok()?;
            if stat.is_symlink() {
                return None;
            }
            let canon = std::fs::canonicalize(path).ok()?;
            if seen.contains(&canon) {
                return None;
            }
            seen.insert(canon.clone());
            Some((canon, stat.file_type()))
        }

        /// fontdb's `load_fonts_dir_impl`.
        fn dir(&mut self, dir: &Path, seen: &mut HashSet<PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for entry in entries.flatten() {
                let Some((path, file_type)) = self.canonicalize(entry.path(), entry, seen) else { continue };
                if file_type.is_file() {
                    if matches!(path.extension().and_then(|e| e.to_str()), Some("ttf" | "ttc" | "TTF" | "TTC" | "otf" | "otc" | "OTF" | "OTC")) {
                        self.file(path);
                    }
                } else if file_type.is_dir() {
                    self.dir(&path, seen);
                }
            }
        }

        /// A font file's faces: from the cache if it has not changed, parsed by fontdb if it has.
        fn file(&mut self, path: PathBuf) {
            let Ok(meta) = std::fs::metadata(&path) else { return };
            let (size, mtime) = (meta.len(), mtime(&meta));
            let entry = match self.old.get(&path).filter(|e| e.size == size && e.mtime == mtime) {
                Some(e) => {
                    self.cached += 1;
                    e.clone()
                }
                None => {
                    self.parsed += 1;
                    let mut one = Database::new();
                    if let Err(e) = one.load_font_file(&path) {
                        log_warn(&format!("Failed to load '{}' cause {}.", path.display(), e));
                        return;
                    }
                    let faces = one.faces().map(|f| Face { index: f.index, families: f.families.iter().map(|(n, l)| (n.clone(), *l == Language::English_UnitedStates)).collect(), post_script_name: f.post_script_name.clone(), style: f.style, weight: f.weight.0, stretch: f.stretch, monospaced: f.monospaced }).collect();
                    Entry { size, mtime, faces }
                }
            };
            for f in &entry.faces {
                self.db.push_face_info(FaceInfo {
                    id: fontdb::ID::dummy(),
                    source: Source::File(path.clone()),
                    index: f.index,
                    // Only whether a name is the US English one is kept: fontdb puts that one first, and
                    // nothing that picks a font reads the others' languages, only their names.
                    families: f.families.iter().map(|(n, en)| (n.clone(), if *en { Language::English_UnitedStates } else { Language::Unknown })).collect(),
                    post_script_name: f.post_script_name.clone(),
                    style: f.style,
                    weight: Weight(f.weight),
                    stretch: f.stretch,
                    monospaced: f.monospaced,
                });
            }
            // Only paths that are text can be written down; any other is parsed each time.
            if path.to_str().is_some() {
                self.new.insert(path, entry);
            }
        }
    }

    fn log_warn(s: &str) {
        if std::env::var_os("PLEAMAR_DEBUG_TEXT").is_some() {
            eprintln!("text   · {s}");
        }
    }

    fn mtime(meta: &std::fs::Metadata) -> i128 {
        use std::os::unix::fs::MetadataExt;
        meta.mtime() as i128 * 1_000_000_000 + meta.mtime_nsec() as i128
    }

    /// `$XDG_CACHE_HOME/pleamar/fonts`, or `~/.cache/pleamar/fonts`.
    fn cache_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".cache")))?;
        Some(base.join("pleamar").join("fonts"))
    }

    // The cache is text, one line per file and one per face under it, fields
    // separated by tabs; in names, `\`, tab and line breaks are escaped.

    fn escape(s: &str) -> String {
        s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n").replace('\r', "\\r")
    }

    fn unescape(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut it = s.chars();
        while let Some(c) = it.next() {
            if c == '\\' {
                match it.next() {
                    Some('t') => out.push('\t'),
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    Some(o) => out.push(o),
                    None => {}
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    fn style_code(s: Style) -> u8 {
        match s {
            Style::Normal => 0,
            Style::Italic => 1,
            Style::Oblique => 2,
        }
    }

    fn style_from(n: u8) -> Option<Style> {
        Some(match n {
            0 => Style::Normal,
            1 => Style::Italic,
            2 => Style::Oblique,
            _ => return None,
        })
    }

    const STRETCHES: [Stretch; 9] = [Stretch::UltraCondensed, Stretch::ExtraCondensed, Stretch::Condensed, Stretch::SemiCondensed, Stretch::Normal, Stretch::SemiExpanded, Stretch::Expanded, Stretch::ExtraExpanded, Stretch::UltraExpanded];

    fn save_cache(path: &Path, entries: &HashMap<PathBuf, Entry>) {
        let mut out = String::with_capacity(entries.len() * 128);
        out.push_str(VERSION);
        out.push('\n');
        let mut paths: Vec<&PathBuf> = entries.keys().collect();
        paths.sort();
        for p in paths {
            let (Some(name), e) = (p.to_str(), &entries[p]) else { continue };
            out.push_str(&format!("F\t{}\t{}\t{}\n", escape(name), e.size, e.mtime));
            for f in &e.faces {
                out.push_str(&format!("f\t{}\t{}\t{}\t{}\t{}\t{}", f.index, style_code(f.style), f.weight, f.stretch.to_number(), f.monospaced as u8, escape(&f.post_script_name)));
                for (n, en) in &f.families {
                    out.push_str(&format!("\t{}{}", if *en { 'E' } else { '-' }, escape(n)));
                }
                out.push('\n');
            }
        }
        // Written aside and moved into place: another process starting at the same
        // time reads the old one or the new one whole, never half of one.
        let Some(dir) = path.parent() else { return };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        let tmp = dir.join(format!(".fonts.{}", std::process::id()));
        if std::fs::write(&tmp, out).is_ok() && std::fs::rename(&tmp, path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    fn load_cache(path: &Path) -> HashMap<PathBuf, Entry> {
        let Ok(text) = std::fs::read_to_string(path) else { return HashMap::new() };
        parse_cache(&text).unwrap_or_default()
    }

    /// Anything it does not understand throws the whole cache away: the fonts are read again.
    fn parse_cache(text: &str) -> Option<HashMap<PathBuf, Entry>> {
        let mut lines = text.lines();
        if lines.next()? != VERSION {
            return None;
        }
        let mut all: HashMap<PathBuf, Entry> = HashMap::new();
        let mut current: Option<PathBuf> = None;
        for line in lines {
            let mut f = line.split('\t');
            match f.next()? {
                "F" => {
                    let p = PathBuf::from(unescape(f.next()?));
                    let size = f.next()?.parse().ok()?;
                    let mtime = f.next()?.parse().ok()?;
                    all.insert(p.clone(), Entry { size, mtime, faces: Vec::new() });
                    current = Some(p);
                }
                "f" => {
                    let index = f.next()?.parse().ok()?;
                    let style = style_from(f.next()?.parse().ok()?)?;
                    let weight = f.next()?.parse().ok()?;
                    let stretch = *STRETCHES.get(f.next()?.parse::<usize>().ok()?.checked_sub(1)?)?;
                    let monospaced = f.next()? == "1";
                    let post_script_name = unescape(f.next()?);
                    let families: Vec<(String, bool)> = f.map(|n| (unescape(&n[1..]), n.starts_with('E'))).collect();
                    all.get_mut(current.as_ref()?)?.faces.push(Face { index, families, post_script_name, style, weight, stretch, monospaced });
                }
                _ => return None,
            }
        }
        Some(all)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_font_cache_reads_back_what_it_wrote() {
            let dir = std::env::temp_dir().join(format!("pleamar-fonts-test-{}", std::process::id()));
            let path = dir.join("fonts");
            let face = Face { index: 2, families: vec![("Noto\tSans \\ CJK".into(), true), ("思源黑體".into(), false)], post_script_name: "NotoSansCJK-Regular".into(), style: Style::Oblique, weight: 350, stretch: Stretch::SemiCondensed, monospaced: true };
            let mut entries = HashMap::new();
            entries.insert(PathBuf::from("/usr/share/fonts/a b.ttc"), Entry { size: 123, mtime: -5, faces: vec![face.clone()] });
            entries.insert(PathBuf::from("/usr/share/fonts/broken.ttf"), Entry { size: 1, mtime: 7, faces: vec![] });
            save_cache(&path, &entries);
            let back = load_cache(&path);
            let _ = std::fs::remove_dir_all(&dir);
            assert_eq!(back.len(), 2);
            let e = &back[Path::new("/usr/share/fonts/a b.ttc")];
            assert_eq!((e.size, e.mtime), (123, -5));
            let f = &e.faces[0];
            assert_eq!((f.index, f.weight, f.stretch, f.monospaced, f.style), (2, 350, Stretch::SemiCondensed, true, Style::Oblique));
            assert_eq!(f.families, face.families);
            assert_eq!(f.post_script_name, face.post_script_name);
            assert!(back[Path::new("/usr/share/fonts/broken.ttf")].faces.is_empty());
            assert!(parse_cache("something else\n").is_none());
        }

        /// The same faces, in the same order, with the same names, as fontdb's own reading of the system.
        #[test]
        fn the_cached_fonts_are_the_ones_fontdb_reads() {
            let mut theirs = Database::new();
            theirs.load_system_fonts();
            let mut ours = Walk { db: Database::new(), old: HashMap::new(), new: HashMap::new(), parsed: 0, cached: 0 };
            ours.system_fonts();
            let first: Vec<_> = ours.db.faces().map(|f| (f.source.clone(), f.index)).collect();
            // And once more from what the first walk would have kept.
            let old = std::mem::take(&mut ours.new);
            let mut again = Walk { db: Database::new(), old, new: HashMap::new(), parsed: 0, cached: 0 };
            again.system_fonts();
            assert_eq!(again.parsed, 0, "nothing changed: nothing parsed");
            for db in [&ours.db, &again.db] {
                let a: Vec<_> = theirs.faces().collect();
                let b: Vec<_> = db.faces().collect();
                assert_eq!(a.len(), b.len());
                for (x, y) in a.iter().zip(&b) {
                    let path = |s: &Source| match s {
                        Source::File(p) => p.clone(),
                        _ => PathBuf::new(),
                    };
                    assert_eq!((path(&x.source), x.index, &x.post_script_name, x.style, x.weight, x.stretch, x.monospaced), (path(&y.source), y.index, &y.post_script_name, y.style, y.weight, y.stretch, y.monospaced));
                    let names = |f: &FaceInfo| f.families.iter().map(|(n, l)| (n.clone(), *l == Language::English_UnitedStates)).collect::<Vec<_>>();
                    assert_eq!(names(x), names(y));
                }
                assert_eq!(theirs.family_name(&fontdb::Family::Cursive), db.family_name(&fontdb::Family::Cursive));
                assert_eq!(theirs.family_name(&fontdb::Family::Fantasy), db.family_name(&fontdb::Family::Fantasy));
            }
            assert_eq!(first.len(), theirs.len());
        }
    }
}
