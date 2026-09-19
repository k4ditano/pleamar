//! Marea: la bolita, sus ojos y la tarjeta de aviso que le nace del costado.
//! Todo lo que hay aquí son datos y decisiones; ni un píxel.

use crate::escena::*;
use crate::logica::{Contexto, Guion};
use crate::texto::{fuente, Lienzo};

const REPOSO_X: f32 = 360.0;
const ABIERTA_X: f32 = 140.0;
const ORBE_Y: f32 = 90.0;
const R: f32 = 28.0;
const HUECO: f32 = 34.0;
const PANEL_W: f32 = 406.0;
const PANEL_H: f32 = 190.0;
const ANCLA_Y: f32 = 70.0;

#[derive(Default)]
pub struct Marea {
    p: Option<Props>,
    abierta: bool,
    dormida: bool,
}

#[derive(Clone, Copy)]
struct Props {
    orbe_x: PropId,
    orbe_y: PropId,
    panel_w: PropId,
    panel_h: PropId,
    fusion: PropId,
    contenido: PropId,
    sueno: PropId,
    boton: PropId,
}

impl Guion for Marea {
    fn escena(&mut self) -> Escena {
        let mut e = Escena::default();
        let orbe_x = e.prop("orbe.x", REPOSO_X);
        let orbe_y = e.prop("orbe.y", ORBE_Y);
        let panel_w = e.prop("panel.ancho", 0.0);
        let panel_h = e.prop("panel.alto", 0.0);
        let fusion = e.prop("fusion", 0.0);
        let contenido = e.prop("contenido", 0.0);
        let sueno = e.prop("sueño", 0.0);
        let boton = e.prop("boton", 0.0);
        let parpado = e.prop("párpado", 1.0);
        let respira = e.prop("respira", 0.0);
        let mira_x = e.prop_con("mirada.x", 0.0, Muelle::OJOS);
        let mira_y = e.prop_con("mirada.y", 0.0, Muelle::OJOS);

        // Estirarse con la velocidad: una expresión, no código.
        let estira = ((orbe_x.vel().abs() - orbe_y.vel().abs()) / 2400.0).acotar(-0.22, 0.22);
        let orbe = Forma::Elipse {
            centro: (orbe_x.e(), orbe_y.e()),
            radio: R + respira,
            escala: (1.0 + estira.clone(), 1.0 / (1.0 + estira)),
        };

        let panel_x = orbe_x + (R + HUECO);
        let panel_y = orbe_y - panel_h * (ANCLA_Y / PANEL_H);
        let panel = Forma::Caja {
            centro: (panel_x.clone() + panel_w * 0.5, panel_y + panel_h * 0.5),
            mitad: (panel_w * 0.5, panel_h * 0.5),
            radio: 24.0.into(),
        };

        // El cuerpo: bolita y tarjeta fundidas por un cuello de agua.
        e.pintar(Instr::Grupo { sombra: Some(Sombra { desplazada: (0.0, 10.0), difusa: 30.0, alfa: 0.34 }) });
        e.pintar(Instr::Forma { forma: orbe.clone(), fusion: 0.0.into() });
        e.pintar(Instr::Forma { forma: panel.clone(), fusion: fusion * panel_w.e().suave(0.0, 40.0) });
        e.pintar(Instr::Relleno {
            color: color(0.082, 0.086, 0.086),
            alfa: 1.0.into(),
            filo: 0.05,
            luz: Some(Luz { cantidad: 0.03, desde_y: orbe_y - ANCLA_Y, alto: PANEL_H }),
        });

        // Lo de dentro de la tarjeta, anclado a donde quedará y recortado a
        // lo que lleve crecido.
        let origen_y = orbe_y - ANCLA_Y;
        let visible = contenido.e().acotar(0.0, 1.0);
        let caja_en = |cx: f32, cy: f32, mx: Expr, my: Expr| Forma::Caja {
            centro: (panel_x.clone() + cx, origen_y.clone() + cy),
            mitad: (mx, my),
            radio: 9.0.into(),
        };
        e.pintar(Instr::Recorte(Some((panel, 1.0))));
        e.pintar(Instr::Plano {
            forma: caja_en(110.0, 151.0, 84.0.into(), 23.0.into()),
            color: color(0.18, 0.184, 0.184),
            alfa: visible.clone(),
        });
        let ver = caja_en(296.0, 151.0, 84.0 + boton * 2.0, 23.0 + boton * 1.5);
        e.pintar(Instr::Plano {
            forma: ver.clone(),
            color: [mezcla(0.62, 0.74, boton), mezcla(0.84, 0.93, boton), mezcla(0.74, 0.84, boton)],
            alfa: visible.clone(),
        });
        e.pintar(Instr::Textura {
            destino: (panel_x.clone(), origen_y.clone(), PANEL_W.into(), PANEL_H.into()),
            uv: [0.0, 0.0, 1.0, 1.0],
            alfa: visible,
        });

        // Los ojos: dos píldoras recortadas a la cara.
        let abierto = (parpado * (1.0 - sueno)).max(0.14);
        let ojo = |lado: f32| Forma::Caja {
            centro: (orbe_x + mira_x + lado, orbe_y + mira_y + sueno * 3.0 - 1.0),
            mitad: (3.5.into(), 8.4 * abierto.clone()),
            radio: 3.5.into(),
        };
        e.pintar(Instr::Recorte(Some((orbe.clone(), 3.0))));
        for lado in [-8.6, 8.6] {
            e.pintar(Instr::Plano { forma: ojo(lado), color: color(0.96, 0.97, 0.96), alfa: 1.0.into() });
        }
        e.pintar(Instr::Recorte(None));

        e.comportamientos = vec![
            Comportamiento::Parpadeo { prop: parpado, cada: (2.4, 6.0), dura: 0.17 },
            Comportamiento::Onda { prop: respira, frecuencia: 1.7, amplitud: sueno * 1.3 },
            Comportamiento::Mirada {
                x: mira_x,
                y: mira_y,
                centro: (orbe_x.e(), orbe_y.e()),
                alcance: (5.0, 3.2),
                distancia: 140.0,
                // Sin ratón, mira a la tarjeta si la hay.
                reposo: (panel_w * (3.2 / PANEL_W), panel_w * (0.6 / PANEL_W)),
            },
        ];

        let rapida = |prop, a| Transicion { prop, a, muelle: Muelle::RAPIDO, retraso: Default::default() };
        let abierta = contenido.e();
        e.zonas = vec![
            Zona {
                id: "conjunto",
                forma: Forma::Caja {
                    centro: ((ABIERTA_X + (HUECO + PANEL_W) * 0.5).into(), (ORBE_Y - ANCLA_Y + PANEL_H * 0.5).into()),
                    mitad: (((R * 2.0 + HUECO + PANEL_W) * 0.5 + 18.0).into(), (PANEL_H * 0.5 + 18.0).into()),
                    radio: 24.0.into(),
                },
                activa: 1.0.into(),
                al_entrar: vec![],
                al_salir: vec![],
            },
            Zona { id: "orbe", forma: Forma::circulo((orbe_x.e(), orbe_y.e()), R + 8.0), activa: 1.0.into(), al_entrar: vec![], al_salir: vec![] },
            Zona {
                id: "descartar",
                forma: caja_en(110.0, 151.0, 84.0.into(), 23.0.into()),
                activa: abierta.clone(),
                al_entrar: vec![],
                al_salir: vec![],
            },
            // El realce del botón no pasa por la lógica: lo ejecuta el render.
            Zona { id: "ver", forma: ver, activa: abierta, al_entrar: vec![rapida(boton, 1.0)], al_salir: vec![rapida(boton, 0.0)] },
        ];

        e.atlas = Some(tarjeta());
        self.p = Some(Props { orbe_x, orbe_y, panel_w, panel_h, fusion, contenido, sueno, boton });
        e
    }

