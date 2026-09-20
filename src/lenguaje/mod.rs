//! El lenguaje de escenas. Un fichero de texto entra y una `Escena` sale —la
//! misma que hasta ahora se escribía en Rust—, o un fallo que dice dónde y por
//! qué. Todo lo que se declara aquí lo ejecuta el render, solo.
//!
//! Tres pasos: `fichas` trocea, `arbol` agrupa en nodos y propiedades sin
//! saber qué significan, y `obra` les da sentido y comprueba los nombres.

mod arbol;
mod fichas;
mod obra;
pub mod vocabulario;

use crate::escena::Escena;
use std::path::{Path, PathBuf};

/// Una escena puede estar hecha de varios ficheros (`import`), y un fallo tiene
/// que decir en cuál. Para no cargar cada ficha con un nombre, el número de línea
/// lleva dentro el del fichero: línea 12 del tercero es 2 000 012.
pub(crate) const POR_FICHERO: usize = 1_000_000;

/// La versión del lenguaje que entiende este programa. El primer número cambia
/// cuando algo escrito deja de valer; el segundo, cuando se añade algo. Un fichero
/// puede decir cuál necesita (`language 0.1`) y enterarse al cargar, no a medias.
pub const VERSION: (u32, u32) = (0, 1);

#[derive(Debug)]
pub struct Fallo {
    pub linea: usize,
    pub col: usize,
    pub mensaje: String,
}

impl Fallo {
    pub fn en(linea: usize, col: usize, mensaje: impl Into<String>) -> Fallo {
        Fallo { linea, col, mensaje: mensaje.into() }
    }

    /// Con la línea del fichero y una flecha debajo, como lo enseña un compilador.
    fn con_fuente(&self, ficheros: &[(PathBuf, String)]) -> String {
        let (ruta, fuente) = ficheros.get(self.linea / POR_FICHERO).or(ficheros.first()).map_or((String::new(), ""), |(r, f)| {
            // Desde donde se está, si se puede: una ruta entera estorba más que ayuda.
            let corta = std::env::current_dir().ok().and_then(|aqui| r.strip_prefix(aqui).ok().map(Path::to_owned)).unwrap_or_else(|| r.clone());
            (corta.display().to_string(), f.as_str())
        });
        let n = self.linea % POR_FICHERO;
        let linea = fuente.lines().nth(n.saturating_sub(1)).unwrap_or("");
        let margen = format!("{n:>4} | ");
        format!("{ruta}:{n}:{}: {}\n{margen}{linea}\n{}^", self.col, self.mensaje, " ".repeat(margen.len() + self.col.saturating_sub(1)))
    }
}

/// «fichero:línea» de algo que se declaró, para decir dónde estaba lo que se pisa.
pub fn sitio(nombres: &[String], linea: usize) -> String {
    format!("{}:{}", nombres.get(linea / POR_FICHERO).map_or("", String::as_str), linea % POR_FICHERO)
}

/// Lo que se ha ido leyendo: cada fichero una vez, con su texto para enseñar los fallos.
#[derive(Default)]
struct Lectura {
    ficheros: Vec<(PathBuf, String)>,
    /// Los que se están leyendo ahora mismo, unos dentro de otros: para ver los círculos.
    abiertos: Vec<PathBuf>,
    /// Qué ficheros son bibliotecas `strict`, por su número.
    estrictos: Vec<usize>,
    /// Cada biblioteca: el número de su fichero, su nombre, y su lógica si tiene un `.luau` al lado.
    bibliotecas: Vec<obra::Biblioteca>,
    /// Lo que un editor tiene abierto y todavía no ha guardado: gana al disco.
    abiertos_de_fuera: Vec<(PathBuf, String)>,
}

