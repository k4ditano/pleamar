//! Which family stands for «sans-serif», «serif» and «monospace» (what a text with no
//! `family:` is set in, and what `family: "sans-serif"` says).
//!
//! cosmic-text's `FontSystem::new` names three of them whatever fontconfig says
//! —Open Sans, DejaVu Serif and Noto Sans Mono—, and so a machine without Open Sans
//! has a sans-serif that is not there: nothing is found for it, and a text asks the
//! fallbacks, which at a weight no face of theirs has (a 600 where there are 400 and
//! 700) end in whichever font comes first, and the emoji font does: its «0–9», `#` and `*`
//! (the keycap emoji) draw the digits of a clock (pleamar#154).
//!
//! Here each of the three is the first of these that is installed, and only if none is, the
//! name it always had:
//!   1. pleamar's own (Open Sans, DejaVu Serif, Noto Sans Mono): what every scene was drawn with,
//!      so a machine that has it looks as it did;
//!   2. what fontconfig says for it, in its order (`prefer`, then `accept`, then `default`);
//!   3. a few families that are everywhere, most common first;
//!   4. any family that is not an emoji or symbol font (for monospace, a monospaced one), the first by name:
//!      a generic name must stand for something that is there, or what it asks for is found by the last
//!      resort of the fallbacks, which tries the emoji font first.
//!
//! (To make fontconfig's win over pleamar's own, swap 1 and 2 in `pick`.)

use cosmic_text::fontdb::Database;

/// What fontconfig says for each of the three, in the order it says it.
#[derive(Default, Debug, Clone, PartialEq)]
pub(crate) struct Aliases {
    pub sans: Vec<String>,
    pub serif: Vec<String>,
    pub mono: Vec<String>,
}

impl Aliases {
    /// One `<alias>`'s families for that generic one (`sans-serif`, `serif`, `monospace`; others are not kept).
    pub(crate) fn add(&mut self, generic: &str, names: impl IntoIterator<Item = String>) {
        let list = match generic.to_lowercase().as_str() {
            "sans-serif" | "sans serif" => &mut self.sans,
            "serif" => &mut self.serif,
            "monospace" => &mut self.mono,
            _ => return,
        };
        for name in names {
            if !list.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                list.push(name);
            }
        }
    }
}

const SANS: &str = "Open Sans";
const SERIF: &str = "DejaVu Serif";
const MONO: &str = "Noto Sans Mono";

const SANS_EVERYWHERE: &[&str] = &["Noto Sans", "DejaVu Sans", "Liberation Sans", "Roboto", "Cantarell", "Inter", "FreeSans"];
const SERIF_EVERYWHERE: &[&str] = &["Noto Serif", "Liberation Serif", "FreeSerif"];
const MONO_EVERYWHERE: &[&str] = &["DejaVu Sans Mono", "Liberation Mono", "FreeMono"];

/// Names the three families in the database, as said above.
pub(crate) fn settle(db: &mut Database, aliases: &Aliases) {
    let sans = pick(db, SANS, &aliases.sans, SANS_EVERYWHERE, false);
    // (A serif that is named nowhere is any family that is there, a monospace any monospaced one.)
    let serif = pick(db, SERIF, &aliases.serif, SERIF_EVERYWHERE, false);
    let mono = pick(db, MONO, &aliases.mono, MONO_EVERYWHERE, true);
    db.set_sans_serif_family(sans);
    db.set_serif_family(serif);
    db.set_monospace_family(mono);
}

/// The family that stands for one of the three: see the top of this file.
fn pick(db: &Database, ours: &str, said: &[String], everywhere: &[&str], mono: bool) -> String {
    let named = std::iter::once(ours).chain(said.iter().map(String::as_str)).chain(everywhere.iter().copied()).find(|name| installed(db, name));
    match named {
        Some(name) => name.to_owned(),
        None => anything(db, mono).unwrap_or_else(|| ours.to_owned()),
    }
}

/// The first family by name that is not an emoji or symbol font, nor (unless `mono`) a monospaced one.
fn anything(db: &Database, mono: bool) -> Option<String> {
    let text = |f: &&cosmic_text::fontdb::FaceInfo| {
        let name = f.families.first().map_or("", |n| n.0.as_str());
        !f.post_script_name.contains("Emoji") && !name.contains("Emoji") && !name.contains("Symbol") && f.monospaced == mono
    };
    db.faces().filter(text).filter_map(|f| f.families.first().map(|n| n.0.clone())).min()
}

