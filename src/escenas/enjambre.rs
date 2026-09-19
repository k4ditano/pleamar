//! Un banco de carga: muchas formas sueltas, todas moviéndose, para medir lo
//! que cuesta pintar cuando la escena deja de ser una bolita.

use crate::escena::*;
use crate::logica::{Contexto, Guion};

#[derive(Default)]
pub struct Enjambre;

impl Guion for Enjambre {
    fn escena(&mut self) -> Escena {
        let n: usize = std::env::var("PLEAMAR_N").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
        let mut e = Escena::default();
        let late = e.prop("late", 0.0);
        let columnas = ((n as f32 * 3.2).sqrt().ceil() as usize).max(1);
        let filas = n.div_ceil(columnas);
        let (paso_x, paso_y) = (700.0 / columnas as f32, 210.0 / filas as f32);
        let r = (paso_x.min(paso_y) * 0.42).min(14.0);
        for k in 0..n {
            let (cx, cy) = (10.0 + paso_x * ((k % columnas) as f32 + 0.5), 8.0 + paso_y * ((k / columnas) as f32 + 0.5));
            let fase = if k % 2 == 0 { 1.0 } else { -1.0 };
            let forma = if k % 3 == 0 {
                Forma::Caja { centro: (cx.into(), cy.into()), mitad: (r + late * fase, (r * 0.7).into()), radio: (r * 0.3).into() }
            } else {
                Forma::circulo((cx.into(), cy.into()), r + late * fase)
            };
            let t = k as f32 / n as f32;
            e.pintar(Instr::Plano { forma, color: color(0.35 + 0.5 * t, 0.85 - 0.3 * t, 0.75), alfa: 0.9.into() });
        }
        e.comportamientos = vec![Comportamiento::Onda { prop: late, frecuencia: 3.0, amplitud: 2.0.into() }];
        println!("enjambre · {n} formas");
        e
    }
    fn evento(&mut self, _: Evento, _: &mut Contexto) {}
}
