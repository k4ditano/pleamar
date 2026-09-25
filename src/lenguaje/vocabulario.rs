//! Las palabras del lenguaje, en un solo sitio. **Estas listas son las que el
//! compilador consulta para aceptar o rechazar**, y las mismas que imprime
//! `pleamar --gramatica`: lo que aquí no esté, no vale; y lo que esté y nadie
//! atienda, revienta en las pruebas. Así la referencia escrita no se puede
//! desfasar sin que `./probar.sh` lo diga.

/// Con qué puede empezar una sentencia (además del nombre de un componente).
pub const SENTENCIAS: &[&str] = &[
    "surface", "permissions", "model", "service", "spring", "prop", "pose", "fact", "event", "text", "image", "figure", "measure", "let", "zone",
    "body", "ellipse", "box", "arc", "line", "path", "input", "clip", "group", "popup",
    "component", "children", "repeat", "for", "row", "column", "space", "between",
    "layer", "on", "every", "blink", "wave", "spin", "follow", "look", "gesture", "posture",
];

/// Lo que una biblioteca puede declarar.
pub const DE_BIBLIOTECA: &[&str] = &["let", "spring", "component", "permissions", "fact", "text", "model", "service", "event", "image", "figure", "prop", "pose", "gesture", "posture", "layer"];

/// Qué propiedades acepta cada elemento. `shape` son las comunes a todas las formas.
pub const PROPIEDADES: &[(&str, &[&str])] = &[
    ("surface", &["size", "anchor", "margin", "level", "reserve", "screens", "keyboard", "open", "kind", "title", "rate"]),
    ("permissions", &["run", "services"]),
    ("shape", &["rotate", "stroke", "color", "opacity", "blend", "active", "show", "cursor", "grow"]),
    ("ellipse", &["at", "radius", "scale"]),
    ("box", &["at", "from", "size", "corner"]),
    ("arc", &["at", "radius", "span", "width"]),
    ("line", &["from", "to", "width"]),
    ("path", &["at", "size"]),
    ("body", &["color", "gradient", "rim", "light", "shadow", "border", "glass", "opacity", "show"]),
    ("text", &["at", "anchor", "width", "size", "weight", "color", "opacity", "lines", "align", "line_height", "family", "measure", "show", "grow"]),
    ("image", &["at", "size", "opacity", "tint", "show", "grow"]),
    ("figure", &["at", "size", "scale", "rotate", "pivot", "color", "opacity", "blend", "stroke", "show", "grow"]),
    ("input", &["at", "width", "size", "weight", "color", "opacity", "family", "placeholder", "selection", "secret", "show"]),
    ("group", &["pivot", "rotate", "scale", "move", "opacity", "size", "show", "grow"]),
    ("popup", &["at", "size", "open"]),
    ("children", &["move"]),
    ("layout", &["at", "anchor", "gap", "padding", "align", "fill", "corner", "show", "opacity", "cursor", "view", "step", "content", "wrap", "size", "grow"]),
];

