# Prueba: ¿cabe la Marea de verdad en el lenguaje?

**Fecha:** 2026-09-19 · **Qué se leyó:** `proyecto-marea/prototype/ExpressionController.qml`, entero, en el commit `ad95cf4`.

**Veredicto corto:** con los *estados planos* del boceto B, **no cabe**. Con tres piezas más —**hechos y sucesos**, **capas con reclamaciones** y **gestos como líneas de tiempo con clase**— **cabe entero, y quita de en medio dos clases de error** que hoy el fichero combate a mano.

## 1. Qué es en realidad el controlador

No es una máquina de estados. Son cinco cosas distintas conviviendo en un `QtObject`:

| Qué | Cuánto | Cómo está hecho hoy |
| --- | --- | --- |
| Un reproductor de gestos | 25 gestos de la casa + los de plugins | Secuencias de fotogramas (`eyes`, `width`, `gap`, `tilt`, `sx`, `sy`, `lift`, `x`, `y`, `ms`, `hold`, `ease`). **Ya son datos.** |
| La forma de los ojos (`forma`) | unas 24 formas, **43 escrituras** | Una cadena que escribe quien quiere. Último en escribir, gana. |
| Adornos que no son `forma` | anillo de terminal, medidores, sudor, perla de Remanso, vistos, órbita, destello, tinte, subir 5 px | Booleanos sueltos con su temporizador |
| Vida propia | parpadeo, mirada, respiro en reposo, respiración dormida | Temporizadores con azar |
| Arbitraje | quién puede pisar a quién | Repartido: `reflejo()`, `ocupadaHasta()`, `caraLibre` (16 condiciones), `insigniaCedida` en la vista |

Y tres impuestos que se pagan por todo el fichero:

- **`reducedMotion` aparece 56 veces.** Casi cada función tiene su rama «sin movimiento».
- **`model.sleeping` aparece 35 veces.** Casi cada función empieza comprobando que no duerme.
- **28 temporizadores**, la mayoría para deshacer algo al cabo de un rato (`volverDeContenta`, `volverDeAviso`, `unsweat`, `unwiden`…).

Un tercio del fichero (679 líneas) es comentario, y es lo mejor que tiene: cada decisión de diseño está razonada. **El lenguaje tiene que dejar escribir así.**

## 2. El hallazgo: la guarda que se repite ocho veces

Este patrón aparece **ocho veces**, casi letra por letra:

```js
function buscar(si) {
    //  Apagar solo lo que se encendió aquí. Sin esta guarda, abrir CUALQUIER
    //  página le borraba la cara […] el disco rojo de estar grabando, incluido.
    //  Es el único error que importa de verdad en esta clase.
    if (!si && forma !== "lupa") return
    forma = si ? "lupa" : "ojos"
}
```

`buscar`, `soltando`, `grabar`, `sinDatos`, `comprobando`, `catalogando`, `enFaro`, `instalando`, `actualizando`. El propio código dice cuál es el problema: **`forma` es una sola variable y muchos la quieren.** La guarda es un parche que hay que acordarse de copiar en cada función nueva.

Y hay sitios donde no está. `confirmado()`, `noticeUrgent()`, `alDia()` y `noticeLanded()` escriben `forma = "contenta"` o `"aviso"` **sin mirar qué había**, y su temporizador devuelve a `"ojos"`. Por lo que leo —no lo he ejecutado—, confirmar algo o recibir un aviso urgente *mientras graba* le quitaría el disco rojo: justo el error que los comentarios llaman «el único que importa». Si está cubierto, es en otro fichero; aquí no lo veo.

**Esto no es un fallo de Marea, es que el modelo de datos no ayuda.** Y es exactamente lo que un lenguaje puede arreglar de raíz.

## 3. Las tres piezas que hacen falta

### 3.1 Hechos y sucesos — la frontera con la lógica

Hoy la lógica llama a 41 funciones del controlador (`onBuscando`, `onGrabacionCambiada`, `onPaquetesAlDia`…) y cada una decide qué hacer con la cara. En el lenguaje, la lógica solo **cuenta lo que pasa**:

