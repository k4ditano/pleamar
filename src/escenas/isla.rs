//! Una isla al estilo de k4, con las mismas piezas que Marea y ni una línea
//! nueva en el render. Aquí la lógica no anima ni decide nada: crecer con el
//! ratón y soltar la gota al pulsar son dos capas y cuatro reglas.

use crate::escena::*;
use crate::logica::{Contexto, Guion};

const CX: f32 = 360.0;
const ARRIBA: f32 = 24.0;
const CERRADA: (f32, f32) = (190.0, 36.0);
const ABIERTA: (f32, f32) = (430.0, 92.0);

#[derive(Default)]
pub struct Isla {
    paso: bool,
    segundos: u32,
}

impl Guion for Isla {
    fn escena(&mut self) -> Escena {
        let mut e = Escena::default();
        let ancho = e.prop("isla.ancho", CERRADA.0);
        let alto = e.prop("isla.alto", CERRADA.1);
        let detalle = e.prop("detalle", 0.0);
        let gota = e.prop("gota", 0.0);
        let fusion = e.prop("fusion", 0.0);
        let late = e.prop("late", 0.0);

        let isla = Forma::Caja {
            centro: (CX.into(), ARRIBA + alto * 0.5),
            mitad: (ancho * 0.5, alto * 0.5),
            radio: (alto * 0.5).min(30.0),
        };
        let gota_x = CX + ancho * 0.5 - 18.0 + gota * 48.0;
        let gota_y = ARRIBA + 18.0;

        e.pintar(Instr::Grupo { sombra: Some(Sombra { desplazada: (0.0, 8.0), difusa: 26.0, alfa: 0.3 }) });
        e.pintar(Instr::Forma { forma: isla.clone(), fusion: 0.0.into() });
        e.pintar(Instr::Forma { forma: Forma::circulo((gota_x.clone(), gota_y.into()), 18.0 * gota.e().suave(0.0, 0.35)), fusion: fusion.e() });
        e.pintar(Instr::Relleno { pintura: color(0.04, 0.043, 0.045).into(), alfa: 1.0.into(), filo: 0.06, luz: None, borde: None });

        // El piloto de la gota, que late solo.
        e.pintar(Instr::Plano {
            forma: Forma::circulo((gota_x, gota_y.into()), 4.5 + late),
            color: color(0.95, 0.36, 0.32),
            alfa: gota.e().suave(0.6, 1.0),
        });

        e.pintar(Instr::Recorte(Some((isla.clone(), 1.0))));
        let t = detalle.e().acotar(0.0, 1.0);
        let blanco = color(0.96, 0.97, 0.96);
        let hora = e.texto_vivo("hora", "--:--");
        let tiempo = e.texto_vivo("tiempo", "0:00 / 4:43");
        // La hora viaja del centro a su esquina cuando la isla crece.
        e.pintar(Instr::Texto {
            contenido: Contenido::Vivo(hora),
            en: (mezcla(CX, CX - ABIERTA.0 * 0.5 + 84.0, t.clone()), mezcla(ARRIBA + 18.0, ARRIBA + 24.0, t.clone())),
            ancla: (0.5, 0.5),
            ancho: None,
            estilo: Estilo::de(15.0, blanco.clone()).peso(600),
            alfa: 0.95.into(), mide: None });
        // Y a su lado, el icono de quien suena. Encontrarlo es cosa de la plataforma.
        let icono = e.imagen(Fuente::Icono("firefox".into()), 28, 28);
        e.pintar(Instr::Imagen { imagen: icono, destino: ((CX - ABIERTA.0 * 0.5 + 22.0).into(), (ARRIBA + 10.0).into(), 28.0.into(), 28.0.into()), alfa: t.clone(), tinte: None });
        let izquierda = CX - ABIERTA.0 * 0.5;
        e.pintar(Instr::Texto { contenido: Contenido::Fijo("Tycho — Awake".into()), en: ((izquierda + 26.0).into(), (ARRIBA + 58.0).into()), ancla: (0.0, 0.5), ancho: Some(250.0.into()), estilo: Estilo::de(16.0, blanco.clone()).peso(500).lineas(1), alfa: t.clone(), mide: None });
        e.pintar(Instr::Texto { contenido: Contenido::Fijo("Reproduciendo".into()), en: ((izquierda + ABIERTA.0 - 26.0).into(), (ARRIBA + 24.0).into()), ancla: (1.0, 0.5), ancho: None, estilo: Estilo::de(12.5, blanco.clone()), alfa: t.clone() * 0.55, mide: None });
        e.pintar(Instr::Texto { contenido: Contenido::Vivo(tiempo), en: ((izquierda + ABIERTA.0 - 26.0).into(), (ARRIBA + 76.0).into()), ancla: (1.0, 0.5), ancho: None, estilo: Estilo::de(12.5, blanco), alfa: t.clone() * 0.6, mide: None });
        e.pintar(Instr::Plano {
            forma: Forma::Caja { centro: ((CX - 60.0).into(), (ARRIBA + 76.0).into()), mitad: (130.0.into(), 1.5.into()), radio: 1.5.into() },
            color: color(0.62, 0.84, 0.74),
            alfa: t,
        });
        e.pintar(Instr::Recorte(None));

        e.comportamientos = vec![Comportamiento::Onda { prop: late, frecuencia: 5.0, amplitud: gota * 0.9 }];

        let encima = e.hecho("encima", 0.0);
        let suelta = e.hecho("suelta", 0.0);
        e.capa("tamaño", Muelle::VIVO, vec![
            Reclamacion::mientras("grande", encima).fija(vec![
                ir(ancho, ABIERTA.0, Muelle::VIVO, 0),
                ir(alto, ABIERTA.1, Muelle::VIVO, 40),
                ir(detalle, 1.0, Muelle::VIVO, 140),
            ]),
            Reclamacion::por_defecto("pequeña").fija(vec![
                ir(detalle, 0.0, Muelle::VIVO, 0),
                ir(alto, CERRADA.1, Muelle::VIVO, 60),
                ir(ancho, CERRADA.0, Muelle::VIVO, 60),
            ]),
        ]);
        let cuello = |a: f32| vec![ir(fusion, 44.0, Muelle::RAPIDO, 0), ir(gota, a, Muelle::SERENO, 40), ir(fusion, 0.0, Muelle::SERENO, 520)];
        e.capa("gota", Muelle::SERENO, vec![
            Reclamacion::mientras("fuera", suelta).fija(cuello(1.0)),
            Reclamacion::por_defecto("dentro").fija(cuello(0.0)),
        ]);

        let z = e.zona("isla", isla, 1.0);
        e.regla(Disparador::Entra(z), vec![Efecto::Hecho(encima, 1.0)]);
        e.regla(Disparador::Sale(z), vec![Efecto::Hecho(encima, 0.0)]);
        e.regla(Disparador::Pulsa(z), vec![Efecto::Alternar(suelta)]);

        e.superficie = Superficie { alto: 140, ..Default::default() };
        e
    }

    fn evento(&mut self, e: Evento, c: &mut Contexto) {
        match e {
            //  Un reloj: cada segundo la lógica dice qué hora es, y nada más.
            Evento::Alarma("inicio") | Evento::Alarma("segundo") => {
                let ahora = chrono::Local::now();
                c.texto("hora", ahora.format("%H:%M").to_string());
                self.segundos += 1;
                c.texto("tiempo", format!("{}:{:02} / 4:43", (134 + self.segundos) / 60, (134 + self.segundos) % 60));
                c.alarma("segundo", 1000);
            }
            Evento::Capa("gota", _) => c.trabajar(),
            // Sin ratón no hay zona que la abra: la demo cuenta los hechos ella.
            Evento::Demo => {
                self.paso = !self.paso;
                c.hecho("encima", self.paso);
                c.hecho("suelta", self.paso);
            }
            _ => {}
        }
    }
}
