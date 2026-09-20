//! `pleamar --lsp`: el servidor de lenguaje. El mismo compilador que lee una escena,
//! hablando LSP por la entrada y la salida, para que un editor diga lo que está mal
//! **mientras se escribe** y no al lanzarla.
//!
//! Tres cosas, que son las que se usan todo el rato: los fallos con su sitio, qué
//! palabras valen aquí, y qué significa la que está bajo el cursor. Todo sale del
//! vocabulario, así que no se puede desfasar del lenguaje de verdad.

use crate::lenguaje::vocabulario as voz;
use crate::lenguaje::Simbolo;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

pub fn servir() {
    let entrada = std::io::stdin();
    let mut entrada = entrada.lock();
    // Lo que el editor tiene abierto, por su ruta: puede no estar guardado.
    let mut abiertos: HashMap<PathBuf, String> = HashMap::new();
    // Y lo que la escena declara, de la última vez que se leyó entera.
    let mut nombres: Vec<(PathBuf, Simbolo)> = Vec::new();
    eprintln!("lsp    · pleamar {}.{} listening on stdio", crate::lenguaje::VERSION.0, crate::lenguaje::VERSION.1);
    while let Some(m) = leer(&mut entrada) {
        let metodo = m["method"].as_str().unwrap_or("").to_owned();
        let id = m.get("id").cloned();
        match metodo.as_str() {
            "initialize" => contestar(id, json!({
                "capabilities": {
                    // 1 = el editor manda el fichero entero en cada cambio. Un fichero de
                    // escena son unos cientos de líneas: compilarlo cuesta milisegundos.
                    "textDocumentSync": { "openClose": true, "change": 1, "save": true },
                    "completionProvider": { "triggerCharacters": [" ", ":", "{"] },
                    "hoverProvider": true,
                    "definitionProvider": true,
                    "documentSymbolProvider": true,
                },
                "serverInfo": { "name": "pleamar", "version": format!("{}.{}", crate::lenguaje::VERSION.0, crate::lenguaje::VERSION.1) },
            })),
            "shutdown" => contestar(id, Value::Null),
            "exit" => return,
            "textDocument/didOpen" | "textDocument/didChange" | "textDocument/didSave" => {
                let d = &m["params"]["textDocument"];
                let Some(ruta) = ruta_de(d["uri"].as_str().unwrap_or("")) else { continue };
                let texto = match metodo.as_str() {
                    "textDocument/didOpen" => d["text"].as_str().map(str::to_owned),
                    _ => m["params"]["contentChanges"][0]["text"].as_str().map(str::to_owned),
                };
                match texto {
                    Some(t) => { abiertos.insert(ruta.clone(), t); }
                    // Al guardar, el editor puede no mandar nada: lo que hay en disco vale.
                    None => { abiertos.remove(&ruta); }
                }
                nombres = revisar_y_contar(&ruta, &abiertos);
            }
            "textDocument/didClose" => {
                if let Some(ruta) = ruta_de(m["params"]["textDocument"]["uri"].as_str().unwrap_or("")) {
                    abiertos.remove(&ruta);
                    // Al cerrarlo, el editor se queda con los subrayados de antes si nadie los quita.
                    publicar(&ruta, Vec::new());
                }
            }
            "textDocument/completion" => {
                let (texto, donde) = donde_esta(&m, &abiertos);
                contestar(id, json!({ "isIncomplete": false, "items": completado(&texto, donde, &nombres) }));
            }
            // Ir a donde nació ese nombre. Puede estar en otro fichero: una biblioteca.
            "textDocument/definition" => {
                let (texto, donde) = donde_esta(&m, &abiertos);
                let palabra = palabra_en(&texto, donde);
                // `rows.3.label` y `mon.$screen.title` llevan a `rows` y a `mon`.
                let raiz = palabra.split('.').next().unwrap_or(&palabra);
                match nombres.iter().find(|(_, s)| s.local == palabra || s.local == raiz) {
                    Some((fichero, s)) => {
                        let suyo = abiertos.get(fichero).cloned().unwrap_or_else(|| std::fs::read_to_string(fichero).unwrap_or_default());
                        let (l, c) = (s.linea.saturating_sub(1), en_utf16(&suyo, s.linea, s.col));
                        contestar(id, json!({ "uri": uri_de(fichero), "range": { "start": { "line": l, "character": c }, "end": { "line": l, "character": c + s.local.chars().count() } } }));
                    }
                    None => contestar(id, Value::Null),
                }
            }
            // El esquema del fichero: lo que declara, para el índice del editor.
            "textDocument/documentSymbol" => {
                let Some(ruta) = ruta_de(m["params"]["textDocument"]["uri"].as_str().unwrap_or("")) else { continue };
                let texto = abiertos.get(&ruta).cloned().unwrap_or_default();
                let suyos: Vec<Value> = nombres
                    .iter()
                    .filter(|(f, _)| f.canonicalize().ok() == ruta.canonicalize().ok())
                    .map(|(_, s)| {
                        let (l, c) = (s.linea.saturating_sub(1), en_utf16(&texto, s.linea, s.col));
                        let sitio = json!({ "start": { "line": l, "character": c }, "end": { "line": l, "character": c + s.local.chars().count() } });
                        json!({ "name": s.local, "detail": s.clase, "kind": clase_lsp(&s.clase), "range": sitio, "selectionRange": sitio })
                    })
                    .collect();
                contestar(id, Value::Array(suyos));
            }
            "textDocument/hover" => {
                let (texto, donde) = donde_esta(&m, &abiertos);
                let palabra = palabra_en(&texto, donde);
                // Si es un nombre de la escena, lo primero es de qué es nombre.
                let suyo = nombres.iter().find(|(_, s)| s.local == palabra).map(|(f, s)| {
                    format!("`{}` — {} declared in `{}`, line {}.", s.local, s.clase, f.file_name().unwrap_or_default().to_string_lossy(), s.linea)
                });
                match suyo.into_iter().chain(ayuda_de(&palabra)).collect::<Vec<_>>() {
                    v if v.is_empty() => contestar(id, Value::Null),
                    v => contestar(id, json!({ "contents": { "kind": "markdown", "value": v.join("\n\n") } })),
                }
            }
            _ if id.is_some() => contestar(id, Value::Null),
            _ => {}
        }
    }
}