```
hecho durmiendo, grabando, buscando, catalogando, en_faro: bool
hecho instalando, actualizando, terminal_ocupada, no_molestar: bool
hecho cuenta: entero = 0
hecho soltando: símbolo?           // nada, o de qué es lo que le dan

suceso confirmado, aviso_urgente, al_día, no_pudo, olvidado
suceso gesto(nombre, clase)
```

Un **hecho** es algo que es verdad durante un rato. Un **suceso** ocurre en un instante. La lógica no toca nada más: ni `forma`, ni poses, ni temporizadores.

### 3.2 Capas con reclamaciones — adiós a la guarda

Una **capa** es un hueco que muchos pueden reclamar. Gana la primera reclamación que se cumple; cuando deja de cumplirse, se ve la siguiente **sola**.

```
capa forma {                          // de más a menos prioridad
    cuenta     mientras cuenta > 0
    rec        mientras grabando
    cámara     mientras en_cámara
    drop       mientras soltando
    roto       desde no_pudo hasta olvidado | instalando | catalogando
    aviso      700ms tras aviso_urgente | cupo_apurado
    contenta   620ms tras confirmado | al_día | avisos_en_racha
    instala    mientras instalando
    actualiza  mientras actualizando
    catálogo   mientras catalogando
    faro       mientras en_faro
    lupa       mientras buscando
    reloj      mientras comprobando_cupos
    sin_datos  mientras cupos_sin_datos y página == "reservas"
    de_plugin  mientras cara_de_plugin y cara.libre
    ojos                              // lo que queda cuando nadie dice nada
}
```

Lo que cambia:

- **Las ocho guardas desaparecen.** Dejar de buscar no puede borrar el disco rojo: `rec` está por encima y nunca dejó de reclamarse.
- **El fallo de `confirmado()` mientras graba no se puede escribir.** `contenta` está por debajo de `rec`; se reclama, no se ve, y caduca sola.
- **Los temporizadores `volverDe…` desaparecen.** `700ms tras aviso_urgente` *es* el temporizador.
- **`wakeReconciliation` desaparece.** Existe porque cara y modelo pueden desincronizarse; si la cara *se deriva* de `durmiendo`, no pueden.

⚠️ **El orden de arriba lo he puesto yo**, deducido de los comentarios. Hoy no existe —manda el último que escribe—, así que hay que revisarlo línea a línea. Esa revisión es, de hecho, el diseño que el fichero actual nunca tuvo que hacer explícito.

Lo que **no** se simplifica tanto: `caraLibre` tiene 16 condiciones. Seis se vuelven implícitas por el orden (grabando, drop, forma ≠ ojos…); las otras diez (`no_molestar`, medidores, sudor, tinte, página abierta con cara propia…) siguen siendo una condición escrita. Es menos, no es magia.

### 3.3 Gestos: líneas de tiempo con clase

Los gestos **ya son datos** en Marea, así que esto es casi una transcripción:

```
pose reposo { ojos: 14; ancho: 6; hueco: 16; giro: 0; sx: 1; sy: 1; sube: 0; mira: 0 0 }

gesto asentir clase reflejo {
    130ms OutQuad { mira: 0 4; ojos: 10 }
    170ms OutBack { ojos: 15 }
}

gesto señalar(lado) clase pedido {
    170ms         { mira: 4*lado 0; giro: 4*lado; ojos: 18 }   aguanta 200ms
    190ms         { ojos: 16 }                                  aguanta 140ms
    110ms InQuad  { mira: 4*lado 0; giro: 5*lado; ojos: 19; sx: 1.08; sy: 0.9; sube: 4 }
    …
}

postura trabajando mientras herramienta_en_curso { … }          // se repite sola
```

Tres reglas del lenguaje sustituyen a código que hoy está escrito a mano:

1. **Cada fotograma parte de `reposo`**: lo que no nombra, vuelve a su valor. Es lo que hace `pose()` con sus doce `=== undefined ?`.
2. **Al acabar, vuelve a la pose base de donde esté** —`reposo` o `dormida`—. Sustituye a `asentarLuego`, a `_esPoseNeutra` y al `concat` que le pega la pose dormida al final de cualquier gesto.
3. **Un gesto solo interrumpe a otro de su clase o inferior**: `estado > pedido > reflejo > postura > ambiente`. Sustituye a `reflejo()`, `_ultimoFueReflejo`, `_finDelGesto` y `ocupadaHasta()`. Es la regla que el comentario de la línea 144 explica en veinte líneas: «los gestos reflejos y los pedidos no valen lo mismo».

