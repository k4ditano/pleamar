//! Marea: la bolita, sus ojos y la tarjeta de aviso que le nace del costado.
//! Todo lo que hay aquí son datos y decisiones; ni un píxel.

use crate::escena::*;
use crate::logica::{Contexto, Guion};

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
    abierta: bool,
    cuantos: usize,
}

impl Guion for Marea {
    fn escena(&mut self) -> Escena {
        let mut e = Escena::default();
        // Banco de ensayo: mide lo suyo. `--pantalla` dice en cuál se abre.
        e.superficies = vec![Superficie { ancho: 720, alto: 224, margen: [40, 0, 0, 0], ..Default::default() }];
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
        let mira_x = e.prop_con("mirada.x", 0.0, Muelle::SUAVE);
        let mira_y = e.prop_con("mirada.y", 0.0, Muelle::SUAVE);

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
        e.pintar(Instr::Grupo { sombra: Some(Sombra { desplazada: (0.0.into(), 10.0.into()), difusa: 30.0.into(), alfa: 0.34.into(), color: None }) });
        e.pintar(Instr::Forma { forma: orbe.clone(), fusion: 0.0.into() });
        e.pintar(Instr::Forma { forma: panel.clone(), fusion: fusion * panel_w.e().suave(0.0, 40.0) });
        e.pintar(Instr::Relleno { pintura: color(0.082, 0.086, 0.086).into(),
            alfa: 1.0.into(),
            filo: 0.05,
            luz: Some(Luz { cantidad: 0.03, desde_y: orbe_y - ANCLA_Y, alto: PANEL_H }),
            borde: None,
            vidrio: None,
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
        //  El contenido se funde como una sola cosa: a medio aparecer, el botón
        //  no se transparenta a través de su propio rótulo.
        e.pintar(Instr::Opacidad(Some(visible)));
        let visible: Expr = 1.0.into();
        e.pintar(Instr::Plano {
            forma: caja_en(110.0, 151.0, 84.0.into(), 23.0.into()),
            color: color(0.18, 0.184, 0.184),
            alfa: visible.clone(), vidrio: None,
        });
        let ver = caja_en(296.0, 151.0, 84.0 + boton * 2.0, 23.0 + boton * 1.5);
        e.pintar(Instr::Plano {
            forma: ver.clone(),
            color: [mezcla(0.62, 0.74, boton), mezcla(0.84, 0.93, boton), mezcla(0.74, 0.84, boton)],
            alfa: visible.clone(), vidrio: None,
        });
        // El texto es texto: lo cambia la lógica cuando llega otro aviso, y se
        // pinta nítido a la escala de cada monitor.
        let blanco = color(0.96, 0.97, 0.96);
        let app = e.texto_vivo("aviso.app", "CALENDARIO");
        let cuando = e.texto_vivo("aviso.cuando", "Ahora");
        let titulo = e.texto_vivo("aviso.título", "Reunión en 5 min");
        let detalle = e.texto_vivo("aviso.detalle", "Revisión de diseño · 18:00");
        let escribe = |e: &mut Escena, c: Contenido, x: f32, y: f32, ancla: (f32, f32), ancho: Option<f32>, estilo: Estilo, alfa: f32| {
            e.pintar(Instr::Texto { contenido: c, en: (panel_x.clone() + x, origen_y.clone() + y), ancla, ancho: ancho.map(Into::into), estilo, alfa: alfa.into(), mide: None });
        };
        escribe(&mut e, Contenido::Vivo(app), 26.0, 33.0, (0.0, 0.5), None, Estilo::de(12.5, blanco.clone()), 0.62);
        escribe(&mut e, Contenido::Vivo(cuando), 380.0, 33.0, (1.0, 0.5), None, Estilo::de(13.5, blanco.clone()), 0.55);
        // Una sola línea: si el título no cabe, puntos suspensivos.
        escribe(&mut e, Contenido::Vivo(titulo), 26.0, 71.0, (0.0, 0.5), Some(354.0), Estilo::de(20.0, blanco.clone()).peso(500).lineas(1), 1.0);
        escribe(&mut e, Contenido::Vivo(detalle), 26.0, 99.0, (0.0, 0.5), Some(354.0), Estilo::de(14.5, blanco.clone()).lineas(1), 0.6);
        escribe(&mut e, Contenido::Fijo("Descartar".into()), 110.0, 151.0, (0.5, 0.5), None, Estilo::de(15.0, blanco.clone()).peso(500), 0.95);
        escribe(&mut e, Contenido::Fijo("Ver evento".into()), 296.0, 151.0, (0.5, 0.5), None, Estilo::de(15.0, color(0.07, 0.12, 0.10)).peso(500), 1.0);
        e.pintar(Instr::Opacidad(None));

        // Los ojos: dos píldoras recortadas a la cara.
        let abierto = (parpado * (1.0 - sueno)).max(0.14);
        let ojo = |lado: f32| Forma::Caja {
            centro: (orbe_x + mira_x + lado, orbe_y + mira_y + sueno * 3.0 - 1.0),
            mitad: (3.5.into(), 8.4 * abierto.clone()),
            radio: 3.5.into(),
        };
        e.pintar(Instr::Recorte(None));
        e.pintar(Instr::Recorte(Some((orbe.clone(), 3.0))));
        for lado in [-8.6, 8.6] {
            e.pintar(Instr::Plano { forma: ojo(lado), color: color(0.96, 0.97, 0.96), alfa: 1.0.into(), vidrio: None });
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

        // ── la frontera, las capas y las reglas ──────────────────
        let abierta = e.hecho("abierta", 0.0);
        let dormida = e.hecho("dormida", 0.0);
        let ver_evento = e.suceso_que_sale("ver-evento");

        //  Abrir y cerrar son dos reclamaciones de la misma capa, cada una con
        //  su coreografía. Quien ponga `abierta` a sí —una regla del ratón o la
        //  lógica porque ha llegado un aviso— no sabe nada de muelles.
        e.capa("tarjeta", Muelle::SERENO, vec![
            Reclamacion::mientras("abierta", abierta).fija(vec![
                ir(fusion, 96.0, Muelle::RAPIDO, 0),
                ir(orbe_x, ABIERTA_X, Muelle::VIVO, 0),
                ir(panel_w, PANEL_W, Muelle::SERENO, 70),
                ir(panel_h, PANEL_H, Muelle::SERENO, 110),
                ir(contenido, 1.0, Muelle::SERENO, 300),
                ir(fusion, 0.0, Muelle::SERENO, 560),
            ]),
            Reclamacion::por_defecto("reposo").fija(vec![
                ir(boton, 0.0, Muelle::RAPIDO, 0),
                ir(contenido, 0.0, Muelle::RAPIDO, 0),
                ir(fusion, 96.0, Muelle::RAPIDO, 0),
                ir(panel_h, 0.0, Muelle::SERENO, 90),
                ir(panel_w, 0.0, Muelle::SERENO, 90),
                ir(orbe_x, REPOSO_X, Muelle::VIVO, 150),
                ir(fusion, 0.0, Muelle::SERENO, 700),
            ]),
        ]);
        e.capa("sueño", Muelle::LENTO, vec![
            Reclamacion::mientras("dormida", dormida.e().y(abierta.e().no())).fija(vec![ir(sueno, 1.0, Muelle::LENTO, 0)]),
            Reclamacion::por_defecto("despierta").fija(vec![ir(sueno, 0.0, Muelle::VIVO, 0)]),
        ]);

        let conjunto = e.zona(
            "conjunto",
            Forma::Caja {
                centro: ((ABIERTA_X + (HUECO + PANEL_W) * 0.5).into(), (ORBE_Y - ANCLA_Y + PANEL_H * 0.5).into()),
                mitad: (((R * 2.0 + HUECO + PANEL_W) * 0.5 + 18.0).into(), (PANEL_H * 0.5 + 18.0).into()),
                radio: 24.0.into(),
            },
            //  Solo mientras hay tarjeta. Cerrada, esa zona está vacía, y el
            //  clic tiene que pasar a lo que haya debajo.
            abierta.e().o(panel_w.e().mayor(1.0)),
        );
        let z_orbe = e.zona("orbe", Forma::circulo((orbe_x.e(), orbe_y.e()), R + 8.0), 1.0);
        let z_descartar = e.zona("descartar", caja_en(110.0, 151.0, 84.0.into(), 23.0.into()), contenido);
        let z_ver = e.zona("ver", ver, contenido);

        //  Todo esto lo ejecuta el render. Con la lógica muerta, Marea se abre,
        //  se cierra, se duerme y realza su botón igual.
        use Disparador::*;
        e.regla(Entra(z_orbe), vec![Efecto::Hecho(dormida, 0.0.into())]);
        e.regla(Encima { zona: z_orbe, durante: ms(320) }, vec![Efecto::Hecho(abierta, 1.0.into())]);
        e.regla(Pulsa(z_orbe), vec![Efecto::Alternar(abierta)]);
        e.regla(Fuera { zona: conjunto, durante: ms(420) }, vec![Efecto::Hecho(abierta, 0.0.into())]);
        e.regla(Entra(z_ver), vec![Efecto::Animar(ir(boton, 1.0, Muelle::RAPIDO, 0))]);
        e.regla(Sale(z_ver), vec![Efecto::Animar(ir(boton, 0.0, Muelle::RAPIDO, 0))]);
        e.regla(Pulsa(z_descartar), vec![Efecto::Hecho(abierta, 0.0.into())]);
        e.regla(Pulsa(z_ver), vec![Efecto::Hecho(abierta, 0.0.into()), Efecto::Impulso(orbe_y, (-620.0).into()), Efecto::Suceso(ver_evento, None)]);
        e.regla(Quieto { durante: ms(14_000), mientras: abierta.e().no() }, vec![Efecto::Hecho(dormida, 1.0.into())]);

        e
    }

    fn evento(&mut self, e: Evento, c: &mut Contexto) {
        match e {
            //  Lo único que le queda a la lógica: enterarse y hacer SU trabajo
            //  —montar el contenido, abrir el calendario—, que puede tardar.
            Evento::Capa("tarjeta", _) => c.trabajar(),
            Evento::Suceso("ver-evento", _) => println!("logic  · someone wants to see the event"),
            //  Sin ratón, la demo hace de aviso que llega y se va.
            Evento::Demo => {
                self.abierta = !self.abierta;
                if self.abierta {
                    //  Cada vez llega un aviso distinto: la lógica cambia el texto
                    //  y nada más. El tercero no cabe, a propósito.
                    const AVISOS: [[&str; 3]; 3] = [
                        ["CALENDARIO", "Reunión en 5 min", "Revisión de diseño · 18:00"],
                        ["MENSAJES", "Lucía: ¿bajas a por café? ☕", "Hace un momento"],
                        ["ACTUALIZACIONES", "Hay 214 paquetes esperando a que alguien se decida a instalarlos", "yay · 1,2 GB"],
                    ];
                    let a = AVISOS[self.cuantos % AVISOS.len()];
                    self.cuantos += 1;
                    c.texto("aviso.app", a[0]);
                    c.texto("aviso.título", a[1]);
                    c.texto("aviso.detalle", a[2]);
                }
                c.hecho("abierta", self.abierta);
            }
            _ => {}
        }
    }
}
