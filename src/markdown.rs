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
/// A table's cell (`| a | b |`): where it goes is a `Block::Table`.
pub const TABLE: u8 = 128;
/// The marks that change the layout; the others only colour it.
pub const LOOKS: u8 = BOLD | ITALIC | CODE;

/// What the layout places itself, not as lines that follow each other: byte
/// ranges of the plain text.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum Block {
    /// A point of a list, its lines below it included: from its start, past its
    /// mark (`• `, `1. `, and the spaces before them) where its words start, to its
    /// end. Its words wrap and go on under where they start, not under the mark.
    Hang { from: u32, body: u32, to: u32 },
    /// A table: its rows (the first its head), each its cells, each a byte range.
    /// In the plain text a cell is followed by a tab and a row by a line break,
    /// which is what is copied.
    Table { from: u32, to: u32, rows: Vec<Vec<(u32, u32)>> },
}

impl Block {
    pub fn ends(&self) -> (u32, u32) {
        match self {
            Block::Hang { from, to, .. } | Block::Table { from, to, .. } => (*from, *to),
        }
    }
}

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
#[cfg(test)]
pub fn plain_with_links(source: &str) -> (String, Vec<(u32, u32, u8)>, Vec<(u32, u32, String)>) {
    let (text, marks, links, _) = plain_with_pictures(source, 0);
    (text, marks, links)
}

/// A picture alone on its line (`![what it shows](where)`), when the text shows
/// them (`images:`): the byte its room starts at in the plain text, how many
/// empty lines that room is, and where the picture is.
pub type Picture = (u32, u32, String);

/// A fenced block shown as a block (`code_block:` on the text): where its head
/// starts, where its «Copy» is, where it ends, its code as written, and how
/// many figures its line numbers take.
#[derive(Clone, Debug, PartialEq)]
pub struct CodeBlock {
    pub head: u32,
    pub copy: (u32, u32),
    pub end: u32,
    pub code: String,
    numbers: usize,
}

