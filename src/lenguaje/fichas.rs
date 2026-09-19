//! Del texto a las fichas: números con su unidad, colores, nombres con puntos.

use super::Fallo;

#[derive(Clone, Debug, PartialEq)]
pub enum F {
    Id(String),
    Num(f32),
    /// Una duración, ya en segundos: `320ms`, `14s`.
    Dur(f32),
    Color([f32; 3]),
    Cadena(String),
    Sim(&'static str),
    Linea,
}

#[derive(Clone, Debug)]
pub struct Ficha {
    pub f: F,
    pub linea: usize,
    pub col: usize,
}

const SIMBOLOS: [&str; 22] = ["->", "..", "<=", ">=", "==", "!=", "{", "}", "(", ")", ",", ":", ";", "=", "~", "+", "-", "*", "/", "<", ">", "%"];

pub fn trocear(fuente: &str) -> Result<Vec<Ficha>, Fallo> {
    let mut fichas = Vec::new();
    // Dentro de un paréntesis un salto de línea no acaba nada.
    let mut parentesis = 0usize;
    for (n, linea) in fuente.lines().enumerate() {
        let c: Vec<char> = linea.chars().collect();
        let mut i = 0;
        while i < c.len() {
            let (ch, col) = (c[i], i + 1);
            let mut poner = |f: F| fichas.push(Ficha { f, linea: n + 1, col });
            if ch.is_whitespace() {
                i += 1;
            } else if ch == '/' && c.get(i + 1) == Some(&'/') {
                break;
            } else if ch == '"' {
                let fin = c[i + 1..].iter().position(|x| *x == '"').ok_or_else(|| Fallo::en(n + 1, col, "esta cadena no se cierra: falta la comilla"))?;
                poner(F::Cadena(c[i + 1..i + 1 + fin].iter().collect()));
                i += fin + 2;
            } else if ch == '#' {
                let hex: String = c[i + 1..].iter().take_while(|x| x.is_ascii_hexdigit()).collect();
                // #151616, o el atajo de tres cifras: #fff es #ffffff.
                let largo = hex.len();
                let hex: String = match largo {
                    6 => hex,
                    3 => hex.chars().flat_map(|c| [c, c]).collect(),
                    _ => return Err(Fallo::en(n + 1, col, "un color son seis cifras hexadecimales (#151616) o tres (#fff)")),
                };
                let v = |k: usize| u8::from_str_radix(&hex[k..k + 2], 16).unwrap() as f32 / 255.0;
                poner(F::Color([v(0), v(2), v(4)]));
                i += largo + 1;
            } else if ch.is_ascii_digit() || (ch == '.' && c.get(i + 1).is_some_and(|x| x.is_ascii_digit())) {
                let mut j = i;
                while j < c.len() && (c[j].is_ascii_digit() || (c[j] == '.' && c.get(j + 1) != Some(&'.'))) {
                    j += 1;
                }
                let numero: f32 = c[i..j].iter().collect::<String>().parse().map_err(|_| Fallo::en(n + 1, col, "este número no se entiende"))?;
                let unidad: String = c[j..].iter().take_while(|x| x.is_ascii_alphabetic() || **x == '%').collect();
                let f = match unidad.as_str() {
                    "" | "px" => F::Num(numero),
                    "%" => F::Num(numero / 100.0),
                    "deg" => F::Num(numero.to_radians()),
                    "ms" => F::Dur(numero / 1000.0),
                    "s" => F::Dur(numero),
                    otra => return Err(Fallo::en(n + 1, j + 1, format!("no conozco la unidad «{otra}»: valen px, %, deg, ms y s"))),
                };
                poner(f);
                i = j + unidad.chars().count();
            } else if ch.is_alphabetic() || ch == '_' {
                let mut j = i;
                while j < c.len() && (c[j].is_alphanumeric() || c[j] == '_' || (c[j] == '.' && c.get(j + 1).is_some_and(|x| x.is_alphabetic() || *x == '_'))) {
                    j += 1;
                }
                poner(F::Id(c[i..j].iter().collect()));
                i = j;
            } else {
                let resto: String = c[i..].iter().take(2).collect();
                let s = SIMBOLOS.iter().find(|s| resto.starts_with(**s)).ok_or_else(|| Fallo::en(n + 1, col, format!("no sé qué hacer con «{ch}»")))?;
                match *s {
                    "(" => parentesis += 1,
                    ")" => parentesis = parentesis.saturating_sub(1),
                    _ => {}
                }
                poner(F::Sim(s));
                i += s.len();
            }
        }
        if parentesis == 0 {
            fichas.push(Ficha { f: F::Linea, linea: n + 1, col: c.len() + 1 });
        }
    }
    Ok(fichas)
}
