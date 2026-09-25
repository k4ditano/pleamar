//! `pleamar --lsp`: the language server. The same compiler that reads a scene,
//! speaking LSP over standard input and output, so that an editor says what is wrong
//! **while it is being written** and not when launching it.
//!
//! Three things, which are the ones used all the time: the errors with their location,
//! which words are valid here, and what the one under the cursor means. It all comes
//! from the vocabulary, so it cannot fall out of step with the real language.

use crate::language::vocabulary as vocab;
use crate::language::Symbol;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

pub fn serve() {
    let input = std::io::stdin();
    let mut input = input.lock();
    // What the editor has open, by its path: it may not be saved.
    let mut open: HashMap<PathBuf, String> = HashMap::new();
    // And what the scene declares, from the last time it was read whole.
    let mut names: Vec<(PathBuf, Symbol)> = Vec::new();
    eprintln!("lsp    · pleamar {}.{} listening on stdio", crate::language::VERSION.0, crate::language::VERSION.1);
    while let Some(m) = read(&mut input) {
        let method = m["method"].as_str().unwrap_or("").to_owned();
        let id = m.get("id").cloned();
        match method.as_str() {
            "initialize" => reply(id, json!({
                "capabilities": {
                    // 1 = the editor sends the whole file on every change. A scene
                    // file is a few hundred lines: compiling it costs milliseconds.
                    "textDocumentSync": { "openClose": true, "change": 1, "save": true },
                    "completionProvider": { "triggerCharacters": [" ", ":", "{"] },
                    "hoverProvider": true,
                    "definitionProvider": true,
                    "documentSymbolProvider": true,
                    "referencesProvider": true,
                    "renameProvider": true,
                },
                "serverInfo": { "name": "pleamar", "version": format!("{}.{}", crate::language::VERSION.0, crate::language::VERSION.1) },
            })),
            "shutdown" => reply(id, Value::Null),
            "exit" => return,
            "textDocument/didOpen" | "textDocument/didChange" | "textDocument/didSave" => {
                let d = &m["params"]["textDocument"];
                let Some(path) = uri_to_path(d["uri"].as_str().unwrap_or("")) else { continue };
                let text = match method.as_str() {
                    "textDocument/didOpen" => d["text"].as_str().map(str::to_owned),
                    _ => m["params"]["contentChanges"][0]["text"].as_str().map(str::to_owned),
                };
                match text {
                    Some(t) => { open.insert(path.clone(), t); }
                    // On save, the editor may send nothing: what is on disk is what counts.
                    None => { open.remove(&path); }
                }
                names = check_and_publish(&path, &open);
            }
            "textDocument/didClose" => {
                if let Some(path) = uri_to_path(m["params"]["textDocument"]["uri"].as_str().unwrap_or("")) {
                    open.remove(&path);
                    // On closing it, the editor keeps the old squiggles if nobody clears them.
                    publish(&path, Vec::new());
                }
            }
            "textDocument/completion" => {
                let (text, pos) = cursor_position(&m, &open);
                reply(id, json!({ "isIncomplete": false, "items": completions(&text, pos, &names) }));
            }
            // Go to where that name was born. It may be in another file: a library.
            "textDocument/definition" => {
                let (text, pos) = cursor_position(&m, &open);
                let word = word_at(&text, pos);
                // `rows.3.label` and `mon.$screen.title` lead to `rows` and to `mon`.
                let root = word.split('.').next().unwrap_or(&word);
                match names.iter().find(|(_, s)| s.class != "use" && (s.local == word || s.local == root)) {
                    Some((file, s)) => {
                        let theirs = open.get(file).cloned().unwrap_or_else(|| std::fs::read_to_string(file).unwrap_or_default());
                        let (l, c) = (s.line.saturating_sub(1), to_utf16_col(&theirs, s.line, s.col));
                        reply(id, json!({ "uri": path_to_uri(file), "range": { "start": { "line": l, "character": c }, "end": { "line": l, "character": c + s.local.chars().count() } } }));
                    }
                    None => reply(id, Value::Null),
                }
            }
            // Where this name is used, with its declaration first.
            "textDocument/references" => {
                let (text, pos) = cursor_position(&m, &open);
                let word = word_at(&text, pos);
                reply(id, Value::Array(occurrences(&word, &names, &open).into_iter().map(|(u, r)| json!({ "uri": u, "range": r })).collect()));
            }
            // Renaming something: in its declaration and in every place it is used.
            "textDocument/rename" => {
                let (text, pos) = cursor_position(&m, &open);
                let word = word_at(&text, pos);
                let new_name = m["params"]["newName"].as_str().unwrap_or("").to_owned();
                let mut changes: HashMap<String, Vec<Value>> = HashMap::new();
                for (u, r) in occurrences(&word, &names, &open) {
                    changes.entry(u).or_default().push(json!({ "range": r, "newText": new_name }));
                }
                reply(id, json!({ "changes": changes }));
            }
            // The outline of the file: what it declares, for the editor's index.
            "textDocument/documentSymbol" => {
                let Some(path) = uri_to_path(m["params"]["textDocument"]["uri"].as_str().unwrap_or("")) else { continue };
                let text = open.get(&path).cloned().unwrap_or_default();
                let theirs: Vec<Value> = names
                    .iter()
                    .filter(|(_, s)| s.class != "use")
                    .filter(|(f, _)| f.canonicalize().ok() == path.canonicalize().ok())
                    .map(|(_, s)| {
                        let (l, c) = (s.line.saturating_sub(1), to_utf16_col(&text, s.line, s.col));
                        let range = json!({ "start": { "line": l, "character": c }, "end": { "line": l, "character": c + s.local.chars().count() } });
                        json!({ "name": s.local, "detail": s.class, "kind": lsp_symbol_kind(&s.class), "range": range, "selectionRange": range })
                    })
                    .collect();
                reply(id, Value::Array(theirs));
            }
            "textDocument/hover" => {
                let (text, pos) = cursor_position(&m, &open);
                let word = word_at(&text, pos);
                // If it is a name from the scene, the first thing is what it is the name of.
                let theirs = names.iter().find(|(_, s)| s.local == word && s.class != "use").map(|(f, s)| {
                    format!("`{}` — {} declared in `{}`, line {}.", s.local, s.class, f.file_name().unwrap_or_default().to_string_lossy(), s.line)
                });
                match theirs.into_iter().chain(help_for(&word)).collect::<Vec<_>>() {
                    v if v.is_empty() => reply(id, Value::Null),
                    v => reply(id, json!({ "contents": { "kind": "markdown", "value": v.join("\n\n") } })),
                }
            }
            _ if id.is_some() => reply(id, Value::Null),
            _ => {}
        }
    }
}

