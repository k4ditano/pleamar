//! The scene language. A text file goes in and a `Scene` comes out —the same
//! one that until now was written in Rust—, or an error that says where and
//! why. Everything declared here is run by the render, on its own.
//!
//! Three steps: `tokens` chops, `tree` groups into nodes and properties without
//! knowing what they mean, and `compiler` gives them meaning and checks the names.

mod compiler;
mod figure;
mod tokens;
mod tree;
pub mod vocabulary;

use crate::scene::Scene;
use std::path::{Path, PathBuf};

/// A scene can be made of several files (`import`), and an error has to say
/// which one. So as not to load every token with a name, the line number
/// carries the file's inside it: line 12 of the third one is 2 000 012.
pub(crate) const PER_FILE: usize = 1_000_000;

/// The version of the language this program understands. The first number changes
/// when something written stops being valid; the second, when something is added. A file
/// can say which one it needs (`language 0.1`) and find out on load, not halfway through.
pub const VERSION: (u32, u32) = (0, 1);

#[derive(Debug)]
pub struct CompileError {
    pub line: usize,
    pub col: usize,
    pub message: String,
}

impl CompileError {
    pub fn at(line: usize, col: usize, message: impl Into<String>) -> CompileError {
        CompileError { line, col, message: message.into() }
    }

    /// With the file's line and an arrow underneath, the way a compiler shows it.
    fn with_source(&self, files: &[(PathBuf, String)]) -> String {
        let (path, source) = files.get(self.line / PER_FILE).or(files.first()).map_or((String::new(), ""), |(r, f)| {
            // Relative to where we are, if possible: a whole path gets in the way more than it helps.
            let short = std::env::current_dir().ok().and_then(|here| r.strip_prefix(here).ok().map(Path::to_owned)).unwrap_or_else(|| r.clone());
            (short.display().to_string(), f.as_str())
        });
        let n = self.line % PER_FILE;
        let line = source.lines().nth(n.saturating_sub(1)).unwrap_or("");
        let margin = format!("{n:>4} | ");
        format!("{path}:{n}:{}: {}\n{margin}{line}\n{}^", self.col, self.message, " ".repeat(margin.len() + self.col.saturating_sub(1)))
    }
}

/// "file:line" of something that was declared, to say where the thing being overridden was.
pub fn location(names: &[String], line: usize) -> String {
    format!("{}:{}", names.get(line / PER_FILE).map_or("", String::as_str), line % PER_FILE)
}

/// What has been read so far: each file once, with its text to show the errors.
#[derive(Default)]
struct Reader {
    files: Vec<(PathBuf, String)>,
    /// The ones being read right now, one inside another: to spot cycles.
    open_stack: Vec<PathBuf>,
    /// Which files are `strict` libraries, by their number.
    strict: Vec<usize>,
    /// Each library: its file number, its name, and its logic if it has a `.luau` next to it.
    libraries: Vec<compiler::Library>,
    /// What an editor has open and has not saved yet: it wins over the disk.
    unsaved: Vec<(PathBuf, String)>,
}

