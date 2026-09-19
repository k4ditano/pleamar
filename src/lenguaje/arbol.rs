//! De las fichas a un árbol sin significado todavía: nodos con su cabecera y
//! su bloque, y propiedades `nombre: valor`. Qué quiere decir cada cosa lo
//! decide `obra.rs`; así la gramática cabe en una página.

use super::fichas::{Ficha, F};
use super::Fallo;

#[derive(Debug)]
pub struct Nodo {
    /// Lo que va antes de la llave: `layer card ~calm`, `on hover orb for 320ms`.
    pub cabeza: Vec<Ficha>,
    pub cuerpo: Option<Vec<Entrada>>,
    pub linea: usize,
    pub col: usize,
}

#[derive(Debug)]
pub enum Entrada {
    Prop { nombre: String, valor: Vec<Ficha>, linea: usize, col: usize },
    Nodo(Nodo),
}

pub fn arbol(fichas: &[Ficha]) -> Result<Vec<Entrada>, Fallo> {
    let mut i = 0;
    let entradas = bloque(fichas, &mut i, false)?;
    Ok(entradas)
}

fn bloque(f: &[Ficha], i: &mut usize, dentro: bool) -> Result<Vec<Entrada>, Fallo> {
    let mut entradas = Vec::new();
    loop {
        while *i < f.len() && matches!(f[*i].f, F::Linea | F::Sim(";")) {
            *i += 1;
        }
        let Some(primera) = f.get(*i) else {
            if dentro {
                let u = f.last().unwrap();
                return Err(Fallo::en(u.linea, u.col, "falta una «}»: algún bloque se quedó abierto"));
            }
            return Ok(entradas);
        };
        if primera.f == F::Sim("}") {
            if !dentro {
                return Err(Fallo::en(primera.linea, primera.col, "esta «}» no cierra nada"));
            }
            *i += 1;
            return Ok(entradas);
        }
        // `nombre:` abre una propiedad; cualquier otra cosa, un nodo.
        if let (F::Id(nombre), Some(F::Sim(":"))) = (&primera.f, f.get(*i + 1).map(|x| &x.f)) {
            let (linea, col) = (primera.linea, primera.col);
            *i += 2;
            let desde = *i;
            while *i < f.len() && !matches!(f[*i].f, F::Linea | F::Sim(";") | F::Sim("}")) {
                *i += 1;
            }
            if desde == *i {
                return Err(Fallo::en(linea, col, format!("«{nombre}:» se quedó sin valor")));
            }
            entradas.push(Entrada::Prop { nombre: nombre.clone(), valor: f[desde..*i].to_vec(), linea, col });
            continue;
        }
        let (linea, col) = (primera.linea, primera.col);
        let desde = *i;
        while *i < f.len() && !matches!(f[*i].f, F::Linea | F::Sim(";") | F::Sim("{") | F::Sim("}")) {
            *i += 1;
        }
        let cabeza = f[desde..*i].to_vec();
        let cuerpo = if f.get(*i).is_some_and(|x| x.f == F::Sim("{")) {
            *i += 1;
            Some(bloque(f, i, true)?)
        } else {
            None
        };
        entradas.push(Entrada::Nodo(Nodo { cabeza, cuerpo, linea, col }));
    }
}