pub const FUNCIONES: &[&str] = &["min", "max", "abs", "floor", "ceil", "sin", "cos", "clamp", "smooth", "mix", "if", "vel"];
/// Dentro de un hueco de un texto.
pub const DE_TEXTO: &[&str] = &["upper", "lower"];
/// Lo que puede ir tras `on`. Cualquier otra palabra es el nombre de un suceso.
pub const DISPARADORES: &[&str] = &["press", "release", "scroll", "drag", "hold", "enter", "leave", "hover", "away", "idle", "key", "submit", "focus", "blur", "drop", "change", "still"];
/// Los efectos con palabra propia. Además: `prop: valor ~muelle` y `hecho = expr`.
pub const EFECTOS: &[&str] = &["toggle", "emit", "impulse", "play", "focus", "blur"];
pub const CURVAS: &[&str] = &["linear", "in_quad", "out_quad", "in_cubic", "out_cubic", "in_out_sine", "out_back"];
/// Lo que puede llevar un fotograma además de una curva.
pub const DE_FOTOGRAMA: &[&str] = &["hold", "emit"];
pub const CLASES: &[&str] = &["ambient", "reflex", "asked", "state"];
/// De un campo de un modelo. Además: un enumerado, `low | normal | critical`.
pub const TIPOS: &[&str] = &["text", "number", "bool", "image"];
/// De un hecho. Además: un enumerado.
pub const TIPOS_DE_HECHO: &[&str] = &["number", "bool"];
/// Una línea por palabra, para el servidor de lenguaje: lo que el editor enseña al
/// pasar por encima. `probar.sh` comprueba que no falte ninguna sentencia.
pub const AYUDA: &[(&str, &str)] = &[
    ("surface", "`surface { size: full, 44; anchor: top }` — a window this scene asks the system for. Several, with a name, share everything."),
    ("permissions", "`permissions { run: \"date\"; services: \"audio\", \"audio.*\" }` — what the logic may touch. Undeclared, nothing. Listening is not commanding."),
    ("model", "`model rows max 14 { label: text }` — a list of records the logic fills. Creates `rows.count`, `rows.total` and `rows.K.field`."),
    ("service", "`service clock as now { time: text }` — a system service by name. What it reports lands in facts and texts with no logic at all."),
    ("spring", "`spring bouncy = 170, 12` — a spring of your own: stiffness, damping. Built in: lively, calm, quick, slow, gentle, pose."),
    ("prop", "`prop orb.x = 360 ~lively` — an animated property. The render moves it; the logic never sees it."),
    ("pose", "`pose eyes = 14` — a property a gesture carries by the hand."),
    ("fact", "`fact open = false` — something that is true for a while. The logic and the rules set it. Can be typed: `fact mode: low | normal | critical`."),
    ("event", "`event confirmed` · `event view ->` — something that happens. With `->`, the logic hears it too."),
    ("text", "Declares a live text (`text title = \"…\"`) or paints one (`text title { size: 14 }`). In a painted one, `\"{a} · {b}\"` has holes."),
    ("image", "`image fox = icon \"firefox\", 48, 48` — an image by icon name, by file, or from a text that says which."),
    ("figure", "`figure hat = file \"hat.svg\"` — an svg as geometry: its layers are paths, by the `id` of each one."),
    ("measure", "`measure label` — creates `label.width` and `label.height`, filled by the text that carries `measure: label`."),
    ("let", "`let panel.x = orb.x + 62` — a name for an expression, or for a colour. It has to come before whoever uses it."),
    ("zone", "`zone box whole { at: …; size: … }` — a shape that is not painted: it only catches the mouse."),
    ("body", "`body { color: …; shadow: … }` — one silhouette out of several shapes, melted together, with its paint, rim, light and shadow."),
    ("ellipse", "`ellipse { at: 40, 40; radius: 12 }` — a circle, or an ellipse with `scale`."),
    ("box", "`box { from: 0, 0; size: 40, 20; corner: 8 }` — a rectangle, rounded if you want."),
    ("arc", "`arc { at: …; radius: 20; span: 2.4; width: 3 }` — an arc like `∩`, open as much as `span` says."),
    ("line", "`line { from: 0, 0; to: 30, 12; width: 2 }` — a line with round ends."),
    ("path", "`path { move 0, 0; line 20, 10; curve 40, 0 via 32, 10; close }` — a broken or curved line. Closed, it is filled."),
    ("input", "`input query { width: 260 }` — a one-line text field. What gets typed reaches the logic as `text:query`."),
    ("clip", "`clip [inset n] forma` — cuts everything that comes after, until the end of its group."),
    ("group", "`group { rotate: …; opacity: … }` — several things moved, turned or faded as one."),
    ("popup", "`popup menu { at: …; size: …; open: shown }` — a little window of its own that can go outside the surface."),
    ("component", "`component Row(r: record, tone: color = mint) { … }` — something to copy, with typed parameters and named holes."),
    ("children", "`children` · `children header` — where a copy's children go, inside a component."),
    ("repeat", "`repeat i in 1..10 { … }` — the same thing several times, with `$i` to build names."),
    ("for", "`for r in rows { … }` · `for r in rows from first { … }` — once per record of a model."),
    ("row", "`row { gap: 8; align: center }` — children side by side, with gap, padding, fill and scroll (`view:`)."),
    ("column", "`column { gap: 8 }` — children one under the other. Same properties as `row`."),
    ("space", "`space 12` — a gap of that size inside a layout."),
    ("between", "`between i { … }` — what goes between every two children of a layout, with its position."),
    ("layer", "`layer mouth { smile while happy }` — who wins when several want the same thing, and how it hands over."),
    ("on", "`on press hit { open = true }` — a rule: when something happens, this. Also `on some_event`."),
    ("every", "`every 2s { … }` · `every 2s..6s while awake { … }` — a rule on a clock."),
    ("blink", "`blink lid every 2.4s..6s for 170ms` — from 1 to 0 and back, now and then."),
    ("wave", "`wave breath = 3 at 1.7` — amplitude · sin(1.7 t)."),
    ("spin", "`spin angle by 0.9` — adds that much per second."),
    ("follow", "`follow on = if(active == i, 1, 0)` — a property that follows an expression, with its spring."),
    ("look", "`look eye.x, eye.y at pointer reach 8, 5 within 260` — eyes that follow something."),
    ("gesture", "`gesture cheer { … }` — a timeline of poses: the render plays it whole."),
    ("posture", "`posture tired { … }` — a still face: where the poses rest."),
    ("import", "`import \"comun/paleta.plm\"` — brings in a library: its colours, components and springs."),
    ("scene", "`scene Bar { … }` — everything a window shows, and what it reacts to."),
    ("library", "`library Palette { … }` — what several scenes share. With a `.luau` next to it, it is a plugin: its own frontier and its own permissions."),
    ("language", "`language 0.1` — the version of the language this file needs. It is the first line."),
];