thread_local! {
    /// Whether fenced blocks are shown as blocks while a text is read (`code_block:`).
    static BOXED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static SHORT_BLANKS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static LIST_INDENT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static CODE_BLOCKS: std::cell::RefCell<Vec<CodeBlock>> = const { std::cell::RefCell::new(Vec::new()) };
    /// How many lines a picture takes while a text is read (0: pictures are their words).
    static PICTURE_LINES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static PICTURES: std::cell::RefCell<Vec<Picture>> = const { std::cell::RefCell::new(Vec::new()) };
    static BLOCKS: std::cell::RefCell<Vec<Block>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The same, and with `lines` above 0, each picture alone on its line is room of
/// that many empty lines for it to be painted in (`images:` on the text).
#[cfg(test)]
pub fn plain_with_pictures(source: &str, lines: u32) -> (String, Vec<(u32, u32, u8)>, Vec<(u32, u32, String)>, Vec<Picture>) {
    let r = read(source, Options { picture_lines: lines, ..Default::default() });
    (r.text, r.marks, r.links, r.pictures)
}

/// All a markdown text is, read: its plain text, its marks, its links, its
/// pictures (with `picture_lines` above 0) and its fenced blocks as blocks
/// (with `boxed`: `code_block:` on the text).
pub struct Read {
    pub text: String,
    pub marks: Vec<(u32, u32, u8)>,
    pub links: Vec<(u32, u32, String)>,
    pub pictures: Vec<Picture>,
    pub code: Vec<CodeBlock>,
    /// The points of its lists and its tables, for the layout to place.
    pub blocks: Vec<Block>,
}

/// How a text with `markdown:` asks to be read.
#[derive(Clone, Copy, Default)]
pub struct Options {
    /// How many lines a picture alone on its line takes (`images:`); 0, its words.
    pub picture_lines: u32,
    /// Fenced blocks as blocks (`code_block:`).
    pub boxed: bool,
    /// A blank line between blocks as a lone space, for the layout to make it as
    /// tall as `paragraph:` says (a line that is only a space).
    pub paragraph: bool,
    /// Spaces before every point of a list, the first level too (`list_indent:`).
    pub list_indent: usize,
}

pub fn read(source: &str, o: Options) -> Read {
    BOXED.with(|b| b.set(o.boxed));
    SHORT_BLANKS.with(|b| b.set(o.paragraph));
    LIST_INDENT.with(|b| b.set(o.list_indent));
    CODE_BLOCKS.with(|c| c.borrow_mut().clear());
    BLOCKS.with(|c| c.borrow_mut().clear());
    let (text, marks, links, pictures) = plain_with_pictures_inner(source, o.picture_lines);
    BOXED.with(|b| b.set(false));
    SHORT_BLANKS.with(|b| b.set(false));
    LIST_INDENT.with(|b| b.set(0));
    Read { text, marks, links, pictures, code: CODE_BLOCKS.with(|c| std::mem::take(&mut *c.borrow_mut())), blocks: BLOCKS.with(|c| std::mem::take(&mut *c.borrow_mut())) }
}

fn plain_with_pictures_inner(source: &str, lines: u32) -> (String, Vec<(u32, u32, u8)>, Vec<(u32, u32, String)>, Vec<Picture>) {
    LINKS.with(|l| l.borrow_mut().clear());
    PICTURES.with(|p| p.borrow_mut().clear());
    PICTURE_LINES.with(|p| p.set(lines));
    let (text, marks) = blocks(source);
    PICTURE_LINES.with(|p| p.set(0));
    (text, marks, LINKS.with(|l| std::mem::take(&mut *l.borrow_mut())), PICTURES.with(|p| std::mem::take(&mut *p.borrow_mut())))
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
            let lang = lang.trim();
            if BOXED.with(|b| b.get()) {
                // As a block (`code_block:`): a head with its language and «Copy», and each
                // line with its number; the box, its lines and the code go to `CODE_BLOCKS`.
                if fenced {
                    // How many lines it has, for the numbers' width.
                    let count = lines[n..].iter().take_while(|l| !l.trim_start().starts_with("```")).count();
                    start(&mut out, &mut looks, &mut first);
                    let head = out.len() as u32;
                    push(&mut out, &mut looks, if lang.is_empty() { "code" } else { lang }, QUOTE);
                    push(&mut out, &mut looks, "   ", 0);
                    let copy = out.len() as u32;
                    push(&mut out, &mut looks, "Copy", LINK);
                    CODE_BLOCKS.with(|c| c.borrow_mut().push(CodeBlock { head, copy: (copy, out.len() as u32), end: out.len() as u32, code: String::new(), numbers: count.to_string().len() }));
                } else {
                    CODE_BLOCKS.with(|c| {
                        if let Some(b) = c.borrow_mut().last_mut() {
                            b.end = out.len() as u32;
                        }
                    });
                }
                continue;
            }
            // ```rust: the language, over the block, as a quote is.
            if fenced && !lang.is_empty() {
                start(&mut out, &mut looks, &mut first);
                push(&mut out, &mut looks, lang, QUOTE);
            }
            continue;
        }
        if fenced {
            start(&mut out, &mut looks, &mut first);
            if BOXED.with(|b| b.get()) {
                CODE_BLOCKS.with(|c| {
                    if let Some(b) = c.borrow_mut().last_mut() {
                        if !b.code.is_empty() || b.end > b.copy.1 {
                            b.code.push('\n');
                        }
                        b.code.push_str(line);
                        let k = b.code.matches('\n').count() + 1;
                        push(&mut out, &mut looks, &format!("{k:>w$}  ", w = b.numbers), QUOTE);
                    }
                });
                push(&mut out, &mut looks, line, CODE);
                CODE_BLOCKS.with(|c| {
                    if let Some(b) = c.borrow_mut().last_mut() {
                        b.end = out.len() as u32;
                    }
                });
                continue;
            }
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
        // A picture alone on its line, when the text shows them: room for it.
        let room = PICTURE_LINES.with(|p| p.get());
        if room > 0 {
            let t = bare.trim_end();
            if let Some(inside) = t.strip_prefix("![").and_then(|r| r.strip_suffix(')')) {
                if let Some((_, src)) = inside.split_once("](") {
                    let from = out.len() as u32;
                    push(&mut out, &mut looks, &"\n".repeat(room as usize - 1), 0);
                    PICTURES.with(|p| p.borrow_mut().push((from, room, src.trim().to_owned())));
                    continue;
                }
            }
        }
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
            let from = out.len() as u32;
            push(&mut out, &mut looks, &(" ".repeat(LIST_INDENT.with(|b| b.get())) + &"   ".repeat(depth)), 0);
            // `- [ ]` and `- [x]`: a box to tick, empty or ticked.
            let (mark, rest) = match rest.get(..4) {
                Some("[ ] ") => ("☐ ", &rest[4..]),
                Some("[x] " | "[X] ") => ("☑ ", &rest[4..]),
                _ => (["• ", "◦ ", "▪ "][depth.min(2)], rest),
            };
            push(&mut out, &mut looks, mark, 0);
            let body = out.len() as u32;
            inline(rest, 0, &mut out, &mut looks);
            n = point_goes_on(&lines, n, columns(indent), &mut out, &mut looks);
            BLOCKS.with(|b| b.borrow_mut().push(Block::Hang { from, body, to: out.len() as u32 }));
        } else if digits > 0 && (bare[digits..].starts_with(". ") || bare[digits..].starts_with(") ")) {
            let from = out.len() as u32;
            push(&mut out, &mut looks, &(" ".repeat(LIST_INDENT.with(|b| b.get())) + &"   ".repeat(depth)), 0);
            push(&mut out, &mut looks, &bare[..digits + 2], 0);
            let body = out.len() as u32;
            inline(&bare[digits + 2..], 0, &mut out, &mut looks);
            n = point_goes_on(&lines, n, columns(indent), &mut out, &mut looks);
            BLOCKS.with(|b| b.borrow_mut().push(Block::Hang { from, body, to: out.len() as u32 }));
        } else if let Some(rest) = bare.strip_prefix("> ") {
            push(&mut out, &mut looks, indent, 0);
            inline(rest, ITALIC | QUOTE, &mut out, &mut looks);
        } else if line.trim().is_empty() && SHORT_BLANKS.with(|b| b.get()) {
            // A blank line between blocks: a lone space, which the layout makes `paragraph:` tall.
            push(&mut out, &mut looks, " ", 0);
        } else {
            inline(line, 0, &mut out, &mut looks);
        }
    }
    (out, stretches(&looks))
}

