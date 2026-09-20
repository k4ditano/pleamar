//! La cara de Marea, sola, para probar lo que pidió su `ExpressionController`:
//! una capa `forma` con reclamaciones por prioridad, gestos por fotogramas con
//! clase, y hechos y sucesos como única frontera con la lógica.
//!
//! El guion hace lo que más miedo le da al código de verdad: buscar y
//! confirmar MIENTRAS graba. El disco rojo tiene que aguantar, y al dejar de
//! grabar tiene que salir la lupa sola, sin que nadie la vuelva a pedir.

use crate::escena::*;
use crate::logica::{Contexto, Guion};

const CX: f32 = 360.0;
const CY: f32 = 108.0;
/// La bolita de verdad mide 56 px; aquí vez y media, para que se vea de lejos.
const S: f32 = 1.5;
const R: f32 = 28.0 * S;

#[derive(Default)]
pub struct Cara {
    paso: usize,
}

const GUION: &[(u64, &str)] = &[
    (1000, "grabando = sí"),
    (1500, "buscando = sí            · la lupa se reclama, pero rec está por encima"),
    (1500, "suceso confirmado + happy · contenta se reclama, y tampoco se ve"),
    (1800, "grabando = no            · nadie pide la lupa otra vez: tiene que salir sola"),
    (1500, "suceso confirmado        · contenta 620 ms, y de vuelta a la lupa"),
    (1500, "buscando = no            · ojos"),
    (1200, "suceso aviso_urgente     · 700 ms"),
    (1500, "trabajando = sí          · una postura: se repite sola"),
    (1500, "gesto asentir (reflejo)  · corta a la postura, que es de menos clase"),
    (1200, "gesto happy (pedido)"),
    (250, "gesto asentir (reflejo)  · rechazado: hay algo pedido puesto"),
    (1800, "trabajando = no"),
];