impl Reader {
    /// Tokenizes and groups a file, with its number put into the lines.
    fn open(&mut self, path: &Path) -> Result<Vec<tree::Entry>, CompileError> {
        let k = self.files.len();
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_owned());
        let source = match self.unsaved.iter().find(|(r, _)| *r == canonical) {
            Some((_, t)) => t.clone(),
            None => std::fs::read_to_string(path).map_err(|e| CompileError::at(0, 0, format!("cannot read {}: {e}", path.display())))?,
        };
        self.files.push((path.to_owned(), source));
        let here = |mut f: CompileError| { f.line += k * PER_FILE; f };
        let mut tokens = tokens::tokenize(&self.files[k].1).map_err(here)?;
        for f in &mut tokens {
            f.line += k * PER_FILE;
        }
        // Every file says which version it needs, a library too.
        check_version(tree::parse(&tokens).map_err(|f| if f.line < PER_FILE { here(f) } else { f })?)
    }

    /// What a file's `import`s declare, in order, and what is left of it.
    /// A library imported twice —by two routes— is read once.
    fn resolve_imports(&mut self, path: &Path, entries: Vec<tree::Entry>, brought: &mut Vec<tree::Entry>) -> Result<Vec<tree::Entry>, CompileError> {
        use tokens::TokenKind;
        let mut rest = Vec::new();
        for e in entries {
            let tree::Entry::Node(n) = &e else { rest.push(e); continue };
            if !matches!(n.head.first().map(|f| &f.kind), Some(TokenKind::Id(p)) if p == "import") {
                rest.push(e);
                continue;
            }
            let (Some(TokenKind::Str(which)), 2, None) = (n.head.get(1).map(|f| &f.kind), n.head.len(), &n.body) else {
                return Err(CompileError::at(n.line, n.col, "an import is `import \"path/to/the/library.plm\"`"));
            };
            // `pleamar:ui` is one of pleamar's own libraries: it comes inside the
            // program, so it is there wherever the scene is and whoever runs it.
            // Anything else is a path, relative to the importing file, not to
            // where it is launched from.
            let target = if let Some(name) = which.strip_prefix("pleamar:") {
                let Some(source) = builtin_library(name) else {
                    return Err(CompileError::at(n.line, n.head[1].col, format!("pleamar has no library '{name}': there is {}", BUILTIN_LIBRARIES.iter().map(|(n, _)| format!("pleamar:{n}")).collect::<Vec<_>>().join(", "))));
                };
                let t = PathBuf::from(format!("pleamar:{name}.plm"));
                if !self.unsaved.iter().any(|(r, _)| *r == t) {
                    self.unsaved.push((t.clone(), source.to_owned()));
                }
                t
            } else {
                let target = path.parent().unwrap_or(Path::new(".")).join(which);
                target.canonicalize().map_err(|e| CompileError::at(n.line, n.head[1].col, format!("cannot find '{which}' (looking in {}): {e}", target.display())))?
            };
            if self.open_stack.contains(&target) {
                return Err(CompileError::at(n.line, n.head[1].col, format!("'{which}' ends up importing itself: {}", self.open_stack.iter().chain([&target]).map(|p| p.file_name().unwrap_or_default().to_string_lossy()).collect::<Vec<_>>().join(" → "))));
            }
            if self.files.iter().any(|(r, _)| r == &target) {
                continue;
            }
            self.open_stack.push(target.clone());
            let number = self.files.len();
            let theirs = self.open(&target)?;
            let theirs = self.resolve_imports(&target, theirs, brought)?;
            self.open_stack.pop();
            let [tree::Entry::Node(b)] = theirs.as_slice() else {
                return Err(CompileError::at(n.line, n.head[1].col, format!("'{which}' is not a library: it has to be `library Name {{ … }}`, and nothing else")));
            };
            if !matches!(b.head.first().map(|f| &f.kind), Some(TokenKind::Id(p)) if p == "library") {
                return Err(CompileError::at(b.line, b.col, "what you import is a library: `library Name { … }`. A scene is not imported"));
            }
            let Some(tree::Entry::Node(b)) = theirs.into_iter().next() else { unreachable!() };
            let TokenKind::Id(library_name) = &b.head.get(1).map(|f| f.kind.clone()).unwrap_or(TokenKind::Id(String::new())) else {
                return Err(CompileError::at(b.line, b.col, "this library is missing its name: `library Name { … }`"));
            };
            let logic = target.with_extension("luau");
            self.libraries.push(compiler::Library { file: number, name: library_name.clone(), logic: logic.is_file().then_some(logic) });
            // `library Menu strict { … }`: its components only read what they ask for.
            match b.head.get(2).map(|f| &f.kind) {
                None => {}
                Some(TokenKind::Id(p)) if p == "strict" && b.head.len() == 3 => self.strict.push(number),
                Some(_) => return Err(CompileError::at(b.line, b.head[2].col, "after a library name only `strict` can go")),
            }
            for d in b.body.unwrap_or_default() {
                // A library declares; it does not paint, or react, or have a frontier with the logic.
                // `text now = "…"` declares; `text "hello" { … }` paints, and that is not a library's business.
                let ok = matches!(&d, tree::Entry::Node(x) if matches!(x.head.first().map(|f| &f.kind), Some(TokenKind::Id(p)) if vocabulary::LIBRARY_STATEMENTS.contains(&p.as_str())
                    && (!["text", "image"].contains(&p.as_str()) || matches!(x.head.get(2).map(|f| &f.kind), Some(TokenKind::Sym("="))))));
                if !ok {
                    let (l, c) = match &d { tree::Entry::Node(x) => (x.line, x.col), tree::Entry::Prop { line, col, .. } => (*line, *col) };
                    return Err(CompileError::at(l, c, "a library only declares: `let`, `spring`, `component`, its frontier (`fact`, `text`, `model`, `event`, `image x = …`, `permissions`) and what it moves inside (`prop`, `pose`, `gesture`, `posture`, `layer`). What gets painted, and loose rules, belong to the scene"));
                }
                brought.push(d);
            }
        }
        Ok(rest)
    }
}