// ── el protocolo ────────────────────────────────────────────────

fn leer(e: &mut impl BufRead) -> Option<Value> {
    let mut largo = 0usize;
    loop {
        let mut linea = String::new();
        if e.read_line(&mut linea).ok()? == 0 {
            return None;
        }
        let linea = linea.trim_end();
        if linea.is_empty() {
            break;
        }
        if let Some(n) = linea.strip_prefix("Content-Length:") {
            largo = n.trim().parse().ok()?;
        }
    }
    let mut cuerpo = vec![0u8; largo];
    e.read_exact(&mut cuerpo).ok()?;
    serde_json::from_slice(&cuerpo).ok()
}

fn mandar(v: Value) {
    let cuerpo = v.to_string();
    let mut salida = std::io::stdout().lock();
    let _ = write!(salida, "Content-Length: {}\r\n\r\n{cuerpo}", cuerpo.len());
    let _ = salida.flush();
}

fn contestar(id: Option<Value>, resultado: Value) {
    let Some(id) = id else { return };
    mandar(json!({ "jsonrpc": "2.0", "id": id, "result": resultado }));
}

/// `file:///casa/escena.plm` → la ruta, con los %20 deshechos.
fn ruta_de(uri: &str) -> Option<PathBuf> {
    let resto = uri.strip_prefix("file://")?;
    let mut s = String::with_capacity(resto.len());
    let mut c = resto.chars();
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

fn uri_de(ruta: &Path) -> String {
    format!("file://{}", ruta.display())
}

// ── los fallos ──────────────────────────────────────────────────

/// Compila lo que se está escribiendo y reparte los fallos por fichero. Un fallo
/// puede caer en una biblioteca importada, así que se publican todos y se limpian
/// los ficheros que ya no tienen ninguno.
fn revisar_y_contar(ruta: &Path, abiertos: &HashMap<PathBuf, String>) -> Vec<(PathBuf, Simbolo)> {
    let copia: Vec<(PathBuf, String)> = abiertos.iter().map(|(r, t)| (r.canonicalize().unwrap_or_else(|_| r.clone()), t.clone())).collect();
    let (avisos, nombres) = crate::lenguaje::indice(&ruta.display().to_string(), copia);
    let mut por_fichero: HashMap<PathBuf, Vec<Value>> = abiertos.keys().map(|r| (r.clone(), Vec::new())).collect();
    por_fichero.entry(ruta.to_owned()).or_default();
    for a in avisos {
        let suyo = abiertos.keys().find(|r| r.canonicalize().ok() == a.fichero.canonicalize().ok()).cloned().unwrap_or(a.fichero);
        let texto = abiertos.get(&suyo).cloned().unwrap_or_else(|| std::fs::read_to_string(&suyo).unwrap_or_default());
        let (l, c) = (a.linea.saturating_sub(1), en_utf16(&texto, a.linea, a.col));
        por_fichero.entry(suyo).or_default().push(json!({
            "range": { "start": { "line": l, "character": c }, "end": { "line": l, "character": c + 1 } },
            "severity": 1,
            "source": "pleamar",
            "message": a.mensaje,
        }));
    }
    for (fichero, avisos) in por_fichero {
        publicar(&fichero, avisos);
    }
    nombres
}

/// La clase de símbolo que el editor entiende, para su icono.
fn clase_lsp(clase: &str) -> u8 {
    match clase {
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

fn publicar(ruta: &Path, avisos: Vec<Value>) {
    mandar(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": { "uri": uri_de(ruta), "diagnostics": avisos },
    }));
}

/// La columna que dice el compilador (caracteres, desde 1) en la que quiere el editor.
fn en_utf16(texto: &str, linea: usize, col: usize) -> usize {
    texto
        .lines()
        .nth(linea.saturating_sub(1))
        .map_or(col.saturating_sub(1), |l| l.chars().take(col.saturating_sub(1)).map(char::len_utf16).sum())
}

// ── dónde está el cursor ────────────────────────────────────────

/// El texto del fichero y en qué byte está el cursor.
fn donde_esta(m: &Value, abiertos: &HashMap<PathBuf, String>) -> (String, usize) {
    let Some(ruta) = ruta_de(m["params"]["textDocument"]["uri"].as_str().unwrap_or("")) else { return (String::new(), 0) };
    let texto = abiertos.get(&ruta).cloned().unwrap_or_else(|| std::fs::read_to_string(&ruta).unwrap_or_default());
    let (l, c) = (m["params"]["position"]["line"].as_u64().unwrap_or(0) as usize, m["params"]["position"]["character"].as_u64().unwrap_or(0) as usize);
    let mut sitio = 0;
    for (n, linea) in texto.split_inclusive('\n').enumerate() {
        if n == l {
            // El editor cuenta en UTF-16; aquí se necesitan bytes.
            let mut u16s = 0;
            for (b, ch) in linea.char_indices() {
                if u16s >= c {
                    return (texto.clone(), sitio + b);
                }
                u16s += ch.len_utf16();
            }
            return (texto.clone(), sitio + linea.trim_end_matches('\n').len());
        }
        sitio += linea.len();
    }
    (texto, sitio)
}

/// La palabra que hay debajo del cursor, o justo antes de él.
fn palabra_en(texto: &str, donde: usize) -> String {
    let es = |c: char| c.is_alphanumeric() || c == '_' || c == '.';
    let b = texto.as_bytes();
    let mut i = donde.min(b.len());
    while i > 0 && es(b[i - 1] as char) {
        i -= 1;
    }
    let mut j = donde.min(b.len());
    while j < b.len() && es(b[j] as char) {
        j += 1;
    }
    texto[i..j].to_owned()
}

/// En qué bloque está el cursor: la palabra que abrió la llave que sigue abierta.
/// `box { at: …` → «box», y con eso ya se sabe qué propiedades valen.
fn bloque_de(texto: &str, donde: usize) -> Option<String> {
    let hasta = &texto[..donde.min(texto.len())];
    let mut hondo = 0i32;
    let mut i = hasta.len();
    let b = hasta.as_bytes();
    while i > 0 {
        i -= 1;
        match b[i] {
            b'}' => hondo += 1,
            b'{' if hondo > 0 => hondo -= 1,
            b'{' => {
                // La cabeza del bloque: la primera palabra de lo que hay antes de la llave.
                let cabeza = hasta[..i].rsplit(['\n', ';', '}']).next().unwrap_or("").trim();
                return cabeza.split_whitespace().next().map(str::to_owned);
            }
            _ => {}
        }
    }
    None
}

// ── qué se puede escribir aquí ──────────────────────────────────

fn item(palabra: &str, clase: u8, detalle: &str) -> Value {
    json!({ "label": palabra, "kind": clase, "detail": detalle, "documentation": { "kind": "markdown", "value": ayuda_de(palabra).unwrap_or_default() } })
}

fn completado(texto: &str, donde: usize, nombres: &[(PathBuf, Simbolo)]) -> Vec<Value> {
    // Lo que la escena declara, cada cosa con su clase: es lo que más se escribe.
    let suyos = |clases: &[&str]| -> Vec<Value> {
        let mut vistos = std::collections::HashSet::new();
        nombres
            .iter()
            .filter(|(_, s)| clases.is_empty() || clases.contains(&s.clase.as_str()))
            .filter(|(_, s)| vistos.insert(s.local.clone()))
            .map(|(_, s)| json!({ "label": s.local, "kind": clase_lsp(&s.clase), "detail": s.clase }))
            .collect()
    };
    let antes = &texto[..donde.min(texto.len())];
    let linea = antes.rsplit('\n').next().unwrap_or("");
    let ultima = linea.split([';', '{', '(', ',']).next_back().unwrap_or("").trim_start();
    // `emit ` solo admite sucesos; `on ` espera lo que pasa, y luego una zona.
    if let Some(resto) = ultima.strip_prefix("emit ") {
        if !resto.contains(' ') {
            return suyos(&["event"]);
        }
    }
    if let Some(resto) = ultima.strip_prefix("on ") {
        return match resto.split_whitespace().count() {
            0 | 1 if !resto.ends_with(' ') => voz::DISPARADORES.iter().map(|x| item(x, 14, "what a rule waits for")).chain(suyos(&["event"])).collect(),
            _ => suyos(&["zone", "box", "ellipse", "arc", "line", "path", "row", "column", "input"]),
        };
    }
    // Tras `propiedad:` van sus valores, que casi siempre son de una lista cerrada;
    // si no, una expresión, y ahí entran los nombres de la escena.
    if let Some((izquierda, _)) = linea.rsplit_once(':') {
        let prop = izquierda.split([';', '{']).next_back().unwrap_or("").trim();
        if let Some(valores) = valores_de(prop) {
            return valores.iter().map(|v| item(v, 12, prop)).collect();
        }
        return suyos(&[]).into_iter().chain(voz::FUNCIONES.iter().map(|x| item(x, 3, "function"))).collect();
    }
    let dentro = bloque_de(texto, donde);
    match dentro.as_deref() {
        // Dentro de un elemento: sus propiedades y las que valen en cualquier forma.
        Some(p) if voz::PROPIEDADES.iter().any(|(n, _)| *n == p) => {
            let comunes: &[&str] = if ["ellipse", "box", "arc", "line", "path"].contains(&p) { voz::propiedades("shape") } else { &[] };
            let pasos: &[&str] = if p == "path" { voz::DE_CAMINO } else { &[] };
            voz::propiedades(p)
                .iter()
                .chain(comunes)
                .map(|x| item(x, 10, &format!("property of {p}")))
                .chain(pasos.iter().map(|x| item(x, 3, "step of a path")))
                .collect()
        }
        Some("row") | Some("column") => voz::propiedades("layout")
            .iter()
            .map(|x| item(x, 10, "property of a layout"))
            .chain(voz::SENTENCIAS.iter().map(|x| item(x, 14, "statement")))
            .collect(),
        // En la escena, o dentro de un grupo: cualquier sentencia, y los componentes
        // que haya declarados, que se copian escribiendo su nombre.
        _ => voz::SENTENCIAS.iter().map(|x| item(x, 14, "statement")).chain(suyos(&["component"])).collect(),
    }
}

/// Las palabras que valen como valor de una propiedad.
fn valores_de(prop: &str) -> Option<&'static [&'static str]> {
    Some(match prop {
        "anchor" => voz::ANCLAS_DE_SUPERFICIE,
        "level" => voz::NIVELES,
        "keyboard" => voz::TECLADOS,
        "align" => voz::ALINEADOS_DE_TEXTO,
        "cursor" => voz::CURSORES,
        _ => return None,
    })
}

