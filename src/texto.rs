//! Texto pintado en CPU a un atlas premultiplicado. Es el papel que tendría
//! «el contenido de un plugin»: una textura que el render coloca y recorta,
//! sin ejecutar nada de quien la hizo.

use fontdue::{Font, FontSettings};
use std::process::Command;

pub use crate::escena::Lienzo;

pub fn fuente(patron: &str) -> Font {
    let ruta = Command::new("fc-match")
        .args(["-f", "%{file}", patron])
        .output()
        .ok()
        .and_then(|s| String::from_utf8(s.stdout).ok())
        .unwrap_or_default();
    let datos = std::fs::read(ruta.trim())
        .or_else(|_| std::fs::read("/usr/share/fonts/noto/NotoSans-Regular.ttf"))
        .expect("no hay ninguna fuente que leer");
    Font::from_bytes(datos, FontSettings::default()).expect("fuente ilegible")
}

impl Lienzo {
    pub fn nuevo(ancho: usize, alto: usize) -> Lienzo {
        Lienzo { ancho, alto, rgba: vec![0; ancho * alto * 4] }
    }

    pub fn medir(f: &Font, s: &str, px: f32, tracking: f32) -> f32 {
        s.chars().map(|ch| f.metrics(ch, px).advance_width + tracking).sum()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn escribir(&mut self, f: &Font, s: &str, px: f32, x: f32, base: f32, color: [f32; 3], alfa: f32, tracking: f32) {
        let mut pluma = x;
        for ch in s.chars() {
            let (m, cobertura) = f.rasterize(ch, px);
            let x0 = (pluma + m.xmin as f32).round() as i32;
            let y0 = (base - m.ymin as f32 - m.height as f32).round() as i32;
            for fila in 0..m.height {
                for col in 0..m.width {
                    let (px_x, px_y) = (x0 + col as i32, y0 + fila as i32);
                    if px_x < 0 || px_y < 0 || px_x >= self.ancho as i32 || px_y >= self.alto as i32 {
                        continue;
                    }
                    let a = cobertura[fila * m.width + col] as f32 / 255.0 * alfa;
                    let i = (px_y as usize * self.ancho + px_x as usize) * 4;
                    for k in 0..3 {
                        let abajo = self.rgba[i + k] as f32 / 255.0;
                        self.rgba[i + k] = ((color[k] * a + abajo * (1.0 - a)) * 255.0) as u8;
                    }
                    let abajo = self.rgba[i + 3] as f32 / 255.0;
                    self.rgba[i + 3] = ((a + abajo * (1.0 - a)) * 255.0) as u8;
                }
            }
            pluma += m.advance_width + tracking;
        }
    }
}
