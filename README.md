# pleamar

Prototipo de una idea: **en una shell de escritorio, animar no debería depender
de la lógica**. La vista declara transiciones («el ancho va a 406 con este
muelle, dentro de 70 ms») y un hilo de render las recorre a la cadencia de la
pantalla, pase lo que pase en el hilo que decide. Es lo que hace Core Animation
en iOS, y lo que QtQuick —y por tanto Quickshell— no puede hacer con un
`Behavior` o un `SpringAnimation`.

Imita a [Marea](../proyecto-marea): la bolita grafito, sus ojos, y la tarjeta de
aviso que le nace del costado.

```sh
cargo build --release
./target/release/pleamar                 # pasa el ratón por la bolita; botón derecho la cierra
./target/release/pleamar --escena isla   # otra escena, el mismo render
./target/release/pleamar --escena cara   # la cara de Marea: capas, gestos y su guion
./target/release/pleamar --escena muestrario   # todo lo que el render sabe pintar, a la vista
./target/release/pleamar --demo          # abre y cierra sola
./target/release/pleamar --ingenuo       # lo mismo con la lógica en el hilo que pinta
```

Sale en `HDMI-A-1`. `--pantalla todas` la pone en cada monitor —y en los que se
enchufen después—; `--pantalla A,B` en los que digas. `--bloqueo MS` cambia lo que se
atasca la lógica tras cada decisión; por defecto 600 ms de espera activa.
`--raton "360,90@500 pulsa@3200 fuera@4500"` mueve un ratón de mentira y cuenta
cada evento que le llega a la lógica: sirve para ensayar sin tocar el de verdad.

## Una escena son datos

El render no sabe qué es una bolita. Recibe una `Escena` y la interpreta:

- **Propiedades** con nombre (`orbe.x`, `panel.ancho`…). Cada una es un muelle.
  Si la escena se sustituye, las que se llaman igual conservan valor y velocidad.
- **Expresiones** puras sobre ellas (`orbe_x + 62.0`, `fusion * panel_w.suave(0, 40)`,
  incluso `orbe_x.vel()`). Son lo que sería un binding: como no tienen efectos,
  el render las evalúa cuando quiere, sin preguntar a nadie.
- **Instrucciones de dibujo**, en orden: `Grupo`, `Forma` (elipse o caja, fundida
  con lo anterior por un mínimo suave), `Relleno`, `Recorte`, `Transformar`,
  `Plano`, `Textura`. El render la compone cada frame en elementos con su caja
  envolvente y los pinta de una sola llamada; no hay nada de Marea en él.
- **Hechos y sucesos**: la única frontera con la lógica. Ella cuenta lo que pasa
  (`grabando = sí`, `confirmado`); no toca formas, poses ni temporizadores.
- **Capas**: un hueco que muchos reclaman, en orden de prioridad. Gana la primera
  reclamación que se cumple; cuando deja de cumplirse se ve la siguiente, sola.
  Una reclamación puede fijar propiedades con su muelle y su retraso: eso es una
  coreografía, y lo que en otros sitios se llama estado.
- **Gestos**: fotogramas con curva sobre las propiedades de pose, con clase
  (`Estado > Pedido > Reflejo > Postura > Ambiente`): uno solo corta a otro de
  su clase o inferior.
- **Reglas** que ejecuta el render: `Encima{320 ms}`, `Fuera{420 ms}`, `Pulsa`,
  `Quieto{14 s}`… → poner un hecho, alternarlo, emitir, impulsar, pedir un gesto.
- **Comportamientos** que el render lleva solo: `Parpadeo`, `Onda`, `Mirada`.
- **Zonas** sensibles al ratón. Lo que declaran en `al_entrar` y `al_salir` lo
  ejecuta el render en el acto —el `:hover` de CSS—, y a la lógica le llega un
  evento con nombre: `Entra("ver")`, `Pulsa("orbe")`. La lógica ya no sabe de
  coordenadas.