// ── the protocol ────────────────────────────────────────────────

fn read(e: &mut impl BufRead) -> Option<Value> {
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        if e.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(n) = line.strip_prefix("Content-Length:") {
            length = n.trim().parse().ok()?;
        }
    }
    let mut body = vec![0u8; length];
    e.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

fn send_message(v: Value) {
    let body = v.to_string();
    let mut output = std::io::stdout().lock();
    let _ = write!(output, "Content-Length: {}\r\n\r\n{body}", body.len());
    let _ = output.flush();
}

fn reply(id: Option<Value>, result: Value) {
    let Some(id) = id else { return };
    send_message(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
}

/// `file:///home/scene.plm` → the path, with the %20 undone.
fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let mut s = String::with_capacity(rest.len());
    let mut c = rest.chars();
    while let Some(x) = c.next() {
        if x == '%' {
            let d: String = c.by_ref().take(2).collect();
            if let Ok(b) = u8::from_str_radix(&d, 16) {
                s.push(b as char);
                continue;
            }
        }
        s.push(x);
    }
    Some(PathBuf::from(s))
}

fn path_to_uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

// ── the errors ──────────────────────────────────────────────────

/// Compiles what is being written and spreads the errors by file. An error
/// can land in an imported library, so all of them are published and the files
/// that no longer have any are cleared.
fn check_and_publish(path: &Path, open: &HashMap<PathBuf, String>) -> Vec<(PathBuf, Symbol)> {
    let copy: Vec<(PathBuf, String)> = open.iter().map(|(r, t)| (r.canonicalize().unwrap_or_else(|_| r.clone()), t.clone())).collect();
    let (diagnostics, names) = crate::language::index(&path.display().to_string(), copy);
    let mut per_file: HashMap<PathBuf, Vec<Value>> = open.keys().map(|r| (r.clone(), Vec::new())).collect();
    per_file.entry(path.to_owned()).or_default();
    for a in diagnostics {
        let theirs = open.keys().find(|r| r.canonicalize().ok() == a.file.canonicalize().ok()).cloned().unwrap_or(a.file);
        let text = open.get(&theirs).cloned().unwrap_or_else(|| std::fs::read_to_string(&theirs).unwrap_or_default());
        let (l, c) = (a.line.saturating_sub(1), to_utf16_col(&text, a.line, a.col));
        per_file.entry(theirs).or_default().push(json!({
            "range": { "start": { "line": l, "character": c }, "end": { "line": l, "character": c + 1 } },
            "severity": 1,
            "source": "pleamar",
            "message": a.message,
        }));
    }
    for (file, diagnostics) in per_file {
        publish(&file, diagnostics);
    }
    names
}

