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
/// `~~struck through~~`: a line across it (`strike:` on the text, its colour).
pub const STRIKE: u8 = 64;
/// A table's cell or rule (`| a | b |`), laid out in monospace.
pub const TABLE: u8 = 128;
/// The marks that change the layout; the others only colour it.
pub const LOOKS: u8 = BOLD | ITALIC | CODE;

/// The text without its marks, and the stretches of it (byte ranges) that
/// are bold, italic or code. A mark still open at the end —a model halfway
/// through writing `**this`— holds to the end of its line, without showing.
#[cfg(test)]
pub fn plain(source: &str) -> (String, Vec<(u32, u32, u8)>) {
    let (text, marks, _) = plain_with_links(source);
    (text, marks)
}

thread_local! {
    /// The links found while reading a text: (from, to, where it goes).
    static LINKS: std::cell::RefCell<Vec<(u32, u32, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The same, and its links: the bytes of each one's words in the plain text and
/// where it goes —a `[link](where)`'s address, an `@name` itself—.
pub fn plain_with_links(source: &str) -> (String, Vec<(u32, u32, u8)>, Vec<(u32, u32, String)>) {
    LINKS.with(|l| l.borrow_mut().clear());
    let (text, marks) = blocks(source);
    (text, marks, LINKS.with(|l| std::mem::take(&mut *l.borrow_mut())))
}

fn blocks(source: &str) -> (String, Vec<(u32, u32, u8)>) {
    let mut out = String::with_capacity(source.len());
    let mut looks: Vec<u8> = Vec::with_capacity(source.len());
    let mut fenced = false;
    let mut first = true;
    let lines: Vec<&str> = source.split('\n').collect();
    let mut n = 0;
    // Each line shown starts a line, but the first.
    let start = |out: &mut String, looks: &mut Vec<u8>, first: &mut bool| {
        if !std::mem::take(first) {
            push(out, looks, "\n", 0);
        }
    };
    while n < lines.len() {
        let line = lines[n];
        let bare = line.trim_start();
        n += 1;
        if let Some(lang) = bare.strip_prefix("```") {
            fenced = !fenced;
            // ```rust: the language, over the block, as a quote is.
            let lang = lang.trim();
            if fenced && !lang.is_empty() {
                start(&mut out, &mut looks, &mut first);
                push(&mut out, &mut looks, lang, QUOTE);
            }
            continue;
        }
        if fenced {
            start(&mut out, &mut looks, &mut first);
            push(&mut out, &mut looks, line, CODE);
            continue;
        }
        // A table: a row, and under it the line of dashes that says it is one.
        if is_row(bare) && lines.get(n).is_some_and(|l| is_rule_of_table(l)) {
            let mut rows = vec![cells(bare)];
            n += 1;
            while n < lines.len() && is_row(lines[n].trim_start()) {
                rows.push(cells(lines[n].trim_start()));
                n += 1;
            }
            table(&rows, &mut out, &mut looks, &mut first);
            continue;
        }
        start(&mut out, &mut looks, &mut first);
        let indent = &line[..line.len() - bare.len()];
        // How deep a list goes: two spaces (or a tab) a level.
        let depth = indent.chars().map(|c| if c == '\t' { 2 } else { 1 }).sum::<usize>() / 2;
        let hashes = bare.bytes().take_while(|b| *b == b'#').count();
        let digits = bare.bytes().take_while(u8::is_ascii_digit).count();
        if (1..=6).contains(&hashes) && bare[hashes..].starts_with(' ') {
            inline(bare[hashes..].trim_start(), BOLD | HEADING, &mut out, &mut looks);
        } else if is_rule(bare) {
            push(&mut out, &mut looks, "────────────────────────", QUOTE);
        } else if let Some(rest) = ["- ", "* ", "+ "].iter().find_map(|m| bare.strip_prefix(m)) {
            push(&mut out, &mut looks, &"   ".repeat(depth), 0);
            // `- [ ]` and `- [x]`: a box to tick, empty or ticked.
            let (mark, rest) = match rest.get(..4) {
                Some("[ ] ") => ("☐ ", &rest[4..]),
                Some("[x] " | "[X] ") => ("☑ ", &rest[4..]),
                _ => (["• ", "◦ ", "▪ "][depth.min(2)], rest),
            };
            push(&mut out, &mut looks, mark, 0);
            inline(rest, 0, &mut out, &mut looks);
        } else if digits > 0 && (bare[digits..].starts_with(". ") || bare[digits..].starts_with(") ")) {
            push(&mut out, &mut looks, &"   ".repeat(depth), 0);
            push(&mut out, &mut looks, &bare[..digits + 2], 0);
            inline(&bare[digits + 2..], 0, &mut out, &mut looks);
        } else if let Some(rest) = bare.strip_prefix("> ") {
            push(&mut out, &mut looks, indent, 0);
            inline(rest, ITALIC | QUOTE, &mut out, &mut looks);
        } else {
            inline(line, 0, &mut out, &mut looks);
        }
    }
    (out, stretches(&looks))
}

/// `---`, `***` or `___` (spaces between them or not): a rule across.
fn is_rule(bare: &str) -> bool {
    let marks: String = bare.chars().filter(|c| !c.is_whitespace()).collect();
    marks.len() >= 3 && ["-", "*", "_"].iter().any(|m| marks.chars().all(|c| c.to_string() == *m))
}

fn is_row(bare: &str) -> bool {
    bare.starts_with('|') && bare.trim_end().len() > 1
}

/// `|---|:--:|`: what makes the row above it a table's head.
fn is_rule_of_table(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.contains('-') && t.chars().all(|c| "|-: ".contains(c))
}

fn cells(row: &str) -> Vec<&str> {
    let t = row.trim().trim_start_matches('|');
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').map(str::trim).collect()
}

/// A table, in monospace so its columns line up: each cell with its own marks,
/// padded to its column, the head in bold and a line under it.
fn table(rows: &[Vec<&str>], out: &mut String, looks: &mut Vec<u8>, first: &mut bool) {
    // Each cell on its own, with the links it has (counted from its start).
    let shown: Vec<Vec<(String, Vec<u8>, Vec<(u32, u32, String)>)>> = rows
        .iter()
        .enumerate()
        .map(|(r, row)| {
            row.iter()
                .map(|cell| {
                    let (mut o, mut l) = (String::new(), Vec::new());
                    let before = LINKS.with(|k| k.borrow().len());
                    inline(cell, CODE | TABLE | if r == 0 { BOLD } else { 0 }, &mut o, &mut l);
                    let links = LINKS.with(|k| k.borrow_mut().split_off(before));
                    (o, l, links)
                })
                .collect()
        })
        .collect();
    let columns = shown.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns).map(|k| shown.iter().filter_map(|r| r.get(k)).map(|c| c.0.chars().count()).max().unwrap_or(0)).collect();
    for (r, row) in shown.iter().enumerate() {
        if !std::mem::take(first) {
            push(out, looks, "\n", 0);
        }
        for (k, w) in widths.iter().enumerate() {
            push(out, looks, if k == 0 { "│ " } else { " │ " }, CODE | TABLE | QUOTE);
            let (text, marks, links) = row.get(k).map_or((String::new(), Vec::new(), Vec::new()), |c| (c.0.clone(), c.1.clone(), c.2.clone()));
            let from = out.len();
            LINKS.with(|l| l.borrow_mut().extend(links.into_iter().map(|(a, b, h)| (a + from as u32, b + from as u32, h))));
            out.push_str(&text);
            looks.extend(marks.iter().copied().chain(std::iter::repeat(CODE | TABLE)).take(text.len()));
            debug_assert_eq!(looks.len(), from + text.len());
            push(out, looks, &" ".repeat(w - text.chars().count()), CODE | TABLE);
        }
        push(out, looks, " │", CODE | TABLE | QUOTE);
        if r == 0 {
            push(out, looks, "\n", 0);
            let line: Vec<String> = widths.iter().map(|w| "─".repeat(w + 2)).collect();
            push(out, looks, &format!("├{}┤", line.join("┼")), CODE | TABLE | QUOTE);
        }
    }
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
    let (mut bold, mut italic, mut strike) = (false, false, false);
    let mut k = 0;
    while k < chars.len() {
        let (i, c) = chars[k];
        let look = base | if bold { BOLD } else { 0 } | if italic { ITALIC } else { 0 } | if strike { STRIKE } else { 0 };
        let mut buf = [0u8; 4];
        match c {
            '\\' if at(k + 1).is_some_and(|n| "*_`[]\\#~!@|".contains(n)) => {
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
            // `~~struck~~`; a lone `~` (about 5) stays.
            '~' if at(k + 1) == Some('~') => {
                let opens = !strike && at(k + 2).is_some_and(|n| !n.is_whitespace()) && line[i + 2..].contains("~~");
                let closes = strike && k.checked_sub(1).and_then(at).is_some_and(|b| !b.is_whitespace());
                if opens || closes {
                    strike = !strike;
                } else if !(at(k + 2).is_none() && !strike) {
                    push(out, looks, "~~", look);
                }
                k += 2;
            }
            // `![what it shows](picture)`: its words, as a link's are, but not a link.
            '!' if at(k + 1) == Some('[') && line[i..].find("](").is_some_and(|m| line[i + m..].contains(')')) => {
                let m = line[i..].find("](").unwrap();
                let e = line[i + m..].find(')').unwrap();
                inline(&line[i + 2..i + m], look, out, looks);
                k = chars.partition_point(|(j, _)| *j <= i + m + e);
            }
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
                        let from = out.len() as u32;
                        inline(&line[i + 1..i + m], look | LINK, out, looks);
                        let href = line[i + m + 2..i + m + e].trim().to_owned();
                        LINKS.with(|l| l.borrow_mut().push((from, out.len() as u32, href)));
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
                let from = out.len() as u32;
                push(out, looks, &line[i..end], look | LINK);
                LINKS.with(|l| l.borrow_mut().push((from, out.len() as u32, line[i..end].to_owned())));
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
        assert_eq!(text, "Pasos\nnota\nla web y ya\nsh\nls -l\nfin");
        assert_eq!(parts, vec![("Pasos".into(), BOLD | HEADING), ("nota".into(), ITALIC | QUOTE), ("la web".into(), LINK), ("sh".into(), QUOTE), ("ls -l".into(), CODE)]);
    }

    /// An agent's answer as the chat shows it: a table with chips in its cells,
    /// a fix struck through, nested steps, boxes to tick and a fenced block.
    const THREAD: &str = "Status of the launch:\n\n| Workstream | Owner | Status |\n|---|---|:---:|\n| Brief | @Chief | **Fix** |\n| Metrics | @Research | ~~old~~ new |\n| Copy | @Launch | done |\n\n- Plan\n  - draft\n    - review\n- [ ] ship\n- [x] test\n1. one\n---\n```shell\nnpm run build\n```";

    #[test]
    fn a_thread_message_with_a_table() {
        let (text, marks) = plain(THREAD);
        let lines: Vec<&str> = text.split('\n').collect();
        // The table: its head, the line under it and three rows, all as wide, in monospace.
        let rows: Vec<&str> = lines.iter().copied().filter(|l| l.starts_with('│') || l.starts_with('├')).collect();
        assert_eq!(rows.len(), 5, "{rows:#?}");
        assert!(rows.iter().all(|r| r.chars().count() == rows[0].chars().count()), "{rows:#?}");
        assert_eq!(rows[0].matches('│').count(), 4, "three columns: {}", rows[0]);
        assert_eq!(rows[0], "│ Workstream │ Owner     │ Status       │");
        let of = |what: &str| marks.iter().find(|m| &text[m.0 as usize..m.1 as usize] == what).map(|m| m.2);
        assert_eq!(of("Fix"), Some(BOLD | CODE | TABLE));
        assert_eq!(of("old"), Some(STRIKE | CODE | TABLE));
        assert_eq!(of("@Chief"), Some(LINK | CODE | TABLE));
        assert_eq!(of("Workstream"), Some(BOLD | CODE | TABLE));
        // The rest, block by block.
        for l in ["• Plan", "   ◦ draft", "      ▪ review", "☐ ship", "☑ test", "1. one", "────────────────────────", "shell", "npm run build"] {
            assert!(lines.contains(&l), "missing {l:?} in {lines:#?}");
        }
        assert_eq!(of("shell"), Some(QUOTE));
        assert_eq!(of("npm run build"), Some(CODE));
    }

    #[test]
    fn links_know_where_they_go() {
        let (text, _, links) = plain_with_links("See [the docs](https://example.org/a) and @plan.md.");
        let got: Vec<(&str, &str)> = links.iter().map(|(a, b, h)| (&text[*a as usize..*b as usize], h.as_str())).collect();
        assert_eq!(got, vec![("the docs", "https://example.org/a"), ("@plan.md", "@plan.md")]);
    }

    #[test]
    fn struck_through_and_pictures() {
        let (text, parts) = marked("~~old~~ new, ~ 5 and ![a cat](cat.png). Half: ~~stil");
        assert_eq!(text, "old new, ~ 5 and a cat. Half: stil");
        assert_eq!(parts, vec![("old".into(), STRIKE), ("stil".into(), STRIKE)]);
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