fn installed(db: &Database, name: &str) -> bool {
    db.faces().any(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic_text::fontdb::{FaceInfo, Family, Language, Source, Stretch, Style, Weight};

    /// The family a generic name stands for in the database: what `settle` chose.
    fn named(db: &Database, generic: Family<'_>) -> String {
        db.family_name(&generic).to_owned()
    }
    use std::sync::Arc;

    /// A database with these families installed, as names only (nothing is read from them).
    fn with(families: &[&str]) -> Database {
        let mut db = Database::new();
        for name in families {
            db.push_face_info(FaceInfo {
                id: cosmic_text::fontdb::ID::dummy(),
                source: Source::Binary(Arc::new(Vec::<u8>::new())),
                index: 0,
                families: vec![((*name).to_owned(), Language::English_UnitedStates)],
                post_script_name: name.replace(' ', ""),
                style: Style::Normal,
                weight: Weight(400),
                stretch: Stretch::Normal,
                monospaced: name.contains("Mono"),
            });
        }
        db
    }

    fn said(sans: &[&str]) -> Aliases {
        let mut a = Aliases::default();
        a.add("sans-serif", sans.iter().map(|s| (*s).to_owned()));
        a
    }

    #[test]
    fn without_open_sans_the_first_family_fontconfig_names_that_is_installed_is_the_sans_serif() {
        // A machine with Noto Sans and an emoji font, whose fontconfig says Bitstream Vera first, then Noto Sans.
        let mut db = with(&["Noto Sans", "Noto Color Emoji"]);
        settle(&mut db, &said(&["Bitstream Vera Sans", "Noto Sans", "DejaVu Sans"]));
        assert_eq!(named(&db, Family::SansSerif), "Noto Sans");
        // Not the emoji font, and not a name that is not there.
        let mut db = with(&["Noto Color Emoji", "DejaVu Sans"]);
        settle(&mut db, &said(&["Bitstream Vera Sans", "DejaVu Sans"]));
        assert_eq!(named(&db, Family::SansSerif), "DejaVu Sans");
    }

    #[test]
    fn with_no_word_from_fontconfig_a_family_that_is_everywhere_stands_in() {
        let mut db = with(&["Noto Color Emoji", "Noto Sans"]);
        settle(&mut db, &Aliases::default());
        assert_eq!(named(&db, Family::SansSerif), "Noto Sans");
        let mut db = with(&["Noto Color Emoji", "Liberation Sans", "DejaVu Sans"]);
        settle(&mut db, &Aliases::default());
        assert_eq!(named(&db, Family::SansSerif), "DejaVu Sans", "the most common first");
    }

    #[test]
    fn where_open_sans_is_installed_it_is_still_the_sans_serif_and_nothing_else_moves() {
        let mut db = with(&["Open Sans", "DejaVu Sans", "Noto Sans", "DejaVu Serif", "Noto Sans Mono"]);
        settle(&mut db, &said(&["DejaVu Sans"]));
        assert_eq!(named(&db, Family::SansSerif), "Open Sans");
        assert_eq!(named(&db, Family::Serif), "DejaVu Serif");
        assert_eq!(named(&db, Family::Monospace), "Noto Sans Mono");
    }

    #[test]
    fn serif_and_monospace_follow_the_same_rule() {
        let mut aliases = Aliases::default();
        aliases.add("serif", ["Source Serif".to_owned(), "Noto Serif".to_owned()]);
        aliases.add("monospace", ["JetBrains Mono".to_owned()]);
        let mut db = with(&["Noto Serif", "JetBrains Mono", "DejaVu Sans Mono"]);
        settle(&mut db, &aliases);
        assert_eq!(named(&db, Family::Serif), "Noto Serif");
        assert_eq!(named(&db, Family::Monospace), "JetBrains Mono");
        // Nothing installed that is named anywhere: what always was, if nothing at all is there.
        let mut db = with(&[]);
        settle(&mut db, &aliases);
        assert_eq!((named(&db, Family::SansSerif).as_str(), named(&db, Family::Serif).as_str(), named(&db, Family::Monospace).as_str()), (SANS, SERIF, MONO));
    }

    #[test]
    fn with_nothing_named_anywhere_it_is_some_family_that_is_there_and_never_the_emoji_font() {
        let mut db = with(&["Noto Color Emoji", "Zapfino Like", "Another Face", "Noto Sans Symbols", "Odd Mono"]);
        settle(&mut db, &Aliases::default());
        assert_eq!(named(&db, Family::SansSerif), "Another Face");
        assert_eq!(named(&db, Family::Serif), "Another Face");
        assert_eq!(named(&db, Family::Monospace), "Odd Mono");
        // Only an emoji font: there is nothing better than the name it always had.
        let mut db = with(&["Noto Color Emoji"]);
        settle(&mut db, &Aliases::default());
        assert_eq!(named(&db, Family::SansSerif), SANS);
    }

    #[test]
    fn fontconfigs_aliases_for_other_families_are_not_kept_and_names_are_not_repeated() {
        let mut a = Aliases::default();
        a.add("cursive", ["Comic".to_owned()]);
        a.add("Sans-Serif", ["A".to_owned(), "a".to_owned(), "B".to_owned()]);
        assert_eq!(a, Aliases { sans: vec!["A".into(), "B".into()], serif: vec![], mono: vec![] });
    }

    /// A font file by name, in `PLEAMAR_TEST_FONTS` or under the usual folders.
    fn find(name: &str) -> Option<std::path::PathBuf> {
        fn walk(dir: &std::path::Path, name: &str, depth: u32) -> Option<std::path::PathBuf> {
            for e in std::fs::read_dir(dir).ok()?.flatten() {
                let p = e.path();
                if p.is_dir() && depth > 0 {
                    if let Some(f) = walk(&p, name, depth - 1) {
                        return Some(f);
                    }
                } else if p.file_name().is_some_and(|n| n == name) {
                    return Some(p);
                }
            }
            None
        }
        let mut roots: Vec<std::path::PathBuf> = std::env::var_os("PLEAMAR_TEST_FONTS").map(std::path::PathBuf::from).into_iter().collect();
        roots.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(std::path::PathBuf::from));
        roots.into_iter().find_map(|r| walk(&r, name, 5))
    }

    /// Which faces draw this text, at a weight, in the default family: their postscript names.
    fn drawn_by(fonts: &mut cosmic_text::FontSystem, text: &str, weight: u16) -> Vec<String> {
        use cosmic_text::{Attrs, Buffer, Metrics, Shaping, Weight};
        let mut buffer = Buffer::new(fonts, Metrics::new(28.0, 34.0));
        buffer.set_text(text, &Attrs::new().weight(Weight(weight)), Shaping::Advanced, None);
        buffer.shape_until_scroll(fonts, false);
        let mut faces = Vec::new();
        for run in buffer.layout_runs() {
            for g in run.glyphs {
                let name = fonts.db().face(g.font_id).map_or(String::new(), |f| f.post_script_name.clone());
                if !faces.contains(&name) {
                    faces.push(name);
                }
            }
        }
        faces
    }

    /// pleamar#154, without a card: shaped as the workshop does, with Noto Sans Regular and an emoji font
    /// and no Open Sans, the digits at weight 600 (a weight neither has) come from Noto Sans.
    #[test]
    fn digits_at_a_weight_no_face_has_are_not_drawn_by_the_emoji_font() {
        let (Some(sans), Some(emoji)) = (find("NotoSans-Regular.ttf"), find("NotoColorEmoji.ttf")) else {
            eprintln!("no Noto Sans Regular and Noto Color Emoji to try with: not tried");
            return;
        };
        let mut db = Database::new();
        db.load_font_file(&sans).unwrap();
        db.load_font_file(&emoji).unwrap();
        settle(&mut db, &Aliases::default());
        let mut fonts = cosmic_text::FontSystem::new_with_locale_and_db("en-US".to_owned(), db);
        for weight in [400, 600, 700] {
            let faces = drawn_by(&mut fonts, "16:36 0123456789 #*", weight);
            assert!(!faces.is_empty() && faces.iter().all(|f| !f.contains("Emoji")), "weight {weight}: drawn by {faces:?}");
        }
        // And a real emoji is still the emoji font's, at any weight.
        for weight in [400, 600] {
            let faces = drawn_by(&mut fonts, "ok \u{1F600}", weight);
            assert!(faces.iter().any(|f| f.contains("Emoji")), "weight {weight}: an emoji was drawn by {faces:?}");
        }
    }
}