/// Every place where that name appears: where it was declared and where it is used. A name
/// with parts (`rows.3.label`) counts for its root, but only the root is marked.
fn occurrences(word: &str, names: &[(PathBuf, Symbol)], open: &HashMap<PathBuf, String>) -> Vec<(String, Value)> {
    let root = word.split('.').next().unwrap_or(word);
    let mut out = Vec::new();
    for (file, s) in names {
        if s.local != word && s.local.split('.').next() != Some(root) {
            continue;
        }
        let text = open.get(file).cloned().unwrap_or_else(|| std::fs::read_to_string(file).unwrap_or_default());
        let (l, c) = (s.line.saturating_sub(1), to_utf16_col(&text, s.line, s.col));
        out.push((path_to_uri(file), json!({
            "start": { "line": l, "character": c },
            "end": { "line": l, "character": c + root.chars().count() },
        })));
    }
    out
}

/// The kind of symbol the editor understands, for its icon.
fn lsp_symbol_kind(class: &str) -> u8 {
    match class {
        "component" => 5,   // class
        "model" => 23,      // struct
        "fact" | "let" => 14, // constant
        "prop" | "pose" => 7, // property
        "text" => 15,       // string
        "event" => 24,      // event
        "surface" | "popup" => 2, // module
        _ => 13,            // variable
    }
}

fn publish(path: &Path, diagnostics: Vec<Value>) {
    send_message(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": { "uri": path_to_uri(path), "diagnostics": diagnostics },
    }));
}

/// The column the compiler gives (characters, from 1) as the editor wants it.
fn to_utf16_col(text: &str, line: usize, col: usize) -> usize {
    text
        .lines()
        .nth(line.saturating_sub(1))
        .map_or(col.saturating_sub(1), |l| l.chars().take(col.saturating_sub(1)).map(char::len_utf16).sum())
}

// ── where the cursor is ─────────────────────────────────────────

/// The text of the file and which byte the cursor is at.
fn cursor_position(m: &Value, open: &HashMap<PathBuf, String>) -> (String, usize) {
    let Some(path) = uri_to_path(m["params"]["textDocument"]["uri"].as_str().unwrap_or("")) else { return (String::new(), 0) };
    let text = open.get(&path).cloned().unwrap_or_else(|| std::fs::read_to_string(&path).unwrap_or_default());
    let (l, c) = (m["params"]["position"]["line"].as_u64().unwrap_or(0) as usize, m["params"]["position"]["character"].as_u64().unwrap_or(0) as usize);
    let mut offset = 0;
    for (n, line) in text.split_inclusive('\n').enumerate() {
        if n == l {
            // The editor counts in UTF-16; here bytes are needed.
            let mut u16s = 0;
            for (b, ch) in line.char_indices() {
                if u16s >= c {
                    return (text.clone(), offset + b);
                }
                u16s += ch.len_utf16();
            }
            return (text.clone(), offset + line.trim_end_matches('\n').len());
        }
        offset += line.len();
    }
    (text, offset)
}