    fn evento(&mut self, e: Evento, c: &mut Contexto) {
        if !matches!(e, Evento::Alarma(_)) {
            c.alarma("dormir", 14_000);
        }
        match e {
            Evento::Entra("orbe") => {
                self.despertar(c);
                if !self.abierta {
                    c.alarma("abrir", 320);
                }
            }
            Evento::Sale("orbe") => c.cancelar("abrir"),
            Evento::Entra("conjunto") => c.cancelar("cerrar"),
            Evento::Sale("conjunto") if self.abierta => c.alarma("cerrar", 420),
            Evento::Pulsa("orbe") | Evento::Demo => {
                if self.abierta {
                    self.cerrar(c, matches!(e, Evento::Demo));
                } else {
                    self.abrir(c);
                }
            }
            Evento::Pulsa("ver") => self.cerrar(c, true),
            Evento::Pulsa("descartar") | Evento::Alarma("cerrar") => self.cerrar(c, false),
            Evento::Alarma("abrir") => self.abrir(c),
            Evento::Alarma("dormir") if !self.abierta && !self.dormida => {
                self.dormida = true;
                c.animar(self.p.unwrap().sueno, 1.0, Muelle::LENTO, 0);
            }
            _ => {}
        }
    }
}

impl Marea {
    fn abrir(&mut self, c: &mut Contexto) {
        if self.abierta {
            return;
        }
        self.abierta = true;
        c.cancelar("abrir");
        self.despertar(c);
        let p = self.p.unwrap();
        // Toda la coreografía, declarada de una vez y con sus retrasos. A
        // partir de aquí este hilo puede desaparecer: nadie le va a esperar.
        c.animar(p.fusion, 96.0, Muelle::RAPIDO, 0);
        c.animar(p.orbe_x, ABIERTA_X, Muelle::VIVO, 0);
        c.animar(p.panel_w, PANEL_W, Muelle::SERENO, 70);
        c.animar(p.panel_h, PANEL_H, Muelle::SERENO, 110);
        c.animar(p.contenido, 1.0, Muelle::SERENO, 300);
        c.animar(p.fusion, 0.0, Muelle::SERENO, 560);
        c.trabajar();
    }