// ── qué significa esta palabra ──────────────────────────────────

/// Lo que el editor enseña al pasar por encima. Sale de las listas del vocabulario,
/// más una línea escrita para cada sentencia (`voz::AYUDA`).
fn ayuda_de(palabra: &str) -> Option<String> {
    if palabra.is_empty() {
        return None;
    }
    let mut partes: Vec<String> = Vec::new();
    if let Some((_, texto)) = voz::AYUDA.iter().find(|(n, _)| *n == palabra) {
        partes.push((*texto).to_owned());
    }
    let donde: Vec<String> = voz::PROPIEDADES
        .iter()
        .filter(|(_, props)| props.contains(&palabra))
        .map(|(n, _)| format!("`{n}`"))
        .collect();
    if !donde.is_empty() {
        partes.push(format!("Property of {}.", donde.join(", ")));
    }
    for (lista, nombre) in [
        (voz::SENTENCIAS, "a statement"),
        (voz::FUNCIONES, "a function"),
        (voz::MUELLES, "a spring"),
        (voz::DISPARADORES, "what a rule waits for"),
        (voz::EFECTOS, "what a rule does"),
        (voz::TIPOS, "the type of a field"),
        (voz::DE_CAMINO, "a step of a path"),
        (voz::CURVAS, "a curve of a gesture"),
    ] {
        if lista.contains(&palabra) && !partes.iter().any(|p| p.contains(nombre)) {
            partes.push(format!("`{palabra}` is {nombre}."));
        }
    }
    if let Some((_, campos)) = voz::SERVICIOS.iter().find(|(n, _)| *n == palabra) {
        partes.push(format!("A service. It reports: {}.", campos.join(", ")));
    }
    (!partes.is_empty()).then(|| partes.join("\n\n"))
}