/// The word under the cursor, or just before it.
fn word_at(text: &str, pos: usize) -> String {
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '.';
    let b = text.as_bytes();
    let mut i = pos.min(b.len());
    while i > 0 && is_word(b[i - 1] as char) {
        i -= 1;
    }
    let mut j = pos.min(b.len());
    while j < b.len() && is_word(b[j] as char) {
        j += 1;
    }
    text[i..j].to_owned()
}

/// Which block the cursor is in: the word that opened the brace that is still open.
/// `box { at: …` → "box", and with that it is known which properties are valid.
fn enclosing_block(text: &str, pos: usize) -> Option<String> {
    let upto = &text[..pos.min(text.len())];
    let mut depth = 0i32;
    let mut i = upto.len();
    let b = upto.as_bytes();
    while i > 0 {
        i -= 1;
        match b[i] {
            b'}' => depth += 1,
            b'{' if depth > 0 => depth -= 1,
            b'{' => {
                // The head of the block: the first word of what comes before the brace.
                let head = upto[..i].rsplit(['\n', ';', '}']).next().unwrap_or("").trim();
                return head.split_whitespace().next().map(str::to_owned);
            }
            _ => {}
        }
    }
    None
}

// ── what can be written here ────────────────────────────────────

fn item(word: &str, kind: u8, detail: &str) -> Value {
    json!({ "label": word, "kind": kind, "detail": detail, "documentation": { "kind": "markdown", "value": help_for(word).unwrap_or_default() } })
}

fn completions(text: &str, pos: usize, names: &[(PathBuf, Symbol)]) -> Vec<Value> {
    // What the scene declares, each thing with its class: it is what gets written the most.
    let theirs = |classes: &[&str]| -> Vec<Value> {
        let mut seen = std::collections::HashSet::new();
        names
            .iter()
            .filter(|(_, s)| s.class != "use")
            .filter(|(_, s)| classes.is_empty() || classes.contains(&s.class.as_str()))
            .filter(|(_, s)| seen.insert(s.local.clone()))
            .map(|(_, s)| json!({ "label": s.local, "kind": lsp_symbol_kind(&s.class), "detail": s.class }))
            .collect()
    };
    let before = &text[..pos.min(text.len())];
    let line = before.rsplit('\n').next().unwrap_or("");
    let last = line.split([';', '{', '(', ',']).next_back().unwrap_or("").trim_start();
    // `emit ` only takes events; `on ` expects what happens, and then a zone.
    if let Some(rest) = last.strip_prefix("emit ") {
        if !rest.contains(' ') {
            return theirs(&["event"]);
        }
    }
    if let Some(rest) = last.strip_prefix("on ") {
        return match rest.split_whitespace().count() {
            0 | 1 if !rest.ends_with(' ') => vocab::TRIGGERS.iter().map(|x| item(x, 14, "what a rule waits for")).chain(theirs(&["event"])).collect(),
            _ => theirs(&["zone", "box", "ellipse", "arc", "line", "path", "row", "column", "input"]),
        };
    }
    // After `property:` come its values, which are almost always from a closed list;
    // if not, an expression, and that is where the scene's names come in.
    if let Some((left, _)) = line.rsplit_once(':') {
        let prop = left.split([';', '{']).next_back().unwrap_or("").trim();
        if let Some(values) = values_for(prop) {
            return values.iter().map(|v| item(v, 12, prop)).collect();
        }
        return theirs(&[]).into_iter().chain(vocab::FUNCTIONS.iter().map(|x| item(x, 3, "function"))).collect();
    }
    let inside = enclosing_block(text, pos);
    match inside.as_deref() {
        // Inside an element: its properties and the ones valid in any shape.
        Some(p) if vocab::PROPERTIES.iter().any(|(n, _)| *n == p) => {
            let common: &[&str] = if ["ellipse", "box", "arc", "line", "path"].contains(&p) { vocab::properties("shape") } else { &[] };
            let steps: &[&str] = if p == "path" { vocab::PATH_COMMANDS } else { &[] };
            vocab::properties(p)
                .iter()
                .chain(common)
                .map(|x| item(x, 10, &format!("property of {p}")))
                .chain(steps.iter().map(|x| item(x, 3, "step of a path")))
                .collect()
        }
        Some("row") | Some("column") => vocab::properties("layout")
            .iter()
            .map(|x| item(x, 10, "property of a layout"))
            .chain(vocab::STATEMENTS.iter().map(|x| item(x, 14, "statement")))
            .collect(),
        // In the scene, or inside a group: any statement, and the components
        // that have been declared, which are copied by writing their name.
        _ => vocab::STATEMENTS.iter().map(|x| item(x, 14, "statement")).chain(theirs(&["component"])).collect(),
    }
}

