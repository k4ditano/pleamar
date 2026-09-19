//! Las palabras del lenguaje, en un solo sitio. **Estas listas son las que el
//! compilador consulta para aceptar o rechazar**, y las mismas que imprime
//! `pleamar --gramatica`: lo que aquí no esté, no vale; y lo que esté y nadie
//! atienda, revienta en las pruebas. Así la referencia escrita no se puede
//! desfasar sin que `./probar.sh` lo diga.

/// Con qué puede empezar una sentencia (además del nombre de un componente).
pub const SENTENCIAS: &[&str] = &[
    "surface", "permissions", "model", "spring", "prop", "pose", "fact", "event", "text", "image", "measure", "let", "zone",
    "body", "ellipse", "box", "arc", "line", "input", "clip", "group", "popup",
    "component", "children", "repeat", "for", "row", "column", "space", "between",
    "layer", "on", "every", "blink", "wave", "spin", "follow", "look", "gesture", "posture",
];

/// Lo que una biblioteca puede declarar.
pub const DE_BIBLIOTECA: &[&str] = &["let", "spring", "component"];

/// Qué propiedades acepta cada elemento. `shape` son las comunes a todas las formas.
pub const PROPIEDADES: &[(&str, &[&str])] = &[
    ("surface", &["size", "anchor", "margin", "level", "reserve", "screens", "keyboard"]),
    ("permissions", &["run", "services"]),
    ("shape", &["rotate", "stroke", "color", "opacity", "blend", "active", "show", "cursor"]),
    ("ellipse", &["at", "radius", "scale"]),
    ("box", &["at", "from", "size", "corner"]),
    ("arc", &["at", "radius", "span", "width"]),
    ("line", &["from", "to", "width"]),
    ("body", &["color", "gradient", "rim", "light", "shadow", "border", "opacity", "show"]),
    ("text", &["at", "anchor", "width", "size", "weight", "color", "opacity", "lines", "align", "line_height", "family", "measure", "show"]),
    ("image", &["at", "size", "opacity", "tint", "show"]),
    ("input", &["at", "width", "size", "weight", "color", "opacity", "family", "placeholder", "selection", "show"]),
    ("group", &["pivot", "rotate", "scale", "move", "opacity", "size", "show"]),
    ("popup", &["at", "size", "open"]),
    ("children", &["move"]),
    ("layout", &["at", "anchor", "gap", "padding", "align", "fill", "corner", "show", "opacity", "cursor"]),
];

pub const FUNCIONES: &[&str] = &["min", "max", "abs", "clamp", "smooth", "mix", "if", "vel"];
/// Dentro de un hueco de un texto.
pub const DE_TEXTO: &[&str] = &["upper", "lower"];
/// Lo que puede ir tras `on`. Cualquier otra palabra es el nombre de un suceso.
pub const DISPARADORES: &[&str] = &["press", "release", "scroll", "drag", "hold", "enter", "leave", "hover", "away", "idle", "key", "submit", "focus", "blur", "drop"];
/// Los efectos con palabra propia. Además: `prop: valor ~muelle` y `hecho = expr`.
pub const EFECTOS: &[&str] = &["toggle", "emit", "impulse", "play", "focus", "blur"];
pub const CURVAS: &[&str] = &["linear", "in_quad", "out_quad", "in_cubic", "out_cubic", "in_out_sine", "out_back"];
/// Lo que puede llevar un fotograma además de una curva.
pub const DE_FOTOGRAMA: &[&str] = &["hold", "emit"];
pub const CLASES: &[&str] = &["ambient", "reflex", "asked", "state"];
pub const TIPOS: &[&str] = &["text", "number", "bool"];
/// Lo que un componente puede pedir: `component Row(r: record, chosen: event, tone: color = mint)`.
pub const TIPOS_DE_PARAMETRO: &[&str] = &["number", "bool", "color", "text", "record", "event", "image", "gesture", "spring"];
pub const MUELLES: &[&str] = &["lively", "calm", "quick", "slow", "eyes", "pose"];
pub const UNIDADES: &[&str] = &["px", "%", "deg", "ms", "s"];
pub const CURSORES: &[&str] = &["default", "pointer", "text", "grab", "grabbing"];
pub const ANCLAS_DE_SUPERFICIE: &[&str] = &["top", "bottom", "left", "right", "top_left", "top_right", "bottom_left", "bottom_right", "center"];
pub const NIVELES: &[&str] = &["background", "bottom", "top", "overlay"];
pub const TECLADOS: &[&str] = &["none", "on_demand", "exclusive"];
pub const ALINEADOS_DE_TEXTO: &[&str] = &["left", "center", "right"];
pub const ALINEADOS_DE_REPARTO: &[&str] = &["start", "center", "end"];

/// Las propiedades de un elemento. Reventar aquí es un fallo de quien programa, no de quien escribe la escena.
pub fn propiedades(de: &str) -> &'static [&'static str] {
    PROPIEDADES.iter().find(|(n, _)| *n == de).map(|(_, p)| *p).unwrap_or_else(|| panic!("el vocabulario no sabe qué propiedades tiene «{de}»"))
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
    linea("parameter_types", TIPOS_DE_PARAMETRO);
    linea("springs", MUELLES);
    linea("units", UNIDADES);
    linea("cursors", CURSORES);
    linea("surface.anchor", ANCLAS_DE_SUPERFICIE);
    linea("surface.level", NIVELES);
    linea("surface.keyboard", TECLADOS);
    linea("text.align", ALINEADOS_DE_TEXTO);
    linea("layout.align", ALINEADOS_DE_REPARTO);
    s
}