    fn cerrar(&mut self, c: &mut Contexto, alegre: bool) {
        if !self.abierta {
            return;
        }
        self.abierta = false;
        c.cancelar("cerrar");
        let p = self.p.unwrap();
        if alegre {
            c.impulso(p.orbe_y, -620.0);
        }
        c.animar(p.boton, 0.0, Muelle::RAPIDO, 0);
        c.animar(p.contenido, 0.0, Muelle::RAPIDO, 0);
        c.animar(p.fusion, 96.0, Muelle::RAPIDO, 0);
        c.animar(p.panel_h, 0.0, Muelle::SERENO, 90);
        c.animar(p.panel_w, 0.0, Muelle::SERENO, 90);
        c.animar(p.orbe_x, REPOSO_X, Muelle::VIVO, 150);
        c.animar(p.fusion, 0.0, Muelle::SERENO, 700);
        c.trabajar();
    }

    fn despertar(&mut self, c: &mut Contexto) {
        if self.dormida {
            self.dormida = false;
            c.animar(self.p.unwrap().sueno, 0.0, Muelle::VIVO, 0);
        }
    }
}

fn tarjeta() -> Lienzo {
    let normal = fuente("Inter");
    let negrita = fuente("Inter:medium");
    let mut l = Lienzo::nuevo(PANEL_W as usize, PANEL_H as usize);
    let blanco = [0.96, 0.97, 0.96];
    let tinta = [0.07, 0.12, 0.10];

    l.escribir(&normal, "CALENDARIO", 12.5, 26.0, 38.0, blanco, 0.62, 0.7);
    let w = Lienzo::medir(&normal, "Ahora", 13.5, 0.0);
    l.escribir(&normal, "Ahora", 13.5, PANEL_W - 26.0 - w, 38.0, blanco, 0.55, 0.0);
    l.escribir(&negrita, "Reunión en 5 min", 20.0, 26.0, 78.0, blanco, 1.0, 0.0);
    l.escribir(&normal, "Revisión de diseño · 18:00", 14.5, 26.0, 104.0, blanco, 0.6, 0.0);
    let w = Lienzo::medir(&negrita, "Descartar", 15.0, 0.0);
    l.escribir(&negrita, "Descartar", 15.0, 110.0 - w / 2.0, 156.5, blanco, 0.95, 0.0);
    let w = Lienzo::medir(&negrita, "Ver evento", 15.0, 0.0);
    l.escribir(&negrita, "Ver evento", 15.0, 296.0 - w / 2.0, 156.5, tinta, 1.0, 0.0);
    l
}