/// The words that are valid as the value of a property.
fn values_for(prop: &str) -> Option<&'static [&'static str]> {
    Some(match prop {
        "anchor" => vocab::SURFACE_ANCHORS,
        "level" => vocab::LEVELS,
        "keyboard" => vocab::KEYBOARD_MODES,
        "align" => vocab::TEXT_ALIGNS,
        "cursor" => vocab::CURSORS,
        _ => return None,
    })
}

// ── what this word means ────────────────────────────────────────

/// What the editor shows on hover. It comes from the vocabulary's lists,
/// plus a line written for each statement (`vocab::HELP`).
fn help_for(word: &str) -> Option<String> {
    if word.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some((_, text)) = vocab::HELP.iter().find(|(n, _)| *n == word) {
        parts.push((*text).to_owned());
    }
    let owners: Vec<String> = vocab::PROPERTIES
        .iter()
        .filter(|(_, props)| props.contains(&word))
        .map(|(n, _)| format!("`{n}`"))
        .collect();
    if !owners.is_empty() {
        parts.push(format!("Property of {}.", owners.join(", ")));
    }
    for (list, name) in [
        (vocab::STATEMENTS, "a statement"),
        (vocab::FUNCTIONS, "a function"),
        (vocab::SPRINGS, "a spring"),
        (vocab::TRIGGERS, "what a rule waits for"),
        (vocab::EFFECTS, "what a rule does"),
        (vocab::TYPES, "the type of a field"),
        (vocab::PATH_COMMANDS, "a step of a path"),
        (vocab::CURVES, "a curve of a gesture"),
    ] {
        if list.contains(&word) && !parts.iter().any(|p| p.contains(name)) {
            parts.push(format!("`{word}` is {name}."));
        }
    }
    if let Some((_, fields)) = vocab::SERVICES.iter().find(|(n, _)| *n == word) {
        parts.push(format!("A service. It reports: {}.", fields.join(", ")));
    }
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

// ── the highlighting, drawn from the vocabulary ─────────────────

/// `pleamar --highlight vim|vscode|tree-sitter`. It writes it to the output, and since
/// it comes from the vocabulary it cannot lag behind the language: `run-tests.sh` checks it.
pub fn highlighting(which: &str) -> i32 {
    let words = |l: &[&str]| l.join(" ");
    match which {
        "vim" => {
            println!("\" pleamar syntax for Vim and Neovim. Made by `pleamar --highlight vim`:");
            println!("\" it is not written by hand, so it does not lag behind the language.");
            println!("if exists(\"b:current_syntax\") | finish | endif");
            println!("syn keyword plmStatement {}", words(vocab::STATEMENTS));
            println!("syn keyword plmStatement scene library import language");
            println!("syn keyword plmKeyword in max while for after from until at by reach within rest inset right middle as via strict each all");
            println!("syn keyword plmFunction {}", words(vocab::FUNCTIONS));
            println!("syn keyword plmFunction {}", words(vocab::TEXT_FUNCTIONS));
            println!("syn keyword plmTrigger {}", words(vocab::TRIGGERS));
            println!("syn keyword plmEffect {}", words(vocab::EFFECTS));
            println!("syn keyword plmStep {}", words(vocab::PATH_COMMANDS));
            println!("syn keyword plmConstant true false {} {} {}", words(vocab::SPRINGS), words(vocab::CURVES), words(vocab::TYPES));
            println!("syn match plmProperty \"\\<\\w\\+\\ze\\s*:\"");
            println!("syn match plmNumber \"\\<\\d\\+\\(\\.\\d\\+\\)\\?\\(px\\|%\\|deg\\|ms\\|s\\)\\?\\>\"");
            println!("syn match plmColour \"#[0-9a-fA-F]\\{{3,8}}\\>\"");
            println!("syn match plmSpring \"\\~\\w\\+\"");
            println!("syn region plmString start=/\"/ skip=/\\\\\"/ end=/\"/ contains=plmHole");
            println!("syn region plmHole start=/{{/ end=/}}/ contained");
            println!("syn match plmComment \"//.*$\"");
            println!("hi def link plmStatement Statement");
            println!("hi def link plmKeyword Keyword");
            println!("hi def link plmFunction Function");
            println!("hi def link plmTrigger Special");
            println!("hi def link plmEffect Special");
            println!("hi def link plmStep Type");
            println!("hi def link plmConstant Constant");
            println!("hi def link plmProperty Identifier");
            println!("hi def link plmNumber Number");
            println!("hi def link plmColour Constant");
            println!("hi def link plmSpring PreProc");
            println!("hi def link plmString String");
            println!("hi def link plmHole SpecialChar");
            println!("hi def link plmComment Comment");
            println!("let b:current_syntax = \"plm\"");
            0
        }
        "vscode" | "textmate" => {
            let o = |l: &[&str]| l.join("|");
            let rules = json!([
                { "name": "comment.line.double-slash.plm", "match": "//.*$" },
                { "name": "string.quoted.double.plm", "begin": "\"", "end": "\"",
                  "patterns": [{ "name": "variable.other.plm", "match": "\\{[^}]*\\}" }] },
                { "name": "constant.other.colour.plm", "match": "#[0-9a-fA-F]{3,8}\\b" },
                { "name": "constant.numeric.plm", "match": "\\b\\d+(\\.\\d+)?(px|%|deg|ms|s)?\\b" },
                { "name": "entity.other.attribute-name.plm", "match": "\\b\\w+(?=\\s*:)" },
                { "name": "keyword.control.plm", "match": format!("\\b({}|scene|library|import|language)\\b", o(vocab::STATEMENTS)) },
                { "name": "keyword.other.plm", "match": "\\b(in|max|while|for|after|from|until|at|by|reach|within|rest|inset|right|middle|as|via|strict|each|all)\\b" },
                { "name": "support.function.plm", "match": format!("\\b({}|{})\\b", o(vocab::FUNCTIONS), o(vocab::TEXT_FUNCTIONS)) },
                { "name": "entity.name.tag.plm", "match": format!("\\b({})\\b", o(vocab::TRIGGERS)) },
                { "name": "keyword.operator.plm", "match": format!("\\b({})\\b", o(vocab::EFFECTS)) },
                { "name": "storage.type.plm", "match": format!("\\b({})\\b", o(vocab::PATH_COMMANDS)) },
                { "name": "constant.language.plm", "match": format!("\\b(true|false|{}|{})\\b", o(vocab::SPRINGS), o(vocab::CURVES)) },
                { "name": "support.constant.spring.plm", "match": "~\\w+" },
            ]);
            println!("{:#}", json!({
                "$comment": "Made by `pleamar --highlight vscode`: it is not written by hand.",
                "name": "pleamar", "scopeName": "source.plm", "fileTypes": ["plm"], "patterns": rules,
            }));
            0
        }
        other => {
            eprintln!("'{other}': the highlighters that can be written are 'vim' and 'vscode'");
            1
        }
    }
}
