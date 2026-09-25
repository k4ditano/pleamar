//! From text to tokens: numbers with their unit, colours, names with dots.

use super::CompileError;

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    Id(String),
    Num(f32),
    /// A duration, already in seconds: `320ms`, `14s`.
    Dur(f32),
    Color([f32; 3]),
    Str(String),
    Sym(&'static str),
    Line,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
}

const SYMBOLS: [&str; 23] = ["->", "..", "<=", ">=", "==", "!=", "{", "}", "(", ")", ",", ":", ";", "=", "~", "+", "-", "*", "/", "<", ">", "%", "|"];

pub fn tokenize(source: &str) -> Result<Vec<Token>, CompileError> {
    let mut tokens = Vec::new();
    // Inside a parenthesis a line break ends nothing.
    let mut parens = 0usize;
    for (n, line) in source.lines().enumerate() {
        let c: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < c.len() {
            let (ch, col) = (c[i], i + 1);
            let mut push = |kind: TokenKind| tokens.push(Token { kind, line: n + 1, col });
            if ch.is_whitespace() {
                i += 1;
            } else if ch == '/' && c.get(i + 1) == Some(&'/') {
                break;
            } else if ch == '"' {
                let end = c[i + 1..].iter().position(|x| *x == '"').ok_or_else(|| CompileError::at(n + 1, col, "this string is not closed: the quote is missing"))?;
                push(TokenKind::Str(c[i + 1..i + 1 + end].iter().collect()));
                i += end + 2;
            } else if ch == '#' {
                let hex: String = c[i + 1..].iter().take_while(|x| x.is_ascii_hexdigit()).collect();
                // #151616, or the three-digit shortcut: #fff is #ffffff.
                let length = hex.len();
                let hex: String = match length {
                    6 => hex,
                    3 => hex.chars().flat_map(|c| [c, c]).collect(),
                    _ => return Err(CompileError::at(n + 1, col, "a colour is six hex digits (#151616) or three (#fff)")),
                };
                let v = |k: usize| u8::from_str_radix(&hex[k..k + 2], 16).unwrap() as f32 / 255.0;
                push(TokenKind::Color([v(0), v(2), v(4)]));
                i += length + 1;
            } else if ch.is_ascii_digit() || (ch == '.' && c.get(i + 1).is_some_and(|x| x.is_ascii_digit())) {
                let mut j = i;
                while j < c.len() && (c[j].is_ascii_digit() || (c[j] == '.' && c.get(j + 1) != Some(&'.'))) {
                    j += 1;
                }
                let number: f32 = c[i..j].iter().collect::<String>().parse().map_err(|_| CompileError::at(n + 1, col, "this number cannot be read"))?;
                let unit: String = c[j..].iter().take_while(|x| x.is_ascii_alphabetic() || **x == '%').collect();
                let kind = match unit.as_str() {
                    "" | "px" => TokenKind::Num(number),
                    "%" => TokenKind::Num(number / 100.0),
                    "deg" => TokenKind::Num(number.to_radians()),
                    "ms" => TokenKind::Dur(number / 1000.0),
                    "s" => TokenKind::Dur(number),
                    other if super::vocabulary::UNITS.contains(&other) => unreachable!("'{other}' is in the vocabulary, but the tokenizer cannot convert it"),
                    other => return Err(CompileError::at(n + 1, j + 1, format!("I don't know the unit '{other}': valid ones are {}", super::vocabulary::UNITS.join(", ")))),
                };
                push(kind);
                i = j + unit.chars().count();
            } else if ch.is_alphabetic() || ch == '_' || ch == '$' {
                let mut j = i;
                while j < c.len() && (c[j].is_alphanumeric() || c[j] == '_' || c[j] == '$' || (c[j] == '.' && c.get(j + 1).is_some_and(|x| x.is_alphanumeric() || *x == '_' || *x == '$'))) {
                    j += 1;
                }
                push(TokenKind::Id(c[i..j].iter().collect()));
                i = j;
            } else {
                let rest: String = c[i..].iter().take(2).collect();
                let s = SYMBOLS.iter().find(|s| rest.starts_with(**s)).ok_or_else(|| CompileError::at(n + 1, col, format!("I don't know what to do with '{ch}'")))?;
                match *s {
                    "(" => parens += 1,
                    ")" => parens = parens.saturating_sub(1),
                    _ => {}
                }
                push(TokenKind::Sym(s));
                i += s.len();
            }
        }
        if parens == 0 {
            tokens.push(Token { kind: TokenKind::Line, line: n + 1, col: c.len() + 1 });
        }
    }
    Ok(tokens)
}