impl Guion for Cara {
    fn escena(&mut self) -> Escena {
        let mut e = Escena::default();
        // Banco de ensayo: mide lo suyo. `--pantalla` dice en cuál se abre.
        e.superficies = vec![Superficie { ancho: 720, alto: 260, margen: [40, 0, 0, 0], ..Default::default() }];

        // ── la frontera ──────────────────────────────────────────
        let grabando = e.hecho("grabando", 0.0);
        let buscando = e.hecho("buscando", 0.0);
        let trabajando = e.hecho("trabajando", 0.0);
        let confirmado = e.suceso("confirmado");
        let aviso_urgente = e.suceso("aviso_urgente");

        // ── quién tiene la cara ──────────────────────────────────
        //  De más a menos. Dejar de buscar no puede borrar el disco rojo:
        //  nadie borra nada, solo deja de reclamar.
        let forma = e.capa("forma", Muelle::RAPIDO, vec![
            Reclamacion::mientras("rec", grabando),
            Reclamacion::tras("aviso", &[aviso_urgente], 700),
            Reclamacion::tras("contenta", &[confirmado], 620),
            Reclamacion::mientras("lupa", buscando),
            Reclamacion::por_defecto("ojos"),
        ]);
        let (p_rec, p_aviso, p_contenta, p_lupa, p_ojos) =
            (forma.presencia(0), forma.presencia(1), forma.presencia(2), forma.presencia(3), forma.presencia(4));

        // ── la pose, en las unidades de Marea ────────────────────
        let ojos = e.prop_de_pose("ojos", 14.0);
        let ancho = e.prop_de_pose("ancho", 6.0);
        let hueco = e.prop_de_pose("hueco", 16.0);
        let giro = e.prop_de_pose("giro", 0.0);
        let sx = e.prop_de_pose("sx", 1.0);
        let sy = e.prop_de_pose("sy", 1.0);
        let sube = e.prop_de_pose("sube", 0.0);
        let mira_x = e.prop_de_pose("mira.x", 0.0);
        let mira_y = e.prop_de_pose("mira.y", 0.0);

        let parpado = e.prop("párpado", 1.0);
        let puntero_x = e.prop_con("puntero.x", 0.0, Muelle::SUAVE);
        let puntero_y = e.prop_con("puntero.y", 0.0, Muelle::SUAVE);

        // ── el dibujo ────────────────────────────────────────────
        let cy = CY + sube * S;
        let cuerpo = Forma::Elipse { centro: (CX.into(), cy.clone()), radio: R.into(), escala: (sx.e(), sy.e()) };
        e.pintar(Instr::Grupo { sombra: Some(Sombra { desplazada: (0.0, 12.0), difusa: 34.0, alfa: 0.34 }) });
        e.pintar(Instr::Forma { forma: cuerpo.clone(), fusion: 0.0.into() });
        e.pintar(Instr::Relleno { pintura: color(0.082, 0.086, 0.086).into(),
            alfa: 1.0.into(),
            filo: 0.05,
            luz: Some(Luz { cantidad: 0.03, desde_y: (CY - R).into(), alto: R * 2.0 }),
            borde: None,
        });
        e.pintar(Instr::Recorte(Some((cuerpo, 3.0))));

        // La cara entera gira con la cabeza, alrededor del centro de la bolita.
        e.pintar(Instr::Transformar(Some(Transformacion::en((CX.into(), cy.clone())).giro(giro * (std::f32::consts::PI / 180.0)))));
        let mira = (mira_x * S + puntero_x, mira_y * S + puntero_y);
        let ojo_x = |lado: f32| CX + mira.0.clone() + hueco * (0.5 * S * lado);
        let ojo_y = |_lado: f32| cy.clone() + mira.1.clone();
        //  Una forma solo se ve con más de media presencia. Como las presencias
        //  de una capa suman uno, nunca hay dos a la vez: una se va y entra la
        //  otra, en vez de fundirse en un borrón.
        let ve = |p: Expr| p.suave(0.5, 1.0);
        let blanco = color(0.96, 0.97, 0.96);

        // ojos · las dos píldoras de siempre. Grabando, la izquierda se queda.
        let pildora = |lado: f32| Forma::Caja {
            centro: (ojo_x(lado), ojo_y(lado)),
            mitad: (ancho * (0.5 * S), (ojos * (0.5 * S) * parpado).max(1.6)),
            radio: ancho * (0.5 * S),
        };
        e.pintar(Instr::Plano { forma: pildora(-1.0), color: blanco.clone(), alfa: ve(p_ojos + p_rec) });
        e.pintar(Instr::Plano { forma: pildora(1.0), color: blanco.clone(), alfa: ve(p_ojos.e()) });

        // rec · el ojo derecho es el disco rojo, que crece al entrar.
        e.pintar(Instr::Plano {
            forma: Forma::circulo((ojo_x(1.0), ojo_y(1.0)), p_rec * (6.5 * S)),
            color: color(0.93, 0.23, 0.2),
            alfa: ve(p_rec.e()),
        });

        // contenta · dos arcos de verdad.
        for lado in [-1.0, 1.0] {
            e.pintar(Instr::Plano {
                forma: Forma::Arco { centro: (ojo_x(lado), ojo_y(lado) + 2.5 * S), radio: (4.6 * S).into(), apertura: 1.2.into(), grosor: (2.7 * S).into() },
                color: blanco.clone(),
                alfa: ve(p_contenta.e()),
            });
        }

        // aviso · una admiración ámbar.
        let ambar = color(0.98, 0.76, 0.32);
        e.pintar(Instr::Plano {
            forma: Forma::Caja { centro: (CX.into(), cy.clone() - 4.0 * S), mitad: ((2.3 * S).into(), (7.0 * S).into()), radio: (2.3 * S).into() },
            color: ambar.clone(),
            alfa: ve(p_aviso.e()),
        });
        e.pintar(Instr::Plano { forma: Forma::circulo((CX.into(), cy.clone() + 9.0 * S), 2.7 * S), color: ambar, alfa: ve(p_aviso.e()) });

        // lupa · un aro y su mango.
        let lx = CX + mira.0.clone() - 2.5 * S;
        let ly = cy.clone() + mira.1.clone() - 2.0 * S;
        e.pintar(Instr::Plano { forma: Forma::aro((lx.clone(), ly.clone()), 6.2 * S, 2.6 * S), color: blanco.clone(), alfa: ve(p_lupa.e()) });
        e.pintar(Instr::Plano {
            forma: Forma::Segmento { de: (lx.clone() + 5.4 * S, ly.clone() + 5.4 * S), a: (lx + 9.6 * S, ly + 9.6 * S), grosor: (2.9 * S).into() },
            color: blanco.clone(),
            alfa: ve(p_lupa.e()),
        });
        e.pintar(Instr::Transformar(None));
        e.pintar(Instr::Recorte(None));

        e.comportamientos = vec![
            Comportamiento::Parpadeo { prop: parpado, cada: (2.5, 7.0), dura: 0.17 },
            Comportamiento::Mirada {
                x: puntero_x,
                y: puntero_y,
                centro: (CX.into(), CY.into()),
                alcance: (5.0 * S, 3.2 * S),
                distancia: 160.0,
                reposo: (0.0.into(), 0.0.into()),
            },
        ];

        // ── los gestos: los fotogramas de Marea, tal cual ────────
        use Curva::*;
        e.gesto("asentir", Clase::Reflejo, vec![
            foto(130, OutQuad).con(mira_y, 4.0).con(ojos, 10.0),
            foto(170, OutBack).con(ojos, 15.0),
            foto(160, InOutSine),
        ]);
        let salto = |f: Fotograma| f.con(ojos, 5.0).con(ancho, 8.0);
        e.gesto("happy", Clase::Pedido, vec![
            // Se agacha, empuja, flota, cae, y un rebote más pequeño.
            salto(foto(130, InQuad)).con(sx, 1.12).con(sy, 0.78).con(sube, 5.0),
            salto(foto(90, OutQuad)).con(sx, 0.90).con(sy, 1.12).con(sube, -3.0),
            salto(foto(150, OutQuad)).con(sx, 0.99).con(sy, 1.03).con(sube, -9.0).con(giro, -5.0).aguanta(35),
            salto(foto(140, InQuad)).con(sx, 0.94).con(sy, 1.08).con(sube, -1.0).con(giro, 2.0),
            salto(foto(105, OutQuad)).con(sx, 1.12).con(sy, 0.80).con(sube, 5.0),
            salto(foto(150, OutQuad)).con(sx, 0.98).con(sy, 1.04).con(sube, -4.0).con(giro, 3.0),
            salto(foto(130, InQuad)).con(sx, 1.04).con(sy, 0.94).con(sube, 1.0),
            foto(200, InOutSine),
        ]);
        let postura = e.gesto("trabajando", Clase::Postura, vec![
            foto(320, InOutSine).con(ojos, 9.0).con(ancho, 7.0).con(giro, -5.0).con(mira_x, -2.0).aguanta(60),
            foto(360, InOutSine).con(ojos, 9.0).con(ancho, 7.0).con(giro, 5.0).con(mira_x, 2.0).aguanta(60),
            foto(260, InOutSine).con(ojos, 11.0),
        ]);
        e.postura(postura, trabajando);
        e
    }

    fn evento(&mut self, e: Evento, c: &mut Contexto) {
        match e {
            Evento::Alarma("inicio") => c.alarma("paso", GUION[0].0),
            Evento::Alarma("paso") => {
                let (_, texto) = GUION[self.paso];
                println!("guion  · {texto}");
                match self.paso {
                    0 => c.hecho("grabando", true),
                    1 => c.hecho("buscando", true),
                    2 => {
                        c.suceso("confirmado");
                        c.gesto("happy");
                        //  Y aquí la lógica se atasca. La cara ni se entera.
                        c.trabajar();
                    }
                    3 => c.hecho("grabando", false),
                    4 => c.suceso("confirmado"),
                    5 => c.hecho("buscando", false),
                    6 => c.suceso("aviso_urgente"),
                    7 => c.hecho("trabajando", true),
                    8 | 10 => c.gesto("asentir"),
                    9 => c.gesto("happy"),
                    _ => c.hecho("trabajando", false),
                }
                self.paso += 1;
                if let Some((espera, _)) = GUION.get(self.paso) {
                    c.alarma("paso", *espera);
                }
            }
            Evento::GestoRechazado(g) => println!("lógica · la escena no concede «{g}»"),
            _ => {}
        }
    }
}
