# Tres bocetos para el lenguaje

Los tres describen el mismo trozo de Marea —el cuerpo con su cuello de agua, los
ojos, el botón que se realza y abrir/cerrar— y los tres compilan a lo mismo: la
`Escena` de `src/escena.rs`. Lo que cambia es dónde vive cada cosa y cuánto
puede hacer el render sin la lógica.

Las palabras clave están en castellano porque el proyecto lo está; si esto sale
de casa, es una decisión aparte.

## A · Árbol — «como QML, pero los muelles son del lenguaje»

Elementos anidados con propiedades. `~` convierte una propiedad en un muelle. Un
hijo se coloca respecto a su padre y se recorta a él. La lógica va aparte, en
Luau, y es imperativa como hoy.

```
Cuerpo {
    color: #151616;  filo: 5%
    sombra: 0 10, difusa 30, 34%

    Elipse orbe {
        x: 360 ~vivo;  y: 90 ~vivo
        radio: 28 + respira
        estira: velocidad                   // se deforma con su propia velocidad

        Ojos { mirada: puntero, hasta 5 3.2;  parpadea: cada 2.4..6s;  cerrados: sueño }
        respira: onda(1.7) * sueño * 1.3
        sueño: 0 ~lento
    }

    Caja panel {
        izquierda: orbe.x + 62;  arriba: orbe.y - alto * 70/190
        ancho: 0 ~sereno;  alto: 0 ~sereno;  radio: 24
        funde: 0 ~rápido                    // cuello de agua con lo anterior
        contenido: 0 ~sereno                // opacidad de los hijos

        Texto { en: 26 78;  "Reunión en 5 min";  20px medio }
        Caja ver {
            centro: 296 151;  tamaño: 168+realce*4  46+realce*3;  radio: 9
            color: mezcla(#9ed6bd, #bdeed6, realce)
            realce: 0 ~rápido
            encima { realce: 1 }            // lo ejecuta el render, sin lógica
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
    al.encima(orbe, 320, abrir)             -- 320 ms encima
    al.pulsa(ver, function() cerrar(true) end)
}
```

**A favor:** se lee como lo que es, un dibujo con partes; las coordenadas
relativas y el recorte por padre salen solos, y de ahí a un layout hay un paso.
Quien venga de QML lo entiende a la primera.
**En contra:** la coreografía sigue siendo código imperativo. Abrir depende de
que la lógica esté viva para declararla: si está atascada, Marea no se abre.

## B · Estados — «la escena es una máquina, y la lleva el render»

Las formas se declaran planas. Lo central son los **estados** —qué valor tiene
cada propiedad en cada uno—, las **transiciones** —con qué muelle y qué retraso
se va de uno a otro— y las **reglas** —qué hace cambiar de estado—. Todo eso lo
ejecuta el render. A la lógica solo le llegan hechos con significado.

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

**A favor:** Marea se abre, se cierra, se duerme y responde al ratón con la
lógica muerta. La lógica queda en lo que de verdad es suyo: poner el texto,
decidir que ha llegado un aviso, reaccionar a `"ver-evento"`. Y es comprobable
antes de ejecutar: un estado que nombra una propiedad que no existe es un error
con número de línea.
**En contra:** una máquina de estados plana no aguanta a la Marea real. Su
`ExpressionController` son 2100 líneas de «quién gana»: dormida, grabando, aviso
urgente, Remanso… Haría falta estados por capas con prioridad, y eso hay que
diseñarlo bien o se vuelve otro lenguaje de programación por la puerta de atrás.

## C · Luau — «ningún lenguaje nuevo»

Una biblioteca en Luau. `orbe.x + 62` no suma: las metatablas construyen el
mismo árbol de expresiones que hoy construye Rust con sus operadores. Vista y
lógica en un solo lenguaje, con su LSP, su formateador y su sandbox ya hechos.

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

**A favor:** es lo más barato con diferencia —no hay parser que escribir— y lo
que mejor escribe una IA, que Luau lo conoce de Roblox. Bucles y funciones
gratis para generar escenas (doce iconos en órbita son un `for`).
**En contra:** el ruido (`{ }`, comas, `local`) y los límites de las metatablas:
no se puede sobrecargar `>`, `and` ni un ternario, así que las condiciones en
expresiones son `si(mayor(a, b), x, y)`. Los errores salen al ejecutar, no al
leer. Y nada distingue a simple vista lo que corre en el render de lo que corre
en la lógica, que es justo la frontera que este proyecto quiere hacer visible.
