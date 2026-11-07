//! `markdown: true` on a text: what a model writes —`**bold**`, `*italic*`,
//! `` `code` ``, `# a title`, `- a point`, a fenced block— shown as it means,
//! not with its marks. The marks go; what they marked is kept as stretches
//! of the plain text, which the workshop lays out bold, italic or monospace.
//! Everything else reads the plain text: its letters, what is selected and
//! copied, what an agent is told.

pub const BOLD: u8 = 1;
pub const ITALIC: u8 = 2;
pub const CODE: u8 = 4;
/// What a stretch is, for its colour (`link:`, `quote:`, `heading:` on the
/// text; code is `CODE`): they do not change how it is laid out.
pub const LINK: u8 = 8;
pub const QUOTE: u8 = 16;
pub const HEADING: u8 = 32;
/// The marks that change the layout; the others only colour it.
pub const LOOKS: u8 = BOLD | ITALIC | CODE;

/// The text without its marks, and the stretches of it (byte ranges) that
/// are bold, italic or code. A mark still open at the end —a model halfway
/// through writing `**this`— holds to the end of its line, without showing.
pub fn plain(source: &str) -> (String, Vec<(u32, u32, u8)>) {
    let mut out = String::with_capacity(source.len());
    let mut looks: Vec<u8> = Vec::with_capacity(source.len());
    let mut fenced = false;
    let mut first = true;
    for line in source.split('\n') {
        let bare = line.trim_start();
        if bare.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if !first {
            push(&mut out, &mut looks, "\n", 0);
        }
        first = false;
        if fenced {
            push(&mut out, &mut looks, line, CODE);
            continue;
        }
        let indent = &line[..line.len() - bare.len()];
        let hashes = bare.bytes().take_while(|b| *b == b'#').count();
        if (1..=6).contains(&hashes) && bare[hashes..].starts_with(' ') {
            inline(bare[hashes..].trim_start(), BOLD | HEADING, &mut out, &mut looks);
        } else if let Some(rest) = ["- ", "* ", "+ "].iter().find_map(|m| bare.strip_prefix(m)) {
            push(&mut out, &mut looks, indent, 0);
            push(&mut out, &mut looks, "• ", 0);
            inline(rest, 0, &mut out, &mut looks);
        } else if let Some(rest) = bare.strip_prefix("> ") {
            push(&mut out, &mut looks, indent, 0);
            inline(rest, ITALIC | QUOTE, &mut out, &mut looks);
        } else {
            inline(line, 0, &mut out, &mut looks);
        }
    }
    (out, stretches(&looks))
}

fn push(out: &mut String, looks: &mut Vec<u8>, s: &str, look: u8) {
    out.push_str(s);
    looks.resize(out.len(), look);
}

/// One line's marks: `**`/`__` bold, `*`/`_` italic, `` ` `` code, `[text](link)`
/// its text, and `\*` a star as it is.
fn inline(line: &str, base: u8, out: &mut String, looks: &mut Vec<u8>) {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let at = |k: usize| chars.get(k).map(|c| c.1);
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    let (mut bold, mut italic) = (false, false);
    let mut k = 0;
    while k < chars.len() {
        let (i, c) = chars[k];
        let look = base | if bold { BOLD } else { 0 } | if italic { ITALIC } else { 0 };
        let mut buf = [0u8; 4];
        match c {
            '\\' if at(k + 1).is_some_and(|n| "*_`[]\\#".contains(n)) => {
                push(out, looks, at(k + 1).unwrap().encode_utf8(&mut buf), look);
                k += 2;
            }
            '`' => match line[i + 1..].find('`') {
                Some(end) if end > 0 => {
                    push(out, looks, &line[i + 1..i + 1 + end], look | CODE);
                    k = chars.partition_point(|(j, _)| *j <= i + 1 + end);
                }
                _ => {
                    push(out, looks, "`", look);
                    k += 1;
                }
            },
            '*' | '_' if at(k + 1) == Some(c) => {
                // `__` inside a word (snake__case) is left as it is.
                let opens = !bold && !at(k + 2).is_some_and(char::is_whitespace) && at(k + 2).is_some() && (c == '*' || !word(k.checked_sub(1).and_then(at)));
                let closes = bold && !k.checked_sub(1).and_then(at).is_some_and(char::is_whitespace) && (c == '*' || !word(at(k + 2)));
                if opens || closes {
                    bold = !bold;
                } else if at(k + 2).is_none() && !bold {
                    // The end of what has arrived: the mark is being written.
                } else {
                    push(out, looks, if c == '*' { "**" } else { "__" }, look);
                }
                k += 2;
            }
            '*' | '_' => {
                let before = k.checked_sub(1).and_then(at);
                let after = at(k + 1);
                // A star between spaces is a product (2 * 3); an underscore in a word, a name.
                let opens = !italic && after.is_some_and(|a| !a.is_whitespace()) && (c == '*' || !word(before)) && line[i + 1..].contains(c);
                let closes = italic && before.is_some_and(|b| !b.is_whitespace()) && (c == '*' || !word(after));
                if opens || closes {
                    italic = !italic;
                } else {
                    push(out, looks, c.encode_utf8(&mut buf), look);
                }
                k += 1;
            }
            '[' => {
                // `[what is read](where it goes)`: what is read.
                let link = line[i..].find("](").and_then(|m| line[i + m..].find(')').map(|e| (m, e)));
                match link {
                    Some((m, e)) if m > 1 => {
                        inline(&line[i + 1..i + m], look | LINK, out, looks);
                        k = chars.partition_point(|(j, _)| *j <= i + m + e);
                    }
                    _ => {
                        push(out, looks, "[", look);
                        k += 1;
                    }
                }
            }
            // `@launch-brief.md`, `@Research`: a name, as a link is (`link:`).
            '@' if !k.checked_sub(1).and_then(at).is_some_and(|b| !b.is_whitespace() && b != '(') && at(k + 1).is_some_and(char::is_alphanumeric) => {
                let mut e = k + 1;
                while at(e).is_some_and(|n| n.is_alphanumeric() || "._-/".contains(n)) {
                    e += 1;
                }
                // A full stop after it ends the sentence, not the name.
                while e > k + 1 && at(e - 1).is_some_and(|n| ".-/".contains(n)) {
                    e -= 1;
                }
                let end = chars.get(e).map_or(line.len(), |c| c.0);
                push(out, looks, &line[i..end], look | LINK);
                k = e;
            }
            _ => {
                push(out, looks, c.encode_utf8(&mut buf), look);
                k += 1;
            }
        }
    }
}

