# Three sketches for the language


> The code in this note is written in Spanish on purpose: that is how the draft was written at the time, and translating its keywords would misrepresent what was actually considered. The language that came out of it has English keywords.

All three describe the same piece of Marea —the body with its water neck, the
eyes, the button that lights up and open/close— and all three compile to the
same thing: the `Escena` of `src/escena.rs`. What changes is where each thing
lives and how much the renderer can do without the logic.

The keywords are in Spanish because the project is; if this ever leaves home,
that is a separate decision.

## A · Tree — "like QML, but the springs belong to the language"

Nested elements with properties. `~` turns a property into a spring. A child is
placed relative to its parent and clipped to it. The logic goes apart, in Luau,
and is imperative just like today.

```
Cuerpo {
    color: #151616;  filo: 5%
    sombra: 0 10, difusa 30, 34%

    Elipse orbe {
        x: 360 ~vivo;  y: 90 ~vivo
        radio: 28 + respira
        estira: velocidad                   // deforms with its own velocity

        Ojos { mirada: puntero, hasta 5 3.2;  parpadea: cada 2.4..6s;  cerrados: sueño }
        respira: onda(1.7) * sueño * 1.3
        sueño: 0 ~lento
    }

    Caja panel {
        izquierda: orbe.x + 62;  arriba: orbe.y - alto * 70/190
        ancho: 0 ~sereno;  alto: 0 ~sereno;  radio: 24
        funde: 0 ~rápido                    // water neck with the previous one
        contenido: 0 ~sereno                // the children's opacity

        Texto { en: 26 78;  "Reunión en 5 min";  20px medio }
        Caja ver {
            centro: 296 151;  tamaño: 168+realce*4  46+realce*3;  radio: 9
            color: mezcla(#9ed6bd, #bdeed6, realce)
            realce: 0 ~rápido
            encima { realce: 1 }            // run by the renderer, no logic
            Texto { centrado; "Ver evento"; 15px medio; #121f1a }
        }
    }
}

lógica {
    local function abrir()
        anima(panel.funde, 96)
        anima(orbe.x, 140)
        anima(panel.ancho, 406, { tras = 70 })
        anima(panel.alto, 190, { tras = 110 })
        anima(panel.contenido, 1, { tras = 300 })
        anima(panel.funde, 0, { tras = 560, muelle = sereno })
    end
    al.encima(orbe, 320, abrir)             -- 320 ms hovering
    al.pulsa(ver, function() cerrar(true) end)
}
```

**For:** it reads like what it is, a drawing made of parts; relative coordinates
and clipping by the parent come out on their own, and from there to a layout is
one step. Anyone coming from QML reads it first time.
**Against:** the choreography is still imperative code. Opening depends on the
logic being alive to declare it: if it is stalled, Marea does not open.

## B · States — "the scene is a machine, and the renderer runs it"

The shapes are declared flat. The central things are the **states** —what value
each property has in each one—, the **transitions** —with which spring and which
delay it goes from one to another— and the **rules** —what makes it change
state—. All of that is run by the renderer. Only facts with meaning reach the logic.

```
prop orbe.x, orbe.y = 360, 90
prop panel.w, panel.h, fusion, contenido, sueño, realce = 0

cuerpo #151616 filo 5% sombra(0 10, 30, 34%) {
    elipse orbe  en orbe.x orbe.y  radio 28+respira  estira velocidad
    caja   panel desde orbe.x+62, orbe.y - panel.h*70/190  tamaño panel.w panel.h  radio 24
                 funde fusion * suave(0, 40, panel.w)
}
dentro de panel, opacidad contenido {
    caja  ver  centro 296 151  tamaño 168+realce*4 46+realce*3  radio 9
               color mezcla(#9ed6bd, #bdeed6, realce)
    texto "Ver evento" centrado en ver  15px medio  #121f1a
}
ojos en orbe  mirada puntero hasta 5 3.2  parpadea cada 2.4..6s  cerrados sueño

estado reposo  { orbe.x: 360;  panel: 0 0;      contenido: 0 }
estado abierta { orbe.x: 140;  panel: 406 190;  contenido: 1 }
estado dormida : reposo { sueño: 1 ~lento }

transición reposo -> abierta {
    fusion:    96 ~rápido, y a los 560ms 0 ~sereno
    orbe.x:    ~vivo
    panel.w:   ~sereno  a los 70ms
    panel.h:   ~sereno  a los 110ms
    contenido: ~sereno  a los 300ms
}
transición abierta -> reposo { … }

encima ver                        => realce: 1 ~rápido
encima orbe durante 320ms         => abierta
pulsa orbe                        => alterna reposo abierta
fuera de conjunto durante 420ms   => reposo
pulsa ver                         => reposo, impulso orbe.y -620, emite "ver-evento"
quieto 14s en reposo              => dormida
encima orbe en dormida            => reposo
```

**For:** Marea opens, closes, falls asleep and answers the mouse with the logic
dead. The logic is left with what is really its own: putting the text in,
deciding a notice has arrived, reacting to `"ver-evento"`. And it is checkable
before running: a state that names a property which does not exist is an error
with a line number.
**Against:** a flat state machine does not hold up against the real Marea. Its
`ExpressionController` is 2100 lines of "who wins": asleep, recording, urgent
notice, Remanso… It would need layered states with priority, and that has to be
designed well or it turns into another programming language through the back door.

## C · Luau — "no new language"

A library in Luau. `orbe.x + 62` does not add: the metatables build the same
expression tree that Rust builds today with its operators. View and logic in a
single language, with its LSP, its formatter and its sandbox already done.

```lua
local e = escena "Marea"
local orbe  = e:props("orbe",  { x = 360, y = 90 })
local panel = e:props("panel", { w = { 0, sereno }, h = { 0, sereno } })
local fusion, contenido, realce = e:prop("fusion", 0), e:prop("contenido", 0), e:prop("realce", 0, rapido)

local forma_orbe  = elipse { centro = { orbe.x, orbe.y }, radio = 28 + e.respira, estira = "velocidad" }
local forma_panel = caja {
    desde  = { orbe.x + 62, orbe.y - panel.h * 70 / 190 },
    tamano = { panel.w, panel.h }, radio = 24,
    funde  = fusion * suave(0, 40, panel.w),
}
e:cuerpo { color = "#151616", filo = 0.05, sombra = { 0, 10, 30, 0.34 }, forma_orbe, forma_panel }

local ver = caja { centro = { 296, 151 }, tamano = { 168 + realce * 4, 46 + realce * 3 }, radio = 9 }
e:dentro(forma_panel, { opacidad = contenido }, {
    plano { ver, color = mezcla("#9ed6bd", "#bdeed6", realce) },
    texto { "Ver evento", centrado = ver, px = 15 },
})
e:zona("ver", ver, { encima = { [realce] = 1 } })

local function abrir()
    anima(fusion, 96, rapido)
    anima(orbe.x, 140)
    anima(panel.w, 406, sereno, 70)
    -- …
end
e:al("encima", "orbe", 320, abrir)
e:al("pulsa", "ver", function() cerrar(true) end)
```

**For:** it is by far the cheapest —there is no parser to write— and the one an
AI writes best, since it knows Luau from Roblox. Loops and functions for free to
generate scenes (twelve icons in orbit are a `for`).
**Against:** the noise (`{ }`, commas, `local`) and the limits of metatables: `>`,
`and` and a ternary cannot be overloaded, so conditions inside expressions are
`si(mayor(a, b), x, y)`. The errors come out when it runs, not when it is read.
And nothing tells apart at a glance what runs in the renderer from what runs in
the logic, which is exactly the boundary this project wants to make visible.