/// pleamar's own libraries, inside the program: `import "pleamar:ui"`.
const BUILTIN_LIBRARIES: &[(&str, &str)] = &[("ui", include_str!("../../lib/ui.plm"))];

fn builtin_library(name: &str) -> Option<&'static str> {
    BUILTIN_LIBRARIES.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

/// `language 0.1`, if present, is the first thing in the file. It is checked and removed.
fn check_version(mut entries: Vec<tree::Entry>) -> Result<Vec<tree::Entry>, CompileError> {
    use tokens::TokenKind;
    let Some(tree::Entry::Node(n)) = entries.first() else { return Ok(entries) };
    if !matches!(n.head.first().map(|f| &f.kind), Some(TokenKind::Id(p)) if p == "language") {
        return Ok(entries);
    }
    let (Some(TokenKind::Num(v)), 2, None) = (n.head.get(1).map(|f| &f.kind), n.head.len(), &n.body) else {
        return Err(CompileError::at(n.line, n.col, format!("the version is asked for like this: `language {}.{}`", VERSION.0, VERSION.1)));
    };
    // `0.1` arrives as a number: the integer part and the first decimal.
    let asked = (v.trunc() as u32, ((v.fract() * 10.0).round()) as u32);
    if asked.0 != VERSION.0 || asked.1 > VERSION.1 {
        return Err(CompileError::at(n.line, n.head[1].col, format!("this file asks for language {}.{}, and this pleamar understands {}.{}", asked.0, asked.1, VERSION.0, VERSION.1)));
    }
    entries.remove(0);
    Ok(entries)
}

/// Reads a scene from its file, with whatever it imports. Returns the scene and all the
/// files it is made of —to watch them—, or the errors already with their file,
/// their line and their arrow.
pub fn read_file(path: &str) -> Result<(Scene, Vec<PathBuf>), String> {
    read_with(path, Vec::new()).map_err(|(errors, files)| {
        let n = errors.len();
        let text: Vec<String> = errors.iter().map(|f| f.with_source(&files)).collect();
        format!("{}\n{}", text.join("\n\n"), if n == 1 { "one error".to_owned() } else { format!("{n} errors") })
    })
}

/// A diagnostic for whoever is writing, as is: in which file, where, and what is wrong.
/// It is what the language server sends the editor while typing.
pub struct Diagnostic {
    pub file: PathBuf,
    pub line: usize,
    pub col: usize,
    pub message: String,
}

pub use compiler::Symbol;

/// What is wrong and what there is: both in one pass, which is how the editor wants them.
/// The line of each name already comes without the file number, and with it apart.
pub fn index(path: &str, unsaved: Vec<(PathBuf, String)>) -> (Vec<Diagnostic>, Vec<(PathBuf, Symbol)>) {
    let (r, symbols) = read_with_symbols(path, unsaved);
    let files: Vec<PathBuf> = match &r {
        Ok((_, f)) => f.clone(),
        Err((_, f)) => f.iter().map(|(p, _)| p.clone()).collect(),
    };
    let file_of = |line: usize| files.get(line / PER_FILE).cloned().unwrap_or_else(|| PathBuf::from(path));
    let names = symbols
        .into_iter()
        .map(|mut s| {
            let f = file_of(s.line);
            s.line %= PER_FILE;
            (f, s)
        })
        .collect();
    (diagnostics_for(path, r), names)
}

