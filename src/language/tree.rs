//! From tokens to a tree with no meaning yet: nodes with their header and
//! their block, and properties `name: value`. What each thing means is
//! decided by `compiler.rs`; that way the grammar fits on one page.

use super::tokens::{Token, TokenKind};
use super::CompileError;

#[derive(Clone, Debug)]
pub struct Node {
    /// What goes before the brace: `layer card ~calm`, `on hover orb for 320ms`.
    pub head: Vec<Token>,
    pub body: Option<Vec<Entry>>,
    pub line: usize,
    pub col: usize,
}

#[derive(Clone, Debug)]
pub enum Entry {
    Prop { name: String, value: Vec<Token>, line: usize, col: usize },
    Node(Node),
}

pub fn parse(tokens: &[Token]) -> Result<Vec<Entry>, CompileError> {
    let mut i = 0;
    let entries = block(tokens, &mut i, false)?;
    Ok(entries)
}

fn block(f: &[Token], i: &mut usize, inside: bool) -> Result<Vec<Entry>, CompileError> {
    let mut entries = Vec::new();
    loop {
        while *i < f.len() && matches!(f[*i].kind, TokenKind::Line | TokenKind::Sym(";")) {
            *i += 1;
        }
        let Some(first) = f.get(*i) else {
            if inside {
                let u = f.last().unwrap();
                return Err(CompileError::at(u.line, u.col, "a '}' is missing: some block was left open"));
            }
            return Ok(entries);
        };
        if first.kind == TokenKind::Sym("}") {
            if !inside {
                return Err(CompileError::at(first.line, first.col, "this '}' closes nothing"));
            }
            *i += 1;
            return Ok(entries);
        }
        // `name:` opens a property; anything else, a node.
        if let (TokenKind::Id(name), Some(TokenKind::Sym(":"))) = (&first.kind, f.get(*i + 1).map(|x| &x.kind)) {
            let (line, col) = (first.line, first.col);
            *i += 2;
            let from = *i;
            while *i < f.len() && !matches!(f[*i].kind, TokenKind::Line | TokenKind::Sym(";") | TokenKind::Sym("}")) {
                *i += 1;
            }
            if from == *i {
                return Err(CompileError::at(line, col, format!("'{name}:' was left without a value")));
            }
            entries.push(Entry::Prop { name: name.clone(), value: f[from..*i].to_vec(), line, col });
            continue;
        }
        let (line, col) = (first.line, first.col);
        let from = *i;
        while *i < f.len() && !matches!(f[*i].kind, TokenKind::Line | TokenKind::Sym(";") | TokenKind::Sym("{") | TokenKind::Sym("}")) {
            *i += 1;
        }
        let head = f[from..*i].to_vec();
        let body = if f.get(*i).is_some_and(|x| x.kind == TokenKind::Sym("{")) {
            *i += 1;
            Some(block(f, i, true)?)
        } else {
            None
        };
        entries.push(Entry::Node(Node { head, body, line, col }));
    }
}