/// Los servicios que se pueden pedir con `service`, y qué cuenta cada uno. La escena
/// elige qué campos quiere de los que hay; pedir uno que no está es un fallo al cargar.
/// Los que traen listas —`apps`, `tray`, `notifications`, `workspaces`— no salen aquí:
/// esos son un modelo, y los reparte la lógica.
pub const SERVICIOS: &[(&str, &[&str])] = &[
    ("clock", &["hour", "minute", "second", "day", "month", "year", "weekday", "time", "date"]),
    ("clock.seconds", &["hour", "minute", "second", "day", "month", "year", "weekday", "time", "date"]),
    ("audio", &["volume", "muted", "input", "input_muted"]),
    ("battery", &["present", "percent", "charging"]),
    ("brightness", &["present", "level"]),
    ("network", &["online", "kind", "name", "strength"]),
    ("media", &["playing", "title", "artist", "album", "player"]),
    ("window", &["title", "class", "monitor"]),
];

/// Los pasos de un camino: por dónde pasa. `curve … via …` es una Bézier cuadrática.
pub const DE_CAMINO: &[&str] = &["move", "line", "curve", "close"];

/// Lo que un modelo puede llevar dentro además de campos: otra lista de fichas.
pub const DE_MODELO: &[&str] = &["list"];
/// Lo que un componente puede pedir: `component Row(r: record, chosen: event, tone: color = mint)`.
pub const TIPOS_DE_PARAMETRO: &[&str] = &["number", "bool", "color", "text", "record", "event", "image", "gesture", "spring"];
pub const MUELLES: &[&str] = &["lively", "calm", "quick", "slow", "gentle", "pose"];
pub const UNIDADES: &[&str] = &["px", "%", "deg", "ms", "s"];
pub const CURSORES: &[&str] = &["default", "pointer", "text", "grab", "grabbing"];
pub const ANCLAS_DE_SUPERFICIE: &[&str] = &["top", "bottom", "left", "right", "top_left", "top_right", "bottom_left", "bottom_right", "center"];
pub const NIVELES: &[&str] = &["background", "bottom", "top", "overlay"];
/// Qué clase de ventana pide una superficie: pegada a un borde, o de las normales.
pub const CLASES_DE_SUPERFICIE: &[&str] = &["panel", "window", "lock"];
pub const TECLADOS: &[&str] = &["none", "on_demand", "exclusive"];
pub const ALINEADOS_DE_TEXTO: &[&str] = &["left", "center", "right"];
pub const ALINEADOS_DE_REPARTO: &[&str] = &["start", "center", "end"];

/// Las propiedades de un elemento. Reventar aquí es un fallo de quien programa, no de quien escribe la escena.
pub fn propiedades(de: &str) -> &'static [&'static str] {
    PROPIEDADES.iter().find(|(n, _)| *n == de).map(|(_, p)| *p).unwrap_or_else(|| panic!("the vocabulary does not know what properties '{de}' has"))
}

/// Todo, como texto: una línea por lista. Es lo que imprime `pleamar --gramatica`
/// y lo que la referencia lleva copiado, para que se puedan comparar.
pub fn como_texto() -> String {
    let mut s = format!("language: {}.{}\n", super::VERSION.0, super::VERSION.1);
    let mut linea = |nombre: &str, lista: &[&str]| s.push_str(&format!("{nombre}: {}\n", lista.join(" ")));
    linea("statements", SENTENCIAS);
    linea("library", DE_BIBLIOTECA);
    for (elemento, props) in PROPIEDADES {
        linea(&format!("properties.{elemento}"), props);
    }
    linea("functions", FUNCIONES);
    linea("text_functions", DE_TEXTO);
    linea("triggers", DISPARADORES);
    linea("effects", EFECTOS);
    linea("curves", CURVAS);
    linea("frame", DE_FOTOGRAMA);
    linea("classes", CLASES);
    linea("field_types", TIPOS);
    linea("fact_types", TIPOS_DE_HECHO);
    linea("model", DE_MODELO);
    linea("path", DE_CAMINO);
    // Las palabras que el editor sabe explicar: `probar.sh` exige que estén todas las sentencias.
    linea("documented", &AYUDA.iter().map(|(n, _)| *n).collect::<Vec<_>>());
    linea("services", &SERVICIOS.iter().map(|(n, _)| *n).collect::<Vec<_>>());
    for (servicio, campos) in SERVICIOS {
        linea(&format!("services.{servicio}"), campos);
    }
    linea("parameter_types", TIPOS_DE_PARAMETRO);
    linea("springs", MUELLES);
    linea("units", UNIDADES);
    linea("cursors", CURSORES);
    linea("surface.anchor", ANCLAS_DE_SUPERFICIE);
    linea("surface.level", NIVELES);
    linea("surface.kind", CLASES_DE_SUPERFICIE);
    linea("surface.keyboard", TECLADOS);
    linea("text.align", ALINEADOS_DE_TEXTO);
    linea("layout.align", ALINEADOS_DE_REPARTO);
    s
}