Un gesto puede **emitir sucesos** en un fotograma —la cámara necesita `disparo` justo cuando cierra el obturador— y **reclamar una capa** mientras dura:

```
gesto foto clase estado, reclama forma cámara {
    210ms { }
    80ms  { obturador: 1 }    emite disparo
    170ms { obturador: 0 }
    emite captura_terminada
}
```

### 3.4 Y dos cosas menores

**Vida propia, con azar:**

```
cada 2.5s..7s  mientras ambiente { parpadea;  17%: otra vez a los 250ms }
cada 5s..14s   mientras ambiente y miradas y no abierta {
    60%: mira puntero | 40%: mira azar(±3.2, ±2);  suelta tras 620ms..2120ms
}
cada 12s..24s  mientras reposo_desnudo { gesto respiro(lado: azar(-1 | 1)) }
```

**Movimiento reducido, del lenguaje y no de cada función.** Con `movimiento reducido` activo el runtime posa los muelles al instante y de cada gesto enseña su **cara quieta** —declarada con `quieta { … }` o, si no, el fotograma que más se aparta del reposo, que es lo que calcula hoy `_quietaDe()`— durante lo que duraría. **Las 56 ramas se quedan en cero.**

## 4. Lo que no cabe, o cabe con ayuda

| Qué | Dónde está | Qué hacer |
| --- | --- | --- |
| Aclarar un color para que se lea (`_tinteLegible`) | l. 1139 | Funciones de color en las expresiones: `hsl()`, `aclarar()` |
| «Como mucho una vez cada 30 s» (`lastGreeting`, `_ultimoClic`) | l. 1871, 862 | Un modificador de regla: `como mucho cada 30s` |
| Elegir gesto según datos (`_comoLoTraga`) | l. 838 | Ternario con símbolos: `piezas > 1 ? lote : …` |
| Gestos de plugin validados por una herramienta | `extraGestures` | Pasan a ser ficheros del mismo lenguaje; validar es cargar |
| **Decidir cuándo cambia un hecho** | `MascotModel`, servicios | **Se queda en la lógica (Luau). Es su trabajo.** |

Nada de esto rompe el diseño. Lo último es la frontera funcionando como debe.

## 5. Lo que la prueba le pide al runtime

Esto es lo caro, y no es el parser:

1. **Pistas de fotogramas con curva** (`OutBack`, `InQuad`…). Hoy pleamar solo tiene muelles.
2. **El evaluador de capas**: reclamaciones, `mientras`, `N ms tras`, `desde … hasta`.
3. **El canal de hechos y sucesos** lógica → escena, y sucesos escena → lógica.
4. **Azar y reglas temporizadas** en el render.
5. **Gestos con parámetros** (`lado`) y **símbolos** en las expresiones.
6. **Giro.** Las formas no rotan todavía, y `tilt` está en casi todos los gestos.
7. **Más primitivas SDF**: anillo, arco, segmento. Las 24 formas de ojos (cámara, candado, reloj de arena, luna…) no salen de elipses y cajas. **Es el trozo más grande.**

## 6. Tamaño, a ojo

De las 2103 líneas: 679 son comentario (se quedan, valen oro), unas 250 son datos de gestos (se quedan casi igual) y unas 1170 son código. De ese código, la mayor parte son guardas, temporizadores de vuelta, ramas de `reducedMotion` y comprobaciones de `sleeping`. **Estimo 250–350 líneas de lenguaje** para lo mismo. Es una estimación, no una medida: la medida será escribirlo.

## 7. Qué decidimos con esto

- El boceto B se queda corto: `estado` no es el concepto central, **`capa` lo es**. Un `estado` de B (reposo/abierta) es una capa con dos reclamaciones que además fijan propiedades.
- El lenguaje necesita **dos formas de animar**, no una: muelles para lo que sigue a un valor, fotogramas para lo que cuenta una historia.
- El siguiente paso con más retorno **no es el parser**: son los puntos 1–3 de la lista del runtime, probados con Marea: `capa forma` con `rec`, `lupa` y `contenta`, y tres gestos.
