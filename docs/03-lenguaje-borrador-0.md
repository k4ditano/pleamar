# El lenguaje — borrador 0

**Estado:** borrador para discutir. Nada de esto tiene parser todavía. Sale de los bocetos A+B ([[pleamar · 06 Bocetos A-B-C]]) corregidos por la prueba con Marea ([[pleamar · 04 Prueba - cabe Marea]]).

**Sin nombre todavía**, ni extensión de fichero. Las palabras clave están en castellano a la espera de decidirlo ([[pleamar · 05 Decisiones y preguntas abiertas]]).

## Qué es

Un **lenguaje de dominio específico, declarativo y no Turing-completo** para describir escenas de escritorio: qué se ve, cómo se mueve y a qué reacciona. De la familia de CSS, QML o Slint, no de la de Lua o Rust.

Una frase lo resume: **todo lo que se declara aquí lo ejecuta el render, solo, a la cadencia de la pantalla; la lógica se limita a contar lo que pasa.**

## Principios

1. **Sintaxis aburrida, semántica nueva.** Llaves, `nombre: expresión`, como QML. Solo se aparta donde aporta algo que QML no tiene.
2. **Lo declarado no puede colgarse.** Sin bucles libres, sin recursión, sin variables que muten. Todo termina; todo se puede comprobar antes de arrancar.
3. **Los errores salen al cargar, con línea y columna.** Nombrar una propiedad que no existe no es un fallo en ejecución.
4. **La frontera con la lógica se ve.** Lo que corre en el render y lo que corre en la lógica no se confunden leyendo el fichero.
5. **Se escribe con comentarios largos.** El `ExpressionController` de Marea es un tercio prosa, y es lo mejor que tiene.

## Las piezas

### 1. Elementos — el árbol (boceto A)

Lo que se ve. Un hijo se coloca respecto a su padre y se recorta a él.

```
Cuerpo {
    color: #151616;  filo: 5%
    sombra: 0 10, difusa 30, 34%

    Elipse orbe { x: 360 ~vivo;  y: 90 ~vivo;  radio: 28 + respira }
    Caja panel  { izquierda: orbe.x + 62;  ancho: 0 ~sereno;  alto: 0 ~sereno;  radio: 24
                  funde: 0 ~rápido }
}
```

- `Cuerpo` funde sus formas en una sola silueta (mínimo suave) y la rellena. `funde` es el radio del cuello de agua.
- Hoy existen `Elipse`, `Caja`, `Texto` (por atlas). Faltan `Anillo`, `Arco`, `Segmento` y el giro.

### 2. Propiedades y muelles

`nombre: valor` es una constante o un enlace. **`~muelle` la convierte en una propiedad animada**: tiene posición y velocidad, y cuando algo le cambia el destino, va hacia él con ese muelle.

```
x: 360 ~vivo            // muelle con nombre
alto: 0 ~muelle(150, 23)   // rigidez, freno
```

Las propiedades tienen **nombre global** (`orbe.x`). Si el fichero se recarga, las que se llaman igual conservan valor y velocidad: la animación no salta.

### 3. Expresiones

Puras, sin efectos. El render las evalúa cada frame.

- Aritmética, comparación, `y`/`o`/`no`, ternario `c ? a : b`.
- Funciones: `min`, `max`, `acotar`, `abs`, `mezcla(a, b, t)`, `suave(a, b, x)`, `azar(a, b)`, y de color `hsl()`, `aclarar()`.
- **`velocidad(prop)`**: la velocidad de un muelle. La lógica no podría saberla nunca; el render sí.
- Unidades: `ms`, `s`, `%`, `px` implícito. Colores `#rrggbb`.

### 4. Hechos y sucesos — la frontera con la lógica

```
hecho grabando, buscando, durmiendo: bool
hecho cuenta: entero = 0
suceso confirmado, aviso_urgente
suceso ver_evento  ->                  // este sale: de la escena a la lógica
```

Un **hecho** es verdad durante un rato; un **suceso** ocurre en un instante. **Es lo único que cruza**: la lógica pone hechos y emite sucesos; la escena emite sucesos de vuelta. La lógica no toca propiedades, poses ni temporizadores.

### 5. Capas — quién gana

