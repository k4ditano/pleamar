//! El lenguaje de escenas. Un fichero de texto entra y una `Escena` sale —la
//! misma que hasta ahora se escribía en Rust—, o un fallo que dice dónde y por
//! qué. Todo lo que se declara aquí lo ejecuta el render, solo.
//!
//! Tres pasos: `fichas` trocea, `arbol` agrupa en nodos y propiedades sin
//! saber qué significan, y `obra` les da sentido y comprueba los nombres.

mod arbol;
mod fichas;
mod obra;

use crate::escena::Escena;

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
    pub fn con_fuente(&self, ruta: &str, fuente: &str) -> String {
        let linea = fuente.lines().nth(self.linea.saturating_sub(1)).unwrap_or("");
        let margen = format!("{:>4} | ", self.linea);
        format!("{ruta}:{}:{}: {}\n{margen}{linea}\n{}^", self.linea, self.col, self.mensaje, " ".repeat(margen.len() + self.col.saturating_sub(1)))
    }
}

/// Devuelve la escena, o todos los fallos que se hayan podido encontrar.
pub fn leer(fuente: &str) -> Result<Escena, Vec<Fallo>> {
    let fichas = fichas::trocear(fuente).map_err(|f| vec![f])?;
    let arbol = arbol::arbol(&fichas).map_err(|f| vec![f])?;
    obra::levantar(&arbol)
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