/// How wide an indent is, a tab as two spaces.
fn columns(indent: &str) -> usize {
    indent.chars().map(|c| if c == '\t' { 2 } else { 1 }).sum()
}

/// The lines under a point of a list that are still its own: indented past
/// where its mark is (`1. **Estefanía**:` and under it, three spaces in, the
/// letter), blank lines between them included, but not a point of its own.
/// They are added to it, each a line, without their indent: the point's words
/// start where they go on. The line it goes on from.
fn point_goes_on(lines: &[&str], mut n: usize, mark_at: usize, out: &mut String, looks: &mut Vec<u8>) -> usize {
    let goes_on = |l: &str| {
        let bare = l.trim_start();
        let digits = bare.bytes().take_while(u8::is_ascii_digit).count();
        let point = ["- ", "* ", "+ "].iter().any(|m| bare.starts_with(m)) || (digits > 0 && (bare[digits..].starts_with(". ") || bare[digits..].starts_with(") ")));
        !bare.is_empty() && columns(&l[..l.len() - bare.len()]) >= mark_at + 2 && !point && !bare.starts_with("```") && !bare.starts_with('|')
    };
    loop {
        let blanks = lines[n.min(lines.len())..].iter().take_while(|l| l.trim().is_empty()).count();
        let Some(next) = lines.get(n + blanks).filter(|l| goes_on(l)) else { return n };
        for _ in 0..blanks {
            push(out, looks, if SHORT_BLANKS.with(|b| b.get()) { "\n " } else { "\n" }, 0);
        }
        push(out, looks, "\n", 0);
        inline(next.trim(), 0, out, looks);
        n += blanks + 1;
    }
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

/// A table: each cell with its own marks (the head's in bold), a tab after
/// each and a line break after each row; where they go is the layout's.
fn table(rows: &[Vec<&str>], out: &mut String, looks: &mut Vec<u8>, first: &mut bool) {
    if !std::mem::take(first) {
        push(out, looks, "\n", 0);
    }
    let from = out.len() as u32;
    let mut placed = Vec::with_capacity(rows.len());
    for (r, row) in rows.iter().enumerate() {
        if r > 0 {
            push(out, looks, "\n", 0);
        }
        let mut cells = Vec::with_capacity(row.len());
        for (k, cell) in row.iter().enumerate() {
            if k > 0 {
                push(out, looks, "\t", 0);
            }
            let a = out.len() as u32;
            inline(cell, TABLE | if r == 0 { BOLD } else { 0 }, out, looks);
            cells.push((a, out.len() as u32));
        }
        placed.push(cells);
    }
    BLOCKS.with(|b| b.borrow_mut().push(Block::Table { from, to: out.len() as u32, rows: placed }));
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
                let opens = !strike && at(k + 2).is_some_and(|n| !n.is_whitespace());
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

/// The `@name`s in a plain text (`@Research`, `@launch-brief.md`), as bytes:
/// an `@` at the start or after a space or `(`, a letter or digit after it,
/// and the name up to what is not a letter, digit or `._-/` (a full stop or
/// dash at its end is the sentence's). A field with `mentions:` shows them as chips.
pub fn mentions(text: &str) -> Vec<(usize, usize)> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let at = |k: usize| chars.get(k).map(|c| c.1);
    let mut out = Vec::new();
    let mut k = 0;
    while k < chars.len() {
        if chars[k].1 == '@' && !k.checked_sub(1).and_then(at).is_some_and(|b| !b.is_whitespace() && b != '(') && at(k + 1).is_some_and(char::is_alphanumeric) {
            let mut e = k + 1;
            while at(e).is_some_and(|n| n.is_alphanumeric() || "._-/".contains(n)) {
                e += 1;
            }
            while e > k + 1 && at(e - 1).is_some_and(|n| ".-/".contains(n)) {
                e -= 1;
            }
            out.push((chars[k].0, chars.get(e).map_or(text.len(), |c| c.0)));
            k = e;
        } else {
            k += 1;
        }
    }
    out
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
        let r = read(THREAD, Options::default());
        let (text, marks) = (&r.text, &r.marks);
        let lines: Vec<&str> = text.split('\n').collect();
        // The table: its head and three rows, a tab between cells, which is what is copied.
        for row in ["Workstream\tOwner\tStatus", "Brief\t@Chief\tFix", "Metrics\t@Research\told new", "Copy\t@Launch\tdone"] {
            assert!(lines.contains(&row), "missing {row:?} in {lines:#?}");
        }
        let Some(Block::Table { from, to, rows }) = r.blocks.iter().find(|b| matches!(b, Block::Table { .. })) else { panic!("no table: {:?}", r.blocks) };
        assert_eq!(&text[*from as usize..*to as usize], "Workstream\tOwner\tStatus\nBrief\t@Chief\tFix\nMetrics\t@Research\told new\nCopy\t@Launch\tdone");
        let cell = |r: usize, k: usize| &text[rows[r][k].0 as usize..rows[r][k].1 as usize];
        assert_eq!((rows.len(), cell(0, 2), cell(2, 1), cell(3, 2)), (4, "Status", "@Research", "done"));
        let of = |what: &str| marks.iter().find(|m| &text[m.0 as usize..m.1 as usize] == what).map(|m| m.2);
        assert_eq!(of("Fix"), Some(BOLD | TABLE));
        assert_eq!(of("old"), Some(STRIKE | TABLE));
        assert_eq!(of("@Chief"), Some(LINK | TABLE));
        assert_eq!(of("Workstream\tOwner\tStatus"), None, "the tabs between head cells are not bold");
        assert_eq!(of("Workstream"), Some(BOLD | TABLE));
        // The rest, block by block.
        for l in ["• Plan", "   ◦ draft", "      ▪ review", "☐ ship", "☑ test", "1. one", "────────────────────────", "shell", "npm run build"] {
            assert!(lines.contains(&l), "missing {l:?} in {lines:#?}");
        }
        assert_eq!(of("shell"), Some(QUOTE));
        assert_eq!(of("npm run build"), Some(CODE));
    }

    #[test]
    fn a_point_goes_on_in_the_lines_indented_under_it() {
        let src = "Borradores:\n\n1. **Estefanía**:\n   \"Buenos días Estefanía,\n   te escribo por lo de ayer.\n\n   Un saludo\"\n2. **Luis**: nada\n- a point\n  - nested\n    still nested\nafter";
        let r = read(src, Options::default());
        assert_eq!(r.text, "Borradores:\n\n1. Estefanía:\n\"Buenos días Estefanía,\nte escribo por lo de ayer.\n\nUn saludo\"\n2. Luis: nada\n• a point\n   ◦ nested\nstill nested\nafter");
        let hangs: Vec<(&str, &str)> = r.blocks.iter().map(|b| match b {
            Block::Hang { from, body, to } => (&r.text[*from as usize..*body as usize], &r.text[*body as usize..*to as usize]),
            _ => panic!("{b:?}"),
        }).collect();
        assert_eq!(hangs, vec![
            ("1. ", "Estefanía:\n\"Buenos días Estefanía,\nte escribo por lo de ayer.\n\nUn saludo\""),
            ("2. ", "Luis: nada"),
            ("• ", "a point"),
            ("   ◦ ", "nested\nstill nested"),
        ]);
        // With `paragraph:`, the blank line in a point is short too.
        assert!(read(src, Options { paragraph: true, ..Default::default() }).text.contains("ayer.\n \nUn saludo"));
    }

    #[test]
    fn links_know_where_they_go() {
        let (text, _, links) = plain_with_links("See [the docs](https://example.org/a) and @plan.md.");
        let got: Vec<(&str, &str)> = links.iter().map(|(a, b, h)| (&text[*a as usize..*b as usize], h.as_str())).collect();
        assert_eq!(got, vec![("the docs", "https://example.org/a"), ("@plan.md", "@plan.md")]);
    }

    #[test]
    fn blank_lines_short_and_lists_indented() {
        let r = read("One.\n\nTwo:\n- a\n  - b", Options { paragraph: true, list_indent: 2, ..Default::default() });
        assert_eq!(r.text, "One.\n \nTwo:\n  • a\n     ◦ b");
    }

    #[test]
    fn a_fenced_block_as_a_block() {
        let r = read("Run:\n```sh\nnpm i\nnpm run build\n```\ndone", Options { boxed: true, ..Default::default() });
        assert_eq!(r.text, "Run:\nsh   Copy\n1  npm i\n2  npm run build\ndone");
        assert_eq!(r.code.len(), 1);
        let b = &r.code[0];
        assert_eq!(&r.text[b.copy.0 as usize..b.copy.1 as usize], "Copy");
        assert_eq!(b.code, "npm i\nnpm run build");
        assert_eq!(&r.text[b.head as usize..b.end as usize], "sh   Copy\n1  npm i\n2  npm run build");
    }

    #[test]
    fn a_picture_alone_takes_its_room() {
        let (text, _, _, pictures) = plain_with_pictures("Look:\n![a cat](/tmp/cat.png)\nnice", 3);
        assert_eq!(text, "Look:\n\n\n\nnice");
        assert_eq!(pictures, vec![(6, 3, "/tmp/cat.png".to_owned())]);
        // Without `images:`, its words.
        assert_eq!(plain_with_pictures("![a cat](/tmp/cat.png)", 0).0, "a cat");
    }

    #[test]
    fn struck_through_and_pictures() {
        let (text, parts) = marked("~~old~~ new, ~ 5 and ![a cat](cat.png). Half: ~~stil");
        assert_eq!(text, "old new, ~ 5 and a cat. Half: stil");
        assert_eq!(parts, vec![("old".into(), STRIKE), ("stil".into(), STRIKE)]);
    }

    #[test]
    fn mentions_in_a_field() {
        let t = "ask @Chief about @launch-brief.md. mail a@b.c";
        let got: Vec<&str> = mentions(t).into_iter().map(|(a, b)| &t[a..b]).collect();
        assert_eq!(got, vec!["@Chief", "@launch-brief.md"]);
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