fn diagnostics_for(path: &str, r: Result<(Scene, Vec<PathBuf>), (Vec<CompileError>, Vec<(PathBuf, String)>)>) -> Vec<Diagnostic> {
    match r {
        Ok(_) => Vec::new(),
        Err((errors, files)) => errors
            .into_iter()
            .map(|f| Diagnostic {
                file: files.get(f.line / PER_FILE).map_or_else(|| PathBuf::from(path), |(r, _): &(PathBuf, String)| r.clone()),
                line: f.line % PER_FILE,
                col: f.col,
                message: f.message,
            })
            .collect(),
    }
}

fn read_with(path: &str, unsaved: Vec<(PathBuf, String)>) -> Result<(Scene, Vec<PathBuf>), (Vec<CompileError>, Vec<(PathBuf, String)>)> {
    let (r, _) = read_with_symbols(path, unsaved);
    r
}

/// The same, and also the names the scene declares with their location: the editor
/// needs them even when the file is half-written and does not compile.
fn read_with_symbols(path: &str, unsaved: Vec<(PathBuf, String)>) -> (Result<(Scene, Vec<PathBuf>), (Vec<CompileError>, Vec<(PathBuf, String)>)>, Vec<compiler::Symbol>) {
    let mut l = Reader { unsaved, ..Default::default() };
    let main = Path::new(path).canonicalize().unwrap_or_else(|_| PathBuf::from(path));
    let mut symbols = Vec::new();
    let compiled = (|| {
        l.open_stack.push(main.clone());
        let entries = l.open(Path::new(path)).map_err(|f| vec![f])?;
        l.files[0].0 = main.clone();
        let mut brought = Vec::new();
        let mut rest = l.resolve_imports(&main, entries, &mut brought).map_err(|f| vec![f])?;
        // What is imported goes before the scene's own, as if it were written there.
        if let [tree::Entry::Node(scene)] = rest.as_mut_slice() {
            if let Some(body) = scene.body.as_mut() {
                brought.append(body);
                *body = brought;
            }
        }
        let names: Vec<String> = l.files.iter().map(|(r, _)| r.file_name().unwrap_or_default().to_string_lossy().into_owned()).collect();
        let dirs: Vec<PathBuf> = l.files.iter().map(|(r, _)| r.parent().map_or_else(PathBuf::new, Path::to_owned)).collect();
        let (r, s) = compiler::compile(&rest, &names, &dirs, &l.strict, &l.libraries);
        symbols = s;
        r
    })();
    // When showing an error, the main file with the path given by whoever opened it.
    if let Some(f) = l.files.first_mut() {
        f.0 = PathBuf::from(path);
    }
    let r = match compiled {
        Ok(e) => Ok((e, l.files.into_iter().map(|(r, _)| r).collect())),
        Err(errors) => Err((errors, l.files)),
    };
    (r, symbols)
}

/// "Did you mean…?": the known name that looks most alike, if it looks alike enough.
pub fn closest_match<'a>(a: &str, known: impl Iterator<Item = &'a String>) -> Option<&'a String> {
    let distance = |x: &str, y: &str| {
        let (x, y): (Vec<char>, Vec<char>) = (x.chars().collect(), y.chars().collect());
        let mut row: Vec<usize> = (0..=y.len()).collect();
        for i in 1..=x.len() {
            let mut prev = row[0];
            row[0] = i;
            for j in 1..=y.len() {
                let saved = row[j];
                row[j] = (prev + (x[i - 1] != y[j - 1]) as usize).min(row[j] + 1).min(row[j - 1] + 1);
                prev = saved;
            }
        }
        row[y.len()]
    };
    known.map(|c| (distance(a, c), c)).filter(|(d, c)| *d <= 1 + c.len() / 4).min_by_key(|(d, _)| *d).map(|(_, c)| c)
}
