//! Un muestrario de lo que el render sabe pintar, para verlo y no solo
//! compilarlo: degradado y borde, transformaciones que se componen, una textura
//! girada y la diferencia entre fundir un grupo y fundir sus piezas.

use crate::escena::*;
use crate::logica::{Contexto, Guion};
use crate::texto::{fuente, Lienzo};
use std::f32::consts::PI;

#[derive(Default)]
pub struct Muestrario;

impl Guion for Muestrario {
    fn escena(&mut self) -> Escena {
        let mut e = Escena::default();
        let angulo = e.prop("ángulo", 0.0);
        let pulso = e.prop("pulso", 0.0);
        let vaiven = e.prop("vaivén", 0.0);
        e.comportamientos = vec![
            Comportamiento::Avance { prop: angulo, por_segundo: 0.9.into() },
            Comportamiento::Onda { prop: pulso, frecuencia: 2.2, amplitud: 0.14.into() },
            Comportamiento::Onda { prop: vaiven, frecuencia: 1.3, amplitud: 0.4.into() },
        ];
        let blanco = color(0.96, 0.97, 0.96);
        let tinta = color(0.07, 0.075, 0.08);

        // Un fondo oscuro para que todo se lea sobre cualquier escritorio.
        e.pintar(Instr::Grupo { sombra: Some(Sombra { desplazada: (0.0, 8.0), difusa: 24.0, alfa: 0.3 }) });
        e.pintar(Instr::Forma { forma: Forma::Caja { centro: (360.0.into(), 108.0.into()), mitad: (350.0.into(), 98.0.into()), radio: 22.0.into() }, fusion: 0.0.into() });
        e.pintar(Instr::Relleno { pintura: tinta.clone().into(), alfa: 0.94.into(), filo: 0.04, luz: None, borde: None });

        // 1 · degradado y borde, en una caja que gira sobre sí misma.
        e.pintar(Instr::Grupo { sombra: None });
        e.pintar(Instr::Forma {
            forma: Forma::Caja { centro: (95.0.into(), 108.0.into()), mitad: (56.0.into(), 38.0.into()), radio: 16.0.into() }.girada(angulo * 0.35),
            fusion: 0.0.into(),
        });
        e.pintar(Instr::Relleno {
            pintura: Pintura::Lineal { de: (45.0.into(), 70.0.into()), a: (145.0.into(), 146.0.into()), c0: color(0.62, 0.84, 0.74), c1: color(0.55, 0.42, 0.92) },
            alfa: 1.0.into(),
            filo: 0.0,
            luz: None,
            borde: Some((2.5.into(), blanco.clone())),
        });

        // 2 · un reloj: escala ∘ giro ∘ giro. Todo él late; dentro giran las
        //     agujas; y en la punta del minutero, una caja gira al revés.
        let centro = (255.0, 108.0);
        let aqui = || -> Punto { (centro.0.into(), centro.1.into()) };
        e.pintar(Instr::Transformar(Some(Transformacion::en(aqui()).escala(1.0 + pulso, 1.0 + pulso))));
        e.pintar(Instr::Plano { forma: Forma::aro(aqui(), 52.0, 3.0), color: blanco.clone(), alfa: 1.0.into() });
        for k in 0..12 {
            e.pintar(Instr::Transformar(Some(Transformacion::en(aqui()).giro(k as f32 * PI / 6.0))));
            e.pintar(Instr::Plano {
                forma: Forma::Segmento { de: (centro.0.into(), (centro.1 - 45.0).into()), a: (centro.0.into(), (centro.1 - 39.0).into()), grosor: 2.0.into() },
                color: blanco.clone(),
                alfa: 0.55.into(),
            });
            e.pintar(Instr::Transformar(None));
        }
        e.pintar(Instr::Transformar(Some(Transformacion::en(aqui()).giro(angulo / 12.0))));
        e.pintar(Instr::Plano { forma: Forma::Segmento { de: aqui(), a: (centro.0.into(), (centro.1 - 22.0).into()), grosor: 4.0.into() }, color: blanco.clone(), alfa: 1.0.into() });
        e.pintar(Instr::Transformar(None));
        e.pintar(Instr::Transformar(Some(Transformacion::en(aqui()).giro(angulo.e()))));
        e.pintar(Instr::Plano { forma: Forma::Segmento { de: aqui(), a: (centro.0.into(), (centro.1 - 36.0).into()), grosor: 2.6.into() }, color: color(0.62, 0.84, 0.74), alfa: 1.0.into() });
        let punta: Punto = (centro.0.into(), (centro.1 - 36.0).into());
        e.pintar(Instr::Transformar(Some(Transformacion::en(punta.clone()).giro(angulo * -3.0))));
        e.pintar(Instr::Plano { forma: Forma::Caja { centro: punta, mitad: (5.0.into(), 5.0.into()), radio: 1.5.into() }, color: color(0.98, 0.76, 0.32), alfa: 1.0.into() });
        e.pintar(Instr::Transformar(None));
        e.pintar(Instr::Transformar(None));
        e.pintar(Instr::Transformar(None));

        // 3 · una textura girada, con un marco en la misma transformación: si
        //     caja y textura no fueran de la mano, se vería.
        let rotulo = (430.0, 108.0);
        e.pintar(Instr::Transformar(Some(Transformacion::en((rotulo.0.into(), rotulo.1.into())).giro(vaiven.e()))));
        e.pintar(Instr::Textura { destino: ((rotulo.0 - 60.0).into(), (rotulo.1 - 16.0).into(), 120.0.into(), 32.0.into()), uv: [0.0, 0.0, 1.0, 1.0], alfa: 1.0.into() });
        e.pintar(Instr::Plano {
            forma: Forma::Caja { centro: (rotulo.0.into(), rotulo.1.into()), mitad: (64.0.into(), 20.0.into()), radio: 8.0.into() }.trazo(1.5),
            color: blanco.clone(),
            alfa: 0.5.into(),
        });
        e.pintar(Instr::Transformar(None));

        // 4 · fundir las piezas (arriba) o fundir el grupo (abajo). Detrás, unas
        //     rayas para que se vea a través.
        for k in 0..9 {
            let x = 560.0 + k as f32 * 13.0;
            e.pintar(Instr::Plano { forma: Forma::Segmento { de: (x.into(), 30.0.into()), a: (x.into(), 186.0.into()), grosor: 3.0.into() }, color: color(0.62, 0.84, 0.74), alfa: 0.8.into() });
        }
        let medio = 0.5 + vaiven;
        let pareja = |e: &mut Escena, y: f32, alfa: Expr| {
            e.pintar(Instr::Plano { forma: Forma::circulo((595.0.into(), y.into()), 26.0), color: color(0.96, 0.97, 0.96), alfa: alfa.clone() });
            e.pintar(Instr::Plano { forma: Forma::circulo((627.0.into(), y.into()), 26.0), color: color(0.93, 0.23, 0.2), alfa });
        };
        pareja(&mut e, 68.0, medio.clone());
        e.pintar(Instr::Opacidad(Some(medio)));
        pareja(&mut e, 148.0, 1.0.into());
        e.pintar(Instr::Opacidad(None));

        // Una zona bajo las mismas transformaciones que lo que se ve: el rótulo
        // se balancea, y el ratón acierta donde está, no donde estaba.
        let z = e.zona_bajo(
            "rótulo",
            Forma::Caja { centro: (rotulo.0.into(), rotulo.1.into()), mitad: (64.0.into(), 20.0.into()), radio: 8.0.into() },
            1.0,
            vec![Transformacion::en((rotulo.0.into(), rotulo.1.into())).giro(vaiven.e())],
        );
        let _ = z;

        let normal = fuente("Inter:medium");
        let mut l = Lienzo::nuevo(120, 32);
        let w = Lienzo::medir(&normal, "pleamar", 22.0, 0.0);
        l.escribir(&normal, "pleamar", 22.0, 60.0 - w / 2.0, 23.0, [0.96, 0.97, 0.96], 1.0, 0.0);
        e.atlas = Some(l);
        e
    }
    fn evento(&mut self, _: Evento, _: &mut Contexto) {}
}