impl Lectura {
    /// Trocea y agrupa un fichero, con su número metido en las líneas.
    fn abrir(&mut self, ruta: &Path) -> Result<Vec<arbol::Entrada>, Fallo> {
        let k = self.ficheros.len();
        let suyo = ruta.canonicalize().unwrap_or_else(|_| ruta.to_owned());
        let fuente = match self.abiertos_de_fuera.iter().find(|(r, _)| *r == suyo) {
            Some((_, t)) => t.clone(),
            None => std::fs::read_to_string(ruta).map_err(|e| Fallo::en(0, 0, format!("cannot read {}: {e}", ruta.display())))?,
        };
        self.ficheros.push((ruta.to_owned(), fuente));
        let aqui = |mut f: Fallo| { f.linea += k * POR_FICHERO; f };
        let mut fichas = fichas::trocear(&self.ficheros[k].1).map_err(aqui)?;
        for f in &mut fichas {
            f.linea += k * POR_FICHERO;
        }
        // Cada fichero dice qué versión necesita, también una biblioteca.
        version_pedida(arbol::arbol(&fichas).map_err(|f| if f.linea < POR_FICHERO { aqui(f) } else { f })?)
    }

    /// Lo que declaran los `import` de un fichero, en orden, y lo que queda de él.
    /// Una biblioteca importada dos veces —por dos caminos— se lee una.
    fn resolver(&mut self, ruta: &Path, entradas: Vec<arbol::Entrada>, traido: &mut Vec<arbol::Entrada>) -> Result<Vec<arbol::Entrada>, Fallo> {
        use fichas::F;
        let mut resto = Vec::new();
        for e in entradas {
            let arbol::Entrada::Nodo(n) = &e else { resto.push(e); continue };
            if !matches!(n.cabeza.first().map(|f| &f.f), Some(F::Id(p)) if p == "import") {
                resto.push(e);
                continue;
            }
            let (Some(F::Cadena(cual)), 2, None) = (n.cabeza.get(1).map(|f| &f.f), n.cabeza.len(), &n.cuerpo) else {
                return Err(Fallo::en(n.linea, n.col, "un import es `import \"ruta/de/la/biblioteca.plm\"`"));
            };
            // Las rutas son relativas al fichero que importa, no a desde dónde se lance.
            let destino = ruta.parent().unwrap_or(Path::new(".")).join(cual);
            let destino = destino.canonicalize().map_err(|e| Fallo::en(n.linea, n.cabeza[1].col, format!("cannot find '{cual}' (looking in {}): {e}", destino.display())))?;
            if self.abiertos.contains(&destino) {
                return Err(Fallo::en(n.linea, n.cabeza[1].col, format!("'{cual}' ends up importing itself: {}", self.abiertos.iter().chain([&destino]).map(|p| p.file_name().unwrap_or_default().to_string_lossy()).collect::<Vec<_>>().join(" → "))));
            }
            if self.ficheros.iter().any(|(r, _)| r == &destino) {
                continue;
            }
            self.abiertos.push(destino.clone());
            let numero = self.ficheros.len();
            let suyas = self.abrir(&destino)?;
            let suyas = self.resolver(&destino, suyas, traido)?;
            self.abiertos.pop();
            let [arbol::Entrada::Nodo(b)] = suyas.as_slice() else {
                return Err(Fallo::en(n.linea, n.cabeza[1].col, format!("'{cual}' is not a library: it has to be `library Name {{ … }}`, and nothing else")));
            };
            if !matches!(b.cabeza.first().map(|f| &f.f), Some(F::Id(p)) if p == "library") {
                return Err(Fallo::en(b.linea, b.col, "what you import is a library: `library Name { … }`. A scene is not imported"));
            }
            let Some(arbol::Entrada::Nodo(b)) = suyas.into_iter().next() else { unreachable!() };
            let F::Id(nombre_de_biblioteca) = &b.cabeza.get(1).map(|f| f.f.clone()).unwrap_or(F::Id(String::new())) else {
                return Err(Fallo::en(b.linea, b.col, "this library is missing its name: `library Name { … }`"));
            };
            let logica = destino.with_extension("luau");
            self.bibliotecas.push(obra::Biblioteca { fichero: numero, nombre: nombre_de_biblioteca.clone(), logica: logica.is_file().then_some(logica) });
            // `library Menu strict { … }`: sus componentes solo leen lo que piden.
            match b.cabeza.get(2).map(|f| &f.f) {
                None => {}
                Some(F::Id(p)) if p == "strict" && b.cabeza.len() == 3 => self.estrictos.push(numero),
                Some(_) => return Err(Fallo::en(b.linea, b.cabeza[2].col, "after a library name only `strict` can go")),
            }
            for d in b.cuerpo.unwrap_or_default() {
                // Una biblioteca declara; no pinta, ni reacciona, ni tiene frontera con la lógica.
                // `text now = "…"` declara; `text "hola" { … }` pinta, y eso no es cosa de una biblioteca.
                let vale = matches!(&d, arbol::Entrada::Nodo(x) if matches!(x.cabeza.first().map(|f| &f.f), Some(F::Id(p)) if vocabulario::DE_BIBLIOTECA.contains(&p.as_str())
                    && (!["text", "image"].contains(&p.as_str()) || matches!(x.cabeza.get(2).map(|f| &f.f), Some(F::Sim("="))))));
                if !vale {
                    let (l, c) = match &d { arbol::Entrada::Nodo(x) => (x.linea, x.col), arbol::Entrada::Prop { linea, col, .. } => (*linea, *col) };
                    return Err(Fallo::en(l, c, "a library only declares: `let`, `spring`, `component`, its frontier (`fact`, `text`, `model`, `event`, `image x = …`, `permissions`) and what it moves inside (`prop`, `pose`, `gesture`, `posture`, `layer`). What gets painted, and loose rules, belong to the scene"));
                }
                traido.push(d);
            }
        }
        Ok(resto)
    }
}