| | |
| --- | --- |
| `main.rs` | Wayland: una superficie layer-shell por monitor, su escala (también fraccional) y el ratón. Nada más. |
| `gpu.rs` | El dispositivo, las láminas —una por superficie, cada una a su escala y con su ritmo— y la composición de la lista de dibujo en elementos. |
| `escena.rs` | El contrato: propiedades, expresiones, instrucciones, comportamientos, zonas. |
| `render.rs` | Intérprete. Dueño de los muelles y del reloj; quieto, no pinta ni un frame. |
| `forma.wgsl` | Un quad por elemento: cada píxel solo ejecuta las formas del elemento que lo cubre. |
| `formas.rs` | La geometría, una vez, para tres usos: la GPU, el ratón y las cajas. Elipse, caja, arco, segmento, trazo y giro. |
| `logica.rs` | Donde corre un `Guion`: recibe eventos, declara transiciones y alarmas, y se bloquea a propósito. |
| `escenas/marea.rs` | La bolita y su tarjeta: 12 propiedades, 12 instrucciones, 4 zonas. |
| `escenas/isla.rs` | Una isla como la de k4 que suelta una gota. Su lógica no decide nada: dos capas y tres reglas. |
| `escenas/muestrario.rs` | Degradado y borde, un reloj con tres transformaciones anidadas, una textura girada y fundir un grupo frente a fundir sus piezas. |
| `escenas/cara.rs` | La cara de Marea: `capa forma` (rec > aviso > contenta > lupa > ojos) y tres de sus gestos, fotograma a fotograma. |
| `texto.rs` | Texto pintado una vez a un atlas: el papel del «contenido de un plugin». |

La gráfica de abajo es una barra por frame; la franja roja es el tiempo que la
lógica estuvo bloqueada, y el piloto de la izquierda, su estado ahora.

## Lo que se midió (19 sep 2026, RTX 2060, pantalla a 60 Hz)

Abrir la tarjeta con la lógica bloqueada 600 ms justo al empezar:

| | frames pintados durante el bloqueo | frame más largo |
| --- | --- | --- |
| pleamar, render separado | 38 | 17–19 ms (un ciclo de cada cinco soltó un frame: 33 ms) |
| pleamar `--ingenuo` | 0 | 617 ms |
| QtQuick sobre Quickshell (`comparar/shell.qml`) | 0 | 600 ms |

Memoria con la escena en reposo: 119 MB de RSS (76 privados) frente a 214 MB
(145 privados) del banco de Quickshell, que es una ventana con tres
rectángulos. Casi todo lo de pleamar es el driver de Vulkan de NVIDIA: el suelo
de abrir un contexto de GPU existe, y un renderer por CPU para lo estático
sería la forma de bajarlo.

Con capas y reglas, Marea se abre, realza su botón y se cierra **con la lógica
bloqueada cinco segundos** (`--bloqueo 5000 --raton …`): la lógica se entera
después. Y en `--escena cara`, buscar y confirmar mientras graba no le quitan el
disco rojo; al dejar de grabar, la lupa sale sola.

Con la escena como datos los números no cambian (38–39 frames durante el
bloqueo, 17 ms), y el realce del botón responde con la lógica congelada.

## Documentación

El diseño, el borrador del lenguaje, la prueba contra la Marea real y lo que
falta —con la lista de limitaciones conocidas en `docs/08`— están en `docs/` (y, al día, en Edinot: `Proyectos/pleamar`).

## Lo que no es

Una escena se escribe todavía en Rust y se compila con el programa: falta el
lenguaje que la describa desde un fichero y la recargue en caliente. No hay
layout —las posiciones son expresiones a mano—, el texto es un mapa de bits fijo
y todas las superficies pintan la misma escena. Y los primeros frames tras despertar salen
sin esperar al vsync, así que el reloj de animación debería ir con el tiempo de
presentación, no con el de la CPU.
