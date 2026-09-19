# Limitaciones conocidas

Todo lo que se sabe que está a medias, para ir tachándolo. **Gravedad**: 🔴 frena lo siguiente · 🟡 molestará pronto · ⚪ caso raro. Cuando algo se arregle, se mueve a la tabla de abajo con su fecha en vez de borrarlo.

## Pendientes

### Pintado

| # | Limitación | Gravedad | Nota |
| --- | --- | --- | --- |
| P1 | **El texto es un mapa de bits fijo, a escala 1.** Se pinta una vez en CPU y no cambia; en un monitor a escala 2 sale blando (las formas no: son fórmulas) | 🔴 | Es el punto 3: texto dinámico con atlas de glifos a la escala de cada lámina |
| P2 | Sin imágenes ni iconos | 🔴 | Punto 3 |
| P3 | Solo degradado lineal; sin radial, sin desenfoque, sin máscaras | 🟡 | |
| P4 | Un grupo con opacidad dentro de otro no tiene capa propia: multiplica. Con más de cuatro fundiéndose a la vez, los que sobran también | ⚪ | `MAX_CAPAS` en `gpu.rs` |
| P5 | Con escala distinta en cada eje, el suavizado del borde es aproximado | ⚪ | `Afin::factor` usa √det |
| P6 | El degradado y la luz viven en el espacio del grupo: si la forma gira *por sí misma* (`.girada`) no giran con ella | ⚪ | Si gira el grupo, sí |
| P7 | Topes fijos: 4096 formas, 2048 elementos, 4 recortes anidados | ⚪ | Pasados, lo que sobra no se pinta, sin avisar |
| P8 | La lista de dibujo se recompone entera en CPU cada frame, haya cambiado o no | ⚪ | 2000 formas: 0,7 ms. Cuando duela, recomponer solo lo que cambie |
| P9 | Los primeros frames tras despertar no esperan a la pantalla: el reloj de animación va con el tiempo de la CPU, no con el de presentación | 🟡 | Se nota como un arranque una pizca lento |
| P10 | Un frame de cada ~400 cae a 33 ms. Sin investigar | 🟡 | `grim` los provoca; sin capturar también pasa alguno |

### Superficies y entrada

| # | Limitación | Gravedad | Nota |
| --- | --- | --- | --- |
| S1 | El tamaño de la superficie lo declara la escena y es fijo. No crece con el contenido | 🟡 | Se esquiva declarándola grande: la región de entrada deja pasar el clic por lo vacío |
| S2 | Todas las superficies pintan **la misma escena con el mismo estado**. No hay una instancia por monitor (lo que en Quickshell es `Variants`) | 🟡 | Es cosa del lenguaje, no del render |
| S3 | **Sin probar en dos monitores de verdad a distinto ritmo** (165 y 60 Hz). Probado: dos superficies en un monitor, y un monitor virtual que aparece, cambia de escala y desaparece | 🟡 | `--pantalla todas`. La lámina del monitor más rápido espera a la pantalla; las demás no bloquean |
| S4 | La región de entrada son las **cajas** de las zonas activas, no sus formas: las esquinas de un círculo también paran el clic | ⚪ | `wl_region` solo sabe de rectángulos; se podría aproximar con varios |
| S5 | **La mirada solo sigue al ratón dentro de las zonas.** Fuera de ellas el compositor ya no nos cuenta dónde está | 🟡 | Hace falta un puntero global: un hecho que venga de Hyprland. Es del grafo de datos (punto 5) |
| S6 | Sin teclado, rueda, arrastrar, mantener pulsado ni forma del cursor. El botón derecho cierra el programa | 🔴 | |
| S7 | Sin ventanas normales, menús emergentes ni bloqueo de sesión | 🟡 | Tramo 2 |
| S8 | Si el compositor no da `Mailbox` ni `Immediate`, las láminas secundarias esperan también a su pantalla y se frenan entre sí | ⚪ | |
| S9 | Si la GPU se reinicia o se pierde el dispositivo, no se recupera. El adaptador se elige con la primera superficie que llega | ⚪ | |
| S10 | Al salir se llama a `exit`: no se destruye nada en orden | ⚪ | |

### Semántica (capas, gestos, reglas)

| # | Limitación | Gravedad | Nota |
| --- | --- | --- | --- |
| L1 | Dos capas que fijan la misma propiedad: gana la última que cambió | 🟡 | Debería ser un error al cargar |
| L2 | La coreografía de una reclamación no depende de *desde cuál* se llega | 🟡 | `transition a -> b` del borrador |
| L3 | Los gestos no tienen parámetros (`señalar(lado)`) | 🟡 | |
| L4 | Lo que emite un fotograma se atiende en el frame siguiente, y lo del *primer* fotograma se ignora | ⚪ | 16 ms de retraso en un obturador |
| L5 | Los hechos son números. Sin símbolos ni cadenas (`soltando: page`) | 🟡 | |
| L6 | Una zona bajo transformaciones hay que declararla a mano (`zona_bajo`) | ⚪ | Con el árbol del lenguaje saldrá sola |
| L7 | El movimiento reducido arranca y no se cae, pero no está mirado con capturas | 🟡 | |
| L8 | **Las escenas se escriben en Rust.** Sin parser, sin recarga en caliente, y la lógica es un `trait` de Rust, no Luau | 🔴 | Puntos 4 y 5 |

## Arregladas

| Fecha | Qué |
| --- | --- |
| 2026-09-19 | El shader recorría toda la lista en cada píxel → un quad por elemento |
| 2026-09-19 | Las transformaciones no se componían y solo había giro → afines que se multiplican |
| 2026-09-19 | Sin opacidad de grupo → capa intermedia |
| 2026-09-19 | Una textura girada usaba la pantalla entera como caja → sus cuatro esquinas transformadas |
| 2026-09-19 | Las zonas del ratón no seguían a las transformaciones → `zona_bajo` |
| 2026-09-19 | Una sola superficie fija en un monitor, a escala 1, que tapaba el ratón en toda su área → una por monitor con los que van y vienen, escala fraccional, y región de entrada que sigue a las zonas |
| 2026-09-19 | El hover del botón pasaba por la lógica → reglas que ejecuta el render |