/// `language 0.1`, si está, es lo primero del fichero. Se comprueba y se quita.
fn version_pedida(mut entradas: Vec<arbol::Entrada>) -> Result<Vec<arbol::Entrada>, Fallo> {
    use fichas::F;
    let Some(arbol::Entrada::Nodo(n)) = entradas.first() else { return Ok(entradas) };
    if !matches!(n.cabeza.first().map(|f| &f.f), Some(F::Id(p)) if p == "language") {
        return Ok(entradas);
    }
    let (Some(F::Num(v)), 2, None) = (n.cabeza.get(1).map(|f| &f.f), n.cabeza.len(), &n.cuerpo) else {
        return Err(Fallo::en(n.linea, n.col, format!("the version is asked for like this: `language {}.{}`", VERSION.0, VERSION.1)));
    };
    // `0.1` llega como un número: la parte entera y el primer decimal.
    let pedida = (v.trunc() as u32, ((v.fract() * 10.0).round()) as u32);
    if pedida.0 != VERSION.0 || pedida.1 > VERSION.1 {
        return Err(Fallo::en(n.linea, n.cabeza[1].col, format!("this file asks for language {}.{}, and this pleamar understands {}.{}", pedida.0, pedida.1, VERSION.0, VERSION.1)));
    }
    entradas.remove(0);
    Ok(entradas)
}

/// Lee una escena de su fichero, con lo que importe. Devuelve la escena y todos los
/// ficheros de los que está hecha —para vigilarlos—, o los fallos ya con su fichero,
/// su línea y su flecha.
pub fn leer_fichero(ruta: &str) -> Result<(Escena, Vec<PathBuf>), String> {
    leer_con(ruta, Vec::new()).map_err(|(fallos, ficheros)| {
        let n = fallos.len();
        let texto: Vec<String> = fallos.iter().map(|f| f.con_fuente(&ficheros)).collect();
        format!("{}\n{}", texto.join("\n\n"), if n == 1 { "one error".to_owned() } else { format!("{n} errors") })
    })
}