// ── el resaltado, salido del vocabulario ────────────────────────

/// `pleamar --resaltado vim|vscode|tree-sitter`. Lo escribe a la salida, y como
/// sale del vocabulario no puede quedarse atrás del lenguaje: `probar.sh` lo mira.
pub fn resaltado(cual: &str) -> i32 {
    let palabras = |l: &[&str]| l.join(" ");
    match cual {
        "vim" => {
            println!("\" Sintaxis de pleamar para Vim y Neovim. La hace `pleamar --resaltado vim`:");
            println!("\" no se escribe a mano, y así no se queda atrás del lenguaje.");
            println!("if exists(\"b:current_syntax\") | finish | endif");
            println!("syn keyword plmStatement {}", palabras(voz::SENTENCIAS));
            println!("syn keyword plmStatement scene library import language");
            println!("syn keyword plmKeyword in max while for after from until at by reach within rest inset right middle as via strict each all");
            println!("syn keyword plmFunction {}", palabras(voz::FUNCIONES));
            println!("syn keyword plmFunction {}", palabras(voz::DE_TEXTO));
            println!("syn keyword plmTrigger {}", palabras(voz::DISPARADORES));
            println!("syn keyword plmEffect {}", palabras(voz::EFECTOS));
            println!("syn keyword plmStep {}", palabras(voz::DE_CAMINO));
            println!("syn keyword plmConstant true false {} {} {}", palabras(voz::MUELLES), palabras(voz::CURVAS), palabras(voz::TIPOS));
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
            let reglas = json!([
                { "name": "comment.line.double-slash.plm", "match": "//.*$" },
                { "name": "string.quoted.double.plm", "begin": "\"", "end": "\"",
                  "patterns": [{ "name": "variable.other.plm", "match": "\\{[^}]*\\}" }] },
                { "name": "constant.other.colour.plm", "match": "#[0-9a-fA-F]{3,8}\\b" },
                { "name": "constant.numeric.plm", "match": "\\b\\d+(\\.\\d+)?(px|%|deg|ms|s)?\\b" },
                { "name": "entity.other.attribute-name.plm", "match": "\\b\\w+(?=\\s*:)" },
                { "name": "keyword.control.plm", "match": format!("\\b({}|scene|library|import|language)\\b", o(voz::SENTENCIAS)) },
                { "name": "keyword.other.plm", "match": "\\b(in|max|while|for|after|from|until|at|by|reach|within|rest|inset|right|middle|as|via|strict|each|all)\\b" },
                { "name": "support.function.plm", "match": format!("\\b({}|{})\\b", o(voz::FUNCIONES), o(voz::DE_TEXTO)) },
                { "name": "entity.name.tag.plm", "match": format!("\\b({})\\b", o(voz::DISPARADORES)) },
                { "name": "keyword.operator.plm", "match": format!("\\b({})\\b", o(voz::EFECTOS)) },
                { "name": "storage.type.plm", "match": format!("\\b({})\\b", o(voz::DE_CAMINO)) },
                { "name": "constant.language.plm", "match": format!("\\b(true|false|{}|{})\\b", o(voz::MUELLES), o(voz::CURVAS)) },
                { "name": "support.constant.spring.plm", "match": "~\\w+" },
            ]);
            println!("{:#}", json!({
                "$comment": "Lo hace `pleamar --resaltado vscode`: no se escribe a mano.",
                "name": "pleamar", "scopeName": "source.plm", "fileTypes": ["plm"], "patterns": reglas,
            }));
            0
        }
        otro => {
            eprintln!("'{otro}': the highlighters that can be written are 'vim' and 'vscode'");
            1
        }
    }
}