/// The stretches of a text of `len` bytes with `extra` added to `base`:
/// where they meet, a byte takes the marks of both.
pub fn merged(len: usize, base: &[(u32, u32, u8)], extra: &[(u32, u32, u8)]) -> Vec<(u32, u32, u8)> {
    if extra.is_empty() {
        return base.to_vec();
    }
    let mut looks = vec![0u8; len];
    for &(a, b, l) in base.iter().chain(extra) {
        for x in &mut looks[(a as usize).min(len)..(b as usize).min(len)] {
            *x |= l;
        }
    }
    stretches(&looks)
}

fn stretches(looks: &[u8]) -> Vec<(u32, u32, u8)> {
    let mut out: Vec<(u32, u32, u8)> = Vec::new();
    for (i, l) in looks.iter().enumerate() {
        match out.last_mut() {
            Some(last) if last.2 == *l && last.1 == i as u32 => last.1 += 1,
            _ if *l != 0 => out.push((i as u32, i as u32 + 1, *l)),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marked(s: &str) -> (String, Vec<(String, u8)>) {
        let (text, marks) = plain(s);
        let parts = marks.iter().map(|(a, b, l)| (text[*a as usize..*b as usize].to_owned(), *l)).collect();
        (text, parts)
    }

    #[test]
    fn a_models_answer_loses_its_marks_and_keeps_what_they_meant() {
        let (text, parts) = marked("Ideas:\n\n- **Micrófono USB**: lo más *fácil*.\n- Usa `arecord -l`.");
        assert_eq!(text, "Ideas:\n\n• Micrófono USB: lo más fácil.\n• Usa arecord -l.");
        assert_eq!(parts, vec![("Micrófono USB".into(), BOLD), ("fácil".into(), ITALIC), ("arecord -l".into(), CODE)]);
    }

    #[test]
    fn titles_quotes_links_and_fenced_code() {
        let (text, parts) = marked("## Pasos\n> nota\n[la web](https://x.y) y ya\n```sh\nls -l\n```\nfin");
        assert_eq!(text, "Pasos\nnota\nla web y ya\nls -l\nfin");
        assert_eq!(parts, vec![("Pasos".into(), BOLD | HEADING), ("nota".into(), ITALIC | QUOTE), ("la web".into(), LINK), ("ls -l".into(), CODE)]);
    }

    #[test]
    fn names_with_an_at_are_links() {
        let (text, parts) = marked("Fix @launch-brief.md and @Research. Mail a@b.c (@x)");
        assert_eq!(text, "Fix @launch-brief.md and @Research. Mail a@b.c (@x)");
        assert_eq!(parts, vec![("@launch-brief.md".into(), LINK), ("@Research".into(), LINK), ("@x".into(), LINK)]);
    }

    #[test]
    fn marks_added_to_markdowns_meet() {
        assert_eq!(merged(10, &[(0, 4, BOLD)], &[(2, 6, ITALIC)]), vec![(0, 2, BOLD), (2, 4, BOLD | ITALIC), (4, 6, ITALIC)]);
    }

    #[test]
    fn what_only_looks_like_a_mark_is_left_alone() {
        assert_eq!(plain("2 * 3 = 6, snake_case_name, a__b").0, "2 * 3 = 6, snake_case_name, a__b");
        assert_eq!(plain("\\*literal\\*").0, "*literal*");
        assert_eq!(plain("a ` b").0, "a ` b");
    }

    #[test]
    fn a_mark_still_being_written_does_not_show() {
        // Letter by letter, as her answers arrive.
        let (text, parts) = marked("Para empezar: **micrófono US");
        assert_eq!(text, "Para empezar: micrófono US");
        assert_eq!(parts, vec![("micrófono US".into(), BOLD)]);
        assert_eq!(plain("Lo **").0, "Lo ");
    }
}
