# La lógica, en Luau

**Estado:** implementado (`src/logica_luau.rs`). Si al lado de `marea.plm` hay un `marea.luau`, esa es su lógica. Se recarga sola al guardarla, igual que la escena.

Ejemplos: `escenas/marea.luau` (avisos que llegan) y `escenas/bandeja.luau` (una lista de datos que crece y encoge).

## Qué puede hacer, y qué no

La lógica **no anima, no pinta y no sabe de coordenadas**. Vive en su propio hilo; el render no la espera nunca. Solo cruza la frontera:

```lua
fact.open = true                          -- decir qué es verdad  (se lee como número: 1 o 0)
text["notice.title"] = "Reunión en 5 min" -- decir qué pone un texto vivo
emit("confirmed")                         -- que ha pasado algo
play("joy")                               -- pedir un gesto (la escena lo concederá o no, según su clase)

on("view_event", function(n) … end)       -- un suceso que la escena deja salir (`event x ->`), con su carga
on("press:view", …)  on("enter:orb", …)  on("leave:orb", …)
on("layer:card", function(claim) … end)   -- una capa cambió de manos
on("fact:open", function(v) … end)        -- una REGLA de la escena cambió un hecho
on("demo", …)                             -- el tic de `--demo`

local t = every(1000, function() … end)   -- temporizadores, en milisegundos
after(500, function() … end)
cancel(t)

run("date", { "+%H:%M" }, function(out, code) … end)   -- una orden del sistema; contesta al acabar
log("lo que sea", 42)
busy(600)                                 -- trabajo de mentira, para ver que al render le da igual
```

Un nombre mal escrito es un error al momento, con sugerencia: `la escena no tiene ningún hecho «opne». ¿Querías decir «open»?`

## La caja de arena

- **Sin `io` ni `os.execute`**: es el modo `sandbox` de Luau. La única puerta al sistema es `run`.
- **Tope de memoria**: 64 MB.
- **Los segundos contados**: un manejador que lleve más de 2 s sin acabar se corta, y la lógica sigue viva. Probado con un `while true do end`.
- **Un error no tumba nada**: se dice por consola y el resto de manejadores siguen.

## Sucesos con carga

En la escena, `emit opened(i)`; en la lógica, `on("opened", function(i) … end)`. Antes hacía falta un suceso por índice.

## Listas que vienen de datos

Todavía no hay copias que nazcan en marcha (G12), pero con la lógica poniendo textos y un hecho `count`, una lista de capacidad fija se comporta como una de verdad. `bandeja.luau` tiene una tabla de avisos: cada 1,6 s llega uno arriba, pulsar uno lo despacha, y la columna de la escena recoloca a los demás con su muelle.

## Multiplataforma

Luau se compila de sus fuentes en C++, y `mlua` lo soporta en los tres sistemas. Aquí no hay compilador cruzado, así que `./portable.sh` comprueba Windows y macOS **sin** la característica `luau` y lo dice. El pegamento es Rust sin nada de sistema.