Una capa es un hueco que muchos reclaman. **Gana la primera reclamación que se cumple**; al dejar de cumplirse, se ve la siguiente sola.

```
capa forma {
    rec       mientras grabando
    aviso     700ms tras aviso_urgente
    contenta  620ms tras confirmado
    lupa      mientras buscando
    ojos
}
```

Formas de reclamar: `mientras <condición>`, `<tiempo> tras <suceso>`, `desde <suceso> hasta <suceso>`, y sin nada (por defecto).

Una reclamación puede **fijar propiedades**, y entonces es lo que el boceto B llamaba *estado*:

```
capa tarjeta {
    abierta mientras abierta? { orbe.x: 140;  panel.ancho: 406;  panel.alto: 190;  contenido: 1 }
    reposo                    { orbe.x: 360;  panel.ancho: 0;    panel.alto: 0;    contenido: 0 }
}
```

Y una **transición** dice con qué muelle y qué retraso se va de una a otra:

```
transición reposo -> abierta {
    panel.funde: 96 ~rápido, y a los 560ms 0 ~sereno
    panel.ancho: ~sereno a los 70ms
    contenido:   ~sereno a los 300ms
}
```

### 6. Gestos — líneas de tiempo

La otra forma de animar. Un muelle sigue a un valor; un gesto cuenta una historia.

```
pose reposo { ojos: 14; ancho: 6; hueco: 16; giro: 0; sx: 1; sy: 1; sube: 0; mira: 0 0 }

gesto asentir clase reflejo {
    130ms OutQuad { mira: 0 4; ojos: 10 }
    170ms OutBack { ojos: 15 }
}
postura trabajando mientras herramienta_en_curso { … }
```

Reglas:
1. Cada fotograma parte de la pose de reposo: lo que no nombra, vuelve a su valor.
2. Al acabar vuelve a la pose base de donde esté (`reposo`, `dormida`…).
3. **Clases**: `estado > pedido > reflejo > postura > ambiente`. Un gesto solo interrumpe a otro de su clase o inferior.
4. Puede llevar parámetros (`señalar(lado)`), `emite` sucesos en un fotograma y `reclama` una capa mientras dura.
5. `quieta { … }` declara su cara sin movimiento.

### 7. Reglas — qué hace cambiar las cosas

```
encima ver                       => realce: 1 ~rápido          // el :hover de CSS
encima orbe durante 320ms        => abierta? = sí
fuera de conjunto durante 420ms  => abierta? = no
pulsa ver                        => abierta? = no, impulso orbe.y -620, emite ver_evento
quieto 14s mientras reposo       => durmiendo = sí
cada 2.5s..7s mientras ambiente  => parpadea;  17%: otra vez a los 250ms
```

Disparadores: `encima`, `fuera de`, `pulsa`, `quieto`, `cada a..b`, `al <suceso>`. Modificadores: `durante`, `mientras`, `como mucho cada`. Las zonas son los propios elementos: el render hace el hit-test con la misma fórmula con la que los pinta.

### 8. Movimiento reducido

Un interruptor del runtime, no de cada escena. Activo: los muelles se posan al instante y cada gesto enseña su cara `quieta` lo que duraría. En Marea hoy son 56 ramas escritas a mano.

### 9. La lógica

Aparte, en **Luau**, en un hilo que puede atascarse sin que se note. Recibe sucesos, pone hechos y emite sucesos. Nada más.

```lua
al("ver_evento", function() abrir_calendario() end)
al_cambiar("notificaciones", function(n) hecho.avisos = #n end)
```

## Lo que falta por diseñar

- **Componentes y repetición.** `repite i en 0..12 { … }` y elementos reutilizables con parámetros. Sin esto no hay lista de notificaciones.
- **Layout.** Hoy las posiciones son expresiones a mano. Filas, columnas y «ocupa lo que quede».
- **Texto de verdad**: contenido dinámico, no un mapa de bits fijo.
- **Varios ficheros**: importar gestos, poses y componentes. Los plugins son esto.
- **Tipos**: `bool`, `entero`, `real`, `color`, `símbolo`, `tiempo`. ¿Se infieren o se escriben?
- **Qué pasa cuando dos capas fijan la misma propiedad.**