/// Un aviso para quien escribe, tal cual: en qué fichero, dónde, y qué pasa.
/// Es lo que el servidor de lenguaje le manda al editor mientras se teclea.
pub struct Aviso {
    pub fichero: PathBuf,
    pub linea: usize,
    pub col: usize,
    pub mensaje: String,
}

/// Compila sin pintar nada y devuelve lo que esté mal. `abiertos` son los ficheros
/// que un editor tiene a medio escribir: lo que diga ahí gana a lo que haya en disco.
pub fn revisar(ruta: &str, abiertos: Vec<(PathBuf, String)>) -> Vec<Aviso> {
    match leer_con(ruta, abiertos) {
        Ok(_) => Vec::new(),
        Err((fallos, ficheros)) => fallos
            .into_iter()
            .map(|f| Aviso {
                fichero: ficheros.get(f.linea / POR_FICHERO).map_or_else(|| PathBuf::from(ruta), |(r, _)| r.clone()),
                linea: f.linea % POR_FICHERO,
                col: f.col,
                mensaje: f.mensaje,
            })
            .collect(),
    }
}

fn leer_con(ruta: &str, abiertos: Vec<(PathBuf, String)>) -> Result<(Escena, Vec<PathBuf>), (Vec<Fallo>, Vec<(PathBuf, String)>)> {
    let mut l = Lectura { abiertos_de_fuera: abiertos, ..Default::default() };
    let principal = Path::new(ruta).canonicalize().unwrap_or_else(|_| PathBuf::from(ruta));
    let levantada = (|| {
        l.abiertos.push(principal.clone());
        let entradas = l.abrir(Path::new(ruta)).map_err(|f| vec![f])?;
        l.ficheros[0].0 = principal.clone();
        let mut traido = Vec::new();
        let mut resto = l.resolver(&principal, entradas, &mut traido).map_err(|f| vec![f])?;
        // Lo importado va delante de lo de la escena, como si estuviera escrito ahí.
        if let [arbol::Entrada::Nodo(escena)] = resto.as_mut_slice() {
            if let Some(cuerpo) = escena.cuerpo.as_mut() {
                traido.append(cuerpo);
                *cuerpo = traido;
            }
        }
        let nombres: Vec<String> = l.ficheros.iter().map(|(r, _)| r.file_name().unwrap_or_default().to_string_lossy().into_owned()).collect();
        let carpetas: Vec<PathBuf> = l.ficheros.iter().map(|(r, _)| r.parent().map_or_else(PathBuf::new, Path::to_owned)).collect();
        obra::levantar(&resto, &nombres, &carpetas, &l.estrictos, &l.bibliotecas)
    })();
    // Al enseñar un fallo, el fichero principal con la ruta que dio quien lo abrió.
    if let Some(f) = l.ficheros.first_mut() {
        f.0 = PathBuf::from(ruta);
    }
    match levantada {
        Ok(e) => Ok((e, l.ficheros.into_iter().map(|(r, _)| r).collect())),
        Err(fallos) => Err((fallos, l.ficheros)),
    }
}

/// «¿Querías decir…?»: el nombre conocido que más se parece, si se parece bastante.
pub fn parecido<'a>(a: &str, conocidos: impl Iterator<Item = &'a String>) -> Option<&'a String> {
    let distancia = |x: &str, y: &str| {
        let (x, y): (Vec<char>, Vec<char>) = (x.chars().collect(), y.chars().collect());
        let mut fila: Vec<usize> = (0..=y.len()).collect();
        for i in 1..=x.len() {
            let mut antes = fila[0];
            fila[0] = i;
            for j in 1..=y.len() {
                let guardado = fila[j];
                fila[j] = (antes + (x[i - 1] != y[j - 1]) as usize).min(fila[j] + 1).min(fila[j - 1] + 1);
                antes = guardado;
            }
        }
        fila[y.len()]
    };
    conocidos.map(|c| (distancia(a, c), c)).filter(|(d, c)| *d <= 1 + c.len() / 4).min_by_key(|(d, _)| *d).map(|(_, c)| c)
}
