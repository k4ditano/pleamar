# The language — draft 0


> The code in this note is written in Spanish on purpose: that is how the draft was written at the time, and translating its keywords would misrepresent what was actually considered. The language that came out of it has English keywords.

> **Historical.** This is the sketch, with Spanish keywords, from before implementing it. What actually works is in [[pleamar · 09 El lenguaje v0]].

**Status:** draft, to discuss. None of this has a parser yet. It comes out of sketches A+B ([[pleamar · 06 Bocetos A-B-C]]) corrected by the test against Marea ([[pleamar · 04 Prueba - cabe Marea]]).

**Still unnamed**, and with no file extension. The keywords are in Spanish while that gets decided ([[pleamar · 05 Decisiones y preguntas abiertas]]).

## What it is

A **declarative, non-Turing-complete domain-specific language** for describing desktop scenes: what is seen, how it moves and what it reacts to. Of the CSS, QML or Slint family, not the Lua or Rust one.

One sentence sums it up: **everything declared here is run by the renderer, on its own, at the cadence of the screen; the logic does no more than report what happens.**

## Principles

1. **Boring syntax, new semantics.** Braces, `nombre: expresión`, like QML. It only departs from that where it brings something QML does not have.
2. **What is declared cannot hang.** No free loops, no recursion, no mutating variables. Everything terminates; everything can be checked before starting.
3. **Errors come out at load time, with line and column.** Naming a property that does not exist is not a runtime failure.
4. **The boundary with the logic is visible.** What runs in the renderer and what runs in the logic are not mixed up when reading the file.
5. **It is written with long comments.** Marea's `ExpressionController` is a third prose, and that is the best thing it has.

## The pieces

### 1. Elements — the tree (sketch A)

What is seen. A child is placed relative to its parent and clipped to it.

```
Cuerpo {
    color: #151616;  filo: 5%
    sombra: 0 10, difusa 30, 34%

    Elipse orbe { x: 360 ~vivo;  y: 90 ~vivo;  radio: 28 + respira }
    Caja panel  { izquierda: orbe.x + 62;  ancho: 0 ~sereno;  alto: 0 ~sereno;  radio: 24
                  funde: 0 ~rápido }
}
```

- `Cuerpo` blends its shapes into a single silhouette (smooth minimum) and fills it. `funde` is the radius of the water neck.
- Today there are `Elipse`, `Caja`, `Texto` (by atlas). Missing: `Anillo`, `Arco`, `Segmento` and rotation.

### 2. Properties and springs

`nombre: valor` is a constant or a binding. **`~muelle` turns it into an animated property**: it has position and velocity, and when something changes its target, it heads there with that spring.

```
x: 360 ~vivo            // a named spring
alto: 0 ~muelle(150, 23)   // stiffness, damping
```

Properties have a **global name** (`orbe.x`). If the file reloads, the ones with the same name keep value and velocity: the animation does not jump.

### 3. Expressions

Pure, no effects. The renderer evaluates them every frame.

- Arithmetic, comparison, `y`/`o`/`no`, ternary `c ? a : b`.
- Functions: `min`, `max`, `acotar`, `abs`, `mezcla(a, b, t)`, `suave(a, b, x)`, `azar(a, b)`, and for color `hsl()`, `aclarar()`.
- **`velocidad(prop)`**: the velocity of a spring. The logic could never know it; the renderer can.
- Units: `ms`, `s`, `%`, implicit `px`. Colors `#rrggbb`.

### 4. Facts and events — the boundary with the logic

```
hecho grabando, buscando, durmiendo: bool
hecho cuenta: entero = 0
suceso confirmado, aviso_urgente
suceso ver_evento  ->                  // this one goes out: from the scene to the logic
```

A **fact** is true for a while; an **event** happens in an instant. **It is the only thing that crosses**: the logic sets facts and emits events; the scene emits events back. The logic does not touch properties, poses or timers.

### 5. Layers — who wins

A layer is a slot many things claim. **The first claim that holds wins**; when it stops holding, the next one is seen on its own.

```
capa forma {
    rec       mientras grabando
    aviso     700ms tras aviso_urgente
    contenta  620ms tras confirmado
    lupa      mientras buscando
    ojos
}
```

Ways to claim: `mientras <condición>`, `<tiempo> tras <suceso>`, `desde <suceso> hasta <suceso>`, and nothing at all (the default).

A claim can **set properties**, and then it is what sketch B called a *state*:

```
capa tarjeta {
    abierta mientras abierta? { orbe.x: 140;  panel.ancho: 406;  panel.alto: 190;  contenido: 1 }
    reposo                    { orbe.x: 360;  panel.ancho: 0;    panel.alto: 0;    contenido: 0 }
}
```

And a **transition** says with which spring and which delay it goes from one to the other:

```
transición reposo -> abierta {
    panel.funde: 96 ~rápido, y a los 560ms 0 ~sereno
    panel.ancho: ~sereno a los 70ms
    contenido:   ~sereno a los 300ms
}
```

### 6. Gestures — timelines

The other way to animate. A spring follows a value; a gesture tells a story.

```
pose reposo { ojos: 14; ancho: 6; hueco: 16; giro: 0; sx: 1; sy: 1; sube: 0; mira: 0 0 }

gesto asentir clase reflejo {
    130ms OutQuad { mira: 0 4; ojos: 10 }
    170ms OutBack { ojos: 15 }
}
postura trabajando mientras herramienta_en_curso { … }
```

Rules:
1. Every keyframe starts from the rest pose: whatever it does not name goes back to its value.
2. When it ends it returns to the base pose of wherever it is (`reposo`, `dormida`…).
3. **Classes**: `estado > pedido > reflejo > postura > ambiente`. A gesture only interrupts another of its own class or lower.
4. It can take parameters (`señalar(lado)`), `emite` events in a keyframe and `reclama` a layer for as long as it lasts.
5. `quieta { … }` declares its still face.

### 7. Rules — what makes things change

```
encima ver                       => realce: 1 ~rápido          // the :hover of CSS
encima orbe durante 320ms        => abierta? = sí
fuera de conjunto durante 420ms  => abierta? = no
pulsa ver                        => abierta? = no, impulso orbe.y -620, emite ver_evento
quieto 14s mientras reposo       => durmiendo = sí
cada 2.5s..7s mientras ambiente  => parpadea;  17%: otra vez a los 250ms
```

Triggers: `encima`, `fuera de`, `pulsa`, `quieto`, `cada a..b`, `al <suceso>`. Modifiers: `durante`, `mientras`, `como mucho cada`. The zones are the elements themselves: the renderer hit-tests with the same formula it paints them with.

### 8. Reduced motion

A runtime switch, not a per-scene one. When on: the springs settle instantly and each gesture shows its `quieta` face for as long as it would have lasted. In Marea today that is 56 hand-written branches.

### 9. The logic

Apart, in **Luau**, on a thread that can stall without it showing. It receives events, sets facts and emits events. Nothing else.

```lua
al("ver_evento", function() abrir_calendario() end)
al_cambiar("notificaciones", function(n) hecho.avisos = #n end)
```

## What is left to design

- **Components and repetition.** `repite i en 0..12 { … }` and reusable elements with parameters. Without this there is no notification list.
- **Layout.** Today the positions are hand-written expressions. Rows, columns and "take whatever is left".
- **Real text**: dynamic content, not a fixed bitmap.
- **Several files**: importing gestures, poses and components. Plugins are this.
- **Types**: `bool`, `entero`, `real`, `color`, `símbolo`, `tiempo`. Inferred or written?
- **What happens when two layers set the same property.**
