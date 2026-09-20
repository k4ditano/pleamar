# Referencia del lenguaje — versión 0.1

**Qué es esta nota.** La descripción completa y exacta de lo que el lenguaje acepta. [[pleamar · 09 El lenguaje v0]] es la guía —se lee de corrido, con el porqué de cada cosa—; esto es donde se mira una duda. Está sacada del compilador (`src/lenguaje/`), no de la memoria, y **no se puede desfasar sin que `./probar.sh` lo diga**: sus ejemplos completos se compilan, y su vocabulario (§17) se compara con el que consulta el compilador.

```sh
pleamar --version                  # pleamar 0.1.0 · lenguaje 0.1
pleamar --comprobar escena.plm     # la lee, con lo que importe; dice si está bien, y sale
./probar.sh                        # pruebas/*.plm, escenas/*.plm y los ejemplos de esta nota
```

## 1. Versión

El lenguaje tiene número propio, aparte del programa: **0.1**. El primero cambia cuando algo escrito deja de valer; el segundo, cuando se añade algo. Un fichero puede decir cuál necesita, en su primera línea:

```
language 0.1
```

Si pide un primer número distinto, o un segundo mayor que el que entiende el programa, es un fallo al cargar —`this file asks for language 0.7, and this pleamar understands 0.1`— y no una escena a medias. Sin esa línea, se lee con lo que haya. Mientras el primero sea 0, nada está prometido: es un lenguaje que aún se está haciendo.

## 2. Lo que garantiza

Es un lenguaje **declarativo y que siempre termina**. No hay bucles libres, ni recursión, ni variables que muten: `repeat` y `for` se despliegan al cargar, con un tope. Todo lo que se escribe lo ejecuta el render, solo, a la cadencia de la pantalla; la lógica (Luau, aparte) solo pone hechos, textos, modelos y sucesos. **Todos los nombres se comprueban al cargar**: uno mal escrito es un fallo con su fichero, su línea, su flecha y un «¿querías decir…?», nunca un error en marcha.

## 3. Léxico

| | |
| --- | --- |
| Comentario | `//` hasta el final de la línea |
| Fin de sentencia | un salto de línea o `;`. Dentro de un paréntesis, un salto de línea no acaba nada |
| Nombre | letras, cifras, `_` y `.`; empieza por letra o `_`. Los puntos son parte del nombre: `orb.x`, `note.0.title`. Dentro de un `repeat`, `$i` se sustituye por el número de la vuelta: `hit.$i` |
| Número | `40`, `0.5`, `-3`. Con unidad: `40px` (= 40), `34%` (= 0.34), `138deg` (a radianes) |
| Duración | `320ms`, `14s`. Donde se espera una duración no vale un número sin unidad |
| Color | `#151616` o `#fff` |
| Texto | `"entre comillas"`. Con huecos, ver §11 |
| Símbolos | `{ } ( ) , : ; = ~ + - * / < > <= >= == != .. -> % \|` |

Palabras del lenguaje (no se pueden usar como nombre de algo propio sin confundir a quien lee, aunque el compilador no lo prohíbe): `language import scene library surface permissions model service spring prop pose fact event text image measure let zone body ellipse box arc line path input clip group popup component repeat for row column space layer on every blink wave spin follow look gesture posture`, y dentro de sus sentencias `in max while for after from until at by reach within rest every inset right middle move curve close via as radial radius to change wrap true false and or not`.

## 4. Gramática

En EBNF: `[ x ]` es opcional, `{ x }` cero o más veces, `|` alternativas, `"x"` tal cual. `fin` es un salto de línea o `;`.

```
fichero      = [ "language" numero fin ] { "import" texto fin } ( escena | biblioteca ) ;
escena       = "scene" nombre "{" { sentencia } "}" ;
biblioteca   = "library" nombre [ "strict" ] "{" { let | muelle | componente | frontera | por_dentro } "}" ;
frontera     = permisos | hecho | suceso | texto_vivo | modelo | imagen_decl ;   (* vive bajo el nombre de la biblioteca: `Clock.now` *)
por_dentro   = propiedad | gesto | capa ;                                        (* lo que mueve por dentro; también bajo su nombre *)

sentencia    = declaracion | dibujo | estructura | capa | regla | comportamiento | gesto ;

declaracion  = superficie | permisos | modelo | muelle | propiedad | hecho | suceso
             | texto_vivo | imagen_decl | medida | let | zona ;
superficie   = "surface" [ nombre ] "{" { propiedad_de | sentencia } "}" ;   (* con nombre: una de varias, con lo suyo dentro *)
permisos     = "permissions" "{" { ( "run" | "services" ) ":" texto { "," texto } fin } "}" ;
modelo       = "model" nombre [ "max" numero ] "{" { campo | lista } "}" ;
campo        = nombre ":" tipo [ "=" literal ] fin ;
lista        = "list" nombre [ "max" numero ] ( "{" { campo | lista } "}"        (* fichas dentro de la ficha *)
                                              | "depth" numero ) ;              (* …o como la de fuera, hasta esa hondura: un árbol *)
tipo         = "text" | "number" | "bool" | "image" numero "," numero | enumerado ;
enumerado    = nombre "|" nombre { "|" nombre } ;
muelle       = "spring" nombre "=" numero "," numero ;
propiedad    = ( "prop" | "pose" ) nombre "=" numero [ "~" ref_muelle ] ;
hecho        = "fact" nombre [ ":" ( "number" | "bool" | enumerado ) ] "=" ( numero | "true" | "false" | nombre ) ;
suceso       = "event" nombre [ "->" ] ;
texto_vivo   = "text" nombre "=" texto ;
imagen_decl  = "image" nombre "=" ( "icon" texto | "file" texto | "from" nombre ) "," numero "," numero ;
medida       = "measure" nombre ;
let          = "let" nombre "=" ( expr | color ) ;
zona         = "zone" forma ;
ref_muelle   = nombre | "spring" "(" numero "," numero ")" ;

dibujo       = cuerpo | forma | texto | imagen | campo | recorte | grupo | emergente ;
cuerpo       = "body" "{" { propiedad_de | forma } "}" ;
forma        = ( "ellipse" | "box" | "arc" | "line" ) [ nombre ] "{" { propiedad_de } "}"
             | "path" [ nombre ] "{" { propiedad_de | paso } "}" ;
paso         = "move" punto | "line" punto | "curve" punto "via" punto | "close" ;
texto        = "text" ( texto | nombre | "number" "(" expr [ "," numero [ "," texto ] ] ")" ) "{" { propiedad_de } "}" ;
imagen       = "image" nombre "{" { propiedad_de } "}" ;
campo        = "input" nombre "{" { propiedad_de } "}" ;
recorte      = "clip" [ "inset" numero ] forma ;
grupo        = "group" "{" { propiedad_de | sentencia } "}" ;
emergente    = "popup" nombre "{" { propiedad_de | sentencia } "}" ;

estructura   = componente | copia | hijos | bloque_hueco | repeat | for | reparto | espacio | separador ;
componente   = "component" nombre [ "(" [ parametro { "," parametro } ] ")" ] "{" { propiedad_de | sentencia } "}" ;
parametro    = nombre [ ":" tipo_param ] [ "=" argumento ] ;
tipo_param   = "number" | "bool" | "color" | "text" | "record" | "event" | "image" | "gesture" | "spring" ;
copia        = Nombre [ "(" [ argumentos ] ")" ] [ "{" { propiedad_de | sentencia } "}" ] ;   (* las sentencias son sus hijos *)
hijos        = "children" [ nombre ] [ "{" { propiedad_de } "}" ] ;                          (* solo dentro de un componente *)
bloque_hueco = nombre "{" { sentencia } "}" ;                                                (* en una copia: lo que va al hueco de ese nombre *)
separador    = "between" [ nombre ] "{" { propiedad_de | sentencia } "}" ;                   (* solo dentro de un reparto; con varias cosas, lleva `size:` *)
argumentos   = argumento { "," argumento } { "," nombre ":" argumento }
             | nombre ":" argumento { "," nombre ":" argumento } ;
argumento    = expr | color | texto | nombre ;
repeat       = "repeat" nombre "in" entero ".." entero "{" { sentencia } "}" ;
for          = "for" nombre "in" nombre [ "from" expr ] "{" { sentencia } "}" ;
reparto      = ( "row" | "column" ) [ nombre ] [ "~" ref_muelle ] "{" { propiedad_de | sentencia } "}" ;
espacio      = "space" expr ;

capa         = "layer" nombre [ "~" ref_muelle ] "{" { reclamacion } "}" ;
reclamacion  = nombre [ cuando ] [ "{" { transicion } "}" ] ;
cuando       = "while" expr | "for" duracion "after" sucesos | "from" sucesos "until" sucesos ;
sucesos      = nombre { "," nombre } ;
transicion   = nombre ":" expr [ "~" ref_muelle ] [ "after" duracion ] fin ;

regla        = "on" disparador [ "while" expr ] "{" { efecto } "}"
             | "every" duracion [ ".." duracion ] [ "while" expr ] "{" { efecto } "}" ;
disparador   = "press" [ "right" | "middle" ] zona_ref | "release" zona_ref | "scroll" zona_ref
             | "drag" zona_ref | "hold" zona_ref "for" duracion
             | "enter" zona_ref | "leave" zona_ref
             | ( "hover" | "away" ) zona_ref "for" duracion
             | "idle" "for" duracion
             | "key" tecla | "submit" nombre | "focus" | "blur" | "drop" zona_ref
             | nombre ;                                  (* un suceso *)
tecla        = nombre { "+" nombre } ;                  (* Escape · Ctrl+k · Super+Alt+s *)
efecto       = transicion | nombre "=" expr | "toggle" nombre
             | "emit" nombre [ "(" expr ")" ] | "impulse" nombre numero
             | "play" nombre | "focus" nombre | "blur" ;

comportamiento = "blink" nombre "every" duracion ".." duracion "for" duracion
             | "wave" nombre "=" expr "at" numero
             | "spin" nombre "by" expr
             | "follow" nombre "=" expr
             | "look" nombre "," nombre "at" punto "reach" numero "," numero "within" numero [ "rest" punto ] ;

gesto        = "gesture" nombre ( "ambient" | "reflex" | "asked" | "state" ) "{" { fotograma } "}"
             | "posture" nombre "while" expr "{" { fotograma } "}" ;
fotograma    = duracion { curva | "hold" duracion | "emit" nombre } [ "{" { nombre ":" expr fin } "}" ] ;
curva        = "linear" | "in_quad" | "out_quad" | "in_cubic" | "out_cubic" | "in_out_sine" | "out_back" ;

propiedad_de = nombre ":" valor { "," valor } fin ;      (* cuáles valen, según el elemento: §8 *)
punto        = expr "," expr ;

expr         = o ;
o            = y { "or" y } ;
y            = no { "and" no } ;
no           = "not" no | suma [ ( "<" | ">" | "<=" | ">=" | "==" | "!=" ) suma ] ;
suma         = producto { ( "+" | "-" ) producto } ;
producto     = unario { ( "*" | "/" ) unario } ;
unario       = "-" unario | "(" expr ")" | numero | duracion | "true" | "false"
             | nombre | funcion "(" [ expr { "," expr } ] ")" ;
color        = "#" hex | nombre | "mix" "(" color "," color "," expr ")" ;
```

`Nombre` en `copia` es el de un componente ya declarado: por convención, con mayúscula, que es lo que lo distingue a la vista de una sentencia del lenguaje.

## 5. Ficheros, orden y nombres

**Un fichero es una escena o una biblioteca.** Una escena se abre; una biblioteca se importa. `import "ruta.plm"` va antes de `scene` o `library`, y la ruta es relativa **al fichero que importa**. Una biblioteca importada por dos caminos se lee una vez; un círculo es un fallo que dice su camino. Una biblioteca solo declara: `let`, `spring` y `component`. Lo importado se comporta como si estuviera escrito al principio de la escena.

**`library Nombre strict { … }`**: sus componentes solo pueden leer lo que piden por parámetro, lo que ellos declaran, lo de su biblioteca (y lo que esta importe), y los nombres que siempre existen. Leer un hecho, un color o un suceso de la escena sin pedirlo es un fallo al cargar —`'Nosy' belongs to a `strict` library and reads 'secret', which is the scene's, without asking for it`—: así una biblioteca de otro no depende de cómo se llamen las cosas en tu escena, ni las toca. Sin `strict`, un componente ve todo lo de quien lo usa, que es lo cómodo para las bibliotecas propias.

**Un plugin es una biblioteca con su lógica al lado**: `reloj.plm` y `reloj.luau`. Puede declarar, además, su propia frontera —`fact`, `text`, `model`, `event`— y sus `permissions`:

```
library Clock strict {
    permissions { run: "date" }           // los de SU lógica, no los de la escena
    text now = "--:--"
    fact seconds = false
    event tapped ->
    component Clock(tone: color = ink) {
        row face { padding: 9; fill: coal; corner: 14;  text now { size: 14; color: tone } }
        on press face { emit tapped }
    }
}
```

- Su frontera **vive bajo el nombre de la biblioteca**: dentro se escribe `now`; desde la escena, `Clock.now`. Dos plugins pueden tener cada uno su `count`, y ninguno pisa el de la escena.
- Su lógica corre en **su propio estado de Luau**, y ahí `text.now` es `Clock.now`: no puede nombrar —ni leer, ni escribir, ni emitir— nada que no sea suyo, ni de la escena ni de otro plugin. No oye el teclado, ni el ratón, ni los sucesos de nadie más; no pide gestos ni mueve el cursor de escribir.
- Lo que toque del sistema lo dicen **sus** `permissions`. Los de la escena no le valen, y los suyos no le valen a la escena. Al arrancar se imprime: `lógica · plugin «Clock» · permisos · órdenes: date`.
- Una escena puede no tener lógica propia y usar plugins que sí. Guardar el `.plm` o el `.luau` de un plugin recarga en caliente, como lo demás.

Pedir permisos sin tener un `.luau` al lado es un fallo: no hay quien los use.

**Los permisos de un plugin los aprueba quien lo usa.** Declararlos no es tenerlos: `pleamar --aprobar escena.plm` enseña lo que pide cada plugin de esa escena y pregunta. Lo aprobado se guarda fuera del plugin, con la huella de su lógica y de lo que pedía: si cambia cualquiera de las dos, vuelve a estar sin aprobar. **Sin aprobar, un plugin corre sin ningún permiso**, y sus errores dicen por qué y cómo aprobarlo. Un intérprete (`sh`, `python`…) sale marcado: es pedirlo todo. La escena que uno abre no pasa por esto: abrirla ya es decidir.

Una biblioteca puede traer también **lo que mueve por dentro** —`prop`, `pose`, `gesture`, `posture`, `layer`— e **imágenes** (`image logo = file "logo.png", 16, 16`: la ruta es relativa al fichero que la escribe, así que la imagen va con ella). Todo bajo su nombre, como su frontera. Lo que no puede es pintar fuera de un componente, ni tener reglas sueltas: eso es de la escena.

**La escena le habla a un plugin emitiendo un suceso suyo** (`on press button { emit Face.cheer }`), que oyen los componentes del plugin (`on cheer { … }`) y su lógica (`on("cheer", …)`); y puede leer y poner los hechos de su frontera (`Face.happy = false`): la escena es la dueña. Un plugin no tiene superficie propia: si algo necesita la suya, es una escena.

**El fichero se lee en cuatro vueltas** —declaraciones; `let` y capas; dibujo; reglas—, así que el orden de lo escrito es el que le convenga a quien lee: una regla puede ir antes que la forma que nombra, y un `prop` al final. Dos excepciones: un `let` tiene que ir antes de quien lo usa, y **se pinta en el orden en que se escribe** (y de las zonas, la que se declara después queda encima).

**Todos los nombres son globales**, salvo dentro de un componente o de una vuelta de `repeat` o `for`: ahí lo que se declara es propio de esa copia (dos copias de `Note` tienen cada una su `lit` y su zona `hit`), y primero se busca lo de dentro —parámetros, `let` del componente— y luego lo de fuera. Un `let` de la escena con el nombre de uno importado lo pisa: así se cambia un tono. Dos componentes con el mismo nombre no conviven.

**Nombres que siempre existen**, y se leen como hechos: `screen.width`, `screen.height` (lo que mide la superficie de verdad), y durante una regla, lo del ratón: `pointer.x`, `pointer.y` (en la superficie), `local.x`, `local.y` (dentro de la zona), `drag.dx`, `drag.dy` (desde que se pulsó), `wheel` (muescas; positivo, hacia arriba). Y el suceso `demo`, que dispara `--demo`.

## 6. Declaraciones

| Sentencia | Qué declara |
| --- | --- |
| `surface { … }` · `surface panel { …; …dibujo… }` | Las ventanas que pide. Ver abajo |
| `permissions { run: "date"; services: "audio", "audio.*" }` | Lo que la lógica puede tocar del sistema. Sin declarar, nada. **Escuchar no es mandar**: `"audio"` deja saber el volumen (`sys.watch`, `sys.ask`); para cambiarlo hace falta `"audio.volume"`, o `"audio.*"` |
| `service clock as now { time: text; hour: number }` | Un servicio del sistema, por su nombre. Lo que cuente rellena `now.time` y `now.hour` **sin una línea de lógica**. Ver abajo |
| `model rows max 14 { label: text; enabled: bool = true; depth: number }` | Una lista de fichas que pone la lógica. `max`: cuántas caben (1 a 256; 16 si no se dice). Crea `rows.count`, `rows.total` y, por ficha, `rows.K.campo` |
| `prop orb.x = 360 ~lively` | Una propiedad animada: un muelle. Sin `~`, `lively` |
| `pose eyes = 14` | Una propiedad de la pose: la que un gesto lleva de la mano |
| `fact open = false` · `fact tries: number = 3` · `fact mode: low \| normal \| critical = normal` | Algo que es verdad un rato. Lo ponen la lógica y las reglas. Ver **Tipos**, abajo |
| `event confirmed` · `event view_event ->` | Algo que ocurre. Con `->`, además le llega a la lógica |
| `text notice.title = "Reunión"` | Un texto vivo: lo cambia la lógica, o un `input` |
| `image fox = icon "firefox", 48, 48` | Una imagen, y a qué tamaño lógico se pinta como mucho. `icon "nombre"`, `file "ruta"`, o `from un_texto`: la que ese texto diga (un nombre de icono, o una ruta si empieza por `/`) |
| `measure label` | Crea `label.width` y `label.height`, que rellena el texto que lleve `measure: label` |
| `let panel.x = orb.x + 62` · `let mint = #9ed6bd` | Un nombre para una expresión, o para un color |
| `spring bouncy = 170, 12` | Un muelle propio: rigidez, freno. De casa: `lively`, `calm`, `quick`, `slow`, `gentle`, `pose`. En línea: `~spring(170, 12)` |
| `zone box whole { at: …; size: …; active: expr }` | Una zona que no se pinta |

**Varias ventanas en un proceso.** `surface { … }` sin nombre es la de la escena, y dibuja lo que hay suelto. Con nombre, `surface panel { … }` es una de varias y **lleva dentro lo que dibuja**:

```
surface { size: full, 40; anchor: top; screens: all }
box knob { … }
on press knob { toggle open }

surface panel {
    size: 300, 160;  anchor: top_right;  margin: 48, 12, 0, 0;  level: overlay
    open: open                                   // está mientras ese hecho sea verdad
    body { … };  box close { … }
    on press close { open = false }
}
```

Todas **comparten propiedades, hechos, modelos y reglas**: una barra y su panel se hablan con un hecho, sin dar la vuelta por el sistema y sin saber una de otra. Por dentro cada una mira a un trozo distinto del mismo plano, igual que una emergente. Una superficie cerrada no se ve y no se puede pulsar.

**Tipos.** Para el render todo son números; los tipos son para quien escribe y para quien habla con la escena desde fuera. Un hecho es un número, un sí o no (`bool`; sin tipo, lo es el que nace `true` o `false`) o un **enumerado**: `fact mode: low | normal | critical = normal`. Los nombres de sus valores valen en cualquier expresión (`mode == critical`, `mode = low` en una regla) y son su posición: `low` es 0. **Un enumerado se compara con sus valores, y el compilador lo comprueba**: `mode == fast`, si `fast` es de otro, es un fallo que dice cuáles valen; y con un enumerado no se hacen cuentas (`mode + 1` no significa nada; con un sí o no, sí: `r.separator * 21`). El mismo nombre puede estar en dos enumerados: comparado con su hecho, cada uno es el suyo; suelto, si significa números distintos, es un fallo que pide la forma larga, `mode.normal`, que vale siempre. En un hueco de un texto, un enumerado se enseña por su nombre: `"modo: {mode}"` → `modo: critical`. La lógica los lee y los escribe como lo que son —`fact.open` es `true`, `fact.mode` es `"critical"`—, y `--decir` también.

Los campos de un modelo tienen esos tipos y dos más: **`image w, h`** —el nombre de un icono o una ruta, y la imagen que eso diga: `image r.icon { … }` sin declarar nada más— y **`list`**, fichas dentro de la ficha:

```
model menu max 8 {
    label: text
    icon: image 20, 20
    kind: plain | checked | danger = plain
    list items max 6 { label: text; enabled: bool = true }
}
for m in menu { …  for it in m.items { text it.label { … } } }
```

Una lista de dentro se recorre con `for it in m.items`, y tiene su `m.items.count` y su `m.items.total`. **`list children max 6 depth 2`**, sin bloque, son fichas como la de fuera, unas dentro de otras hasta esa hondura (de 1 a 6): un árbol, como el menú de una aplicación. Se recorre con tantos `for` como niveles se quieran enseñar. Todo se despliega al cargar: 8 × 6 son 48 fichas, y el tope entre todas las listas de un modelo es 4096.

**Servicios, sin lógica.** `service` pide algo del sistema por su nombre y dice qué campos quiere de los que ese servicio trae. Cada campo es un hecho o un texto normal —del tipo que se le ponga— con el nombre delante, y se rellena solo cuando el sistema cuenta algo:

```
permissions { services: "clock" }                      // sin permiso no se monta
service clock { time: text = "--:--"; date: text }     // sin `as`: clock.time, clock.date
service clock.seconds as tick { second: number }       // con `as`: tick.second
text clock.time { size: 14; color: ink }
```

Qué trae cada servicio está en el vocabulario (§17), y **pedirle lo que no tiene es un fallo al cargar**, con su «did you mean…?». Lo que no venga en un aviso se queda como estaba. Los que traen listas —`apps`, `tray`, `notifications`, `workspaces`— no se piden así: eso es un modelo, y lo reparte la lógica con `sys.watch`.

Los permisos son los de la escena, y son los mismos de `sys.watch`: `services: "clock"`. Sin ellos la escena carga igual, dice por qué en la consola y ese campo se queda como nació. `clock` avisa al cambiar el minuto y `clock.seconds` cada segundo; ninguno de los dos pregunta la hora a nadie ni despierta a la máquina para mirar si ya toca.

**Ficheros.** Una escena tiene **su propia carpeta**, y de ahí no sale: sin rutas, sin `..`, como `require`. Se usa desde la lógica, con permiso `services: "files"` para leer y `"files.write"` para escribir:

| | |
| --- | --- |
| `sys.ask("files.read", "settings.json")` | lo que diga, o `nil` si no está |
| `sys.ask("files.read", n, "json")` | eso mismo, ya como tabla. Si el fichero está roto, es un fallo con su sitio, no una tabla a medias |
| `sys.ask("files.exists", n)` · `sys.ask("files.list")` · `sys.ask("files.folder")` | si está · lo que hay · dónde |
| `sys.call("files.write", n, texto)` · `sys.call("files.remove", n)` | escribir (entero o nada: primero al lado, luego en su sitio) · borrar |
| `sys.call("files.write", n, { tone = 2 })` | una tabla se guarda como JSON, con sus saltos de línea: lo que se guarda también se lee a mano |
| `sys.watch("files:settings.txt", f)` | avisa cuando ese fichero cambie, también si lo toca otro |

Un plugin tiene la suya, bajo su nombre: lo que guarde no lo ve la escena, ni al revés.

**Una por monitor.** `screens: each [max N]` repite la superficie en cada monitor (4 como mucho, si no se dice otra cosa), y **cada copia tiene lo suyo**: sus propiedades, sus zonas y sus reglas. Dentro:

| | |
| --- | --- |
| `$screen` | su número, para interpolar en un nombre: `mon.$screen.active`, como `$i` en un `repeat` |
| `screen.index` | lo mismo, como número |
| `screen.name` | el nombre de **su** monitor, que pone el render (`HDMI-A-1`) |
| `screen.width` · `screen.height` | lo que mide **su** monitor |
| `screens.count` | cuántos monitores están enseñando algo |

Con la superficie de la escena (la que no lleva nombre), lo que se repite es el dibujo suelto. `--pantalla A,B` reparte las copias entre esos monitores, que es como se ensayan dos sin tener dos.

```
surface { size: full, 44; anchor: top; screens: each }
model mon max 4 { active: number = 1; title: text }     // una ficha por monitor, de la lógica
text mon.$screen.title { … }
repeat i in 1..10 { Desk(i, mon.$screen.active) { show: ws.$i.there } }
```

**`surface`**: `size: ancho, alto` (`full` como ancho es todo el monitor) · `anchor:` `top` `bottom` `left` `right` `top_left` `top_right` `bottom_left` `bottom_right` `center` · `margin: n` o `arriba, derecha, abajo, izquierda` · `level:` `background` `bottom` `top` `overlay` · `reserve: n` (el sitio que las ventanas le dejan) · `screens: all` o `"HDMI-A-1", "DP-3"` · `keyboard:` `none` `on_demand` `exclusive`, y con `while expr` solo lo pide mientras sea verdad.

## 7. Expresiones

Son números. **Verdad es más de 0.5**; `true` es 1 y `false` es 0. Las evalúa el render, en cada frame que haga falta.

De menos a más fuerza: `or` · `and` · `not` · `< > <= >= == !=` (no se encadenan: `a < b < c` no vale) · `+ -` · `* /` · `-` delante. `==` es «iguales a menos de una milésima»: son números con coma, y un muelle nunca llega del todo.

| Función | |
| --- | --- |
| `min(a, b)` `max(a, b)` `abs(x)` | |
| `floor(x)` `ceil(x)` | al entero de abajo o al de arriba |
| `clamp(x, a, b)` | x, entre a y b |
| `smooth(a, b, x)` | de 0 a 1 mientras x va de a a b, con entrada y salida suaves |
| `mix(a, b, t)` | entre a y b. También entre dos colores |
| `if(cond, a, b)` | |
| `vel(prop)` | la velocidad de un muelle, que solo el render conoce |

Vale como nombre: un `let`, un `prop`, un `fact`, una medida (`label.width`), lo que ocupa un reparto con nombre y cuántos hijos tiene a la vista (`list.width`, `list.height`, `list.count`: se pueden leer también antes de donde se declara), el campo numérico de una ficha (`r.depth`, `r.index`, `rows.count`), y la presencia de una reclamación (`shape.rec`: 1 mientras gana).

## 8. Dibujo

Cada elemento acepta estas propiedades y ninguna más; otra es un fallo, con sugerencia.

| Elemento | Propiedades |
| --- | --- |
| `ellipse` | `at` · `radius` · `scale: sx, sy` |
| `box` | `at: cx, cy` o `from: x, y` · `size: w, h` · `corner` |
| `arc` (como «∩») | `at` · `radius` · `span` · `width` |
| `line` | `from` · `to` · `width` |
| `path` | `at` (de dónde cuelgan sus puntos) · `size: w, h` (lo que ocupa en un reparto), y dentro sus pasos: `move x, y` (una vez, la primera) · `line x, y` · `curve x, y via cx, cy` · `close`. Cerrado se rellena; abierto o con `stroke`, es una línea |
| …y todas las formas | `color` · `opacity` · `rotate` · `stroke` (solo el contorno) · `blend` (dentro de un `body`: cuánto se funde con lo anterior) · `active` · `cursor` · `show` |
| `body` | `color` o `gradient` (abajo) · `rim` · `light: cantidad, desde_y, alto` · `shadow: dx, dy, difusa, alfa` · `border: grosor, #color` · `opacity` · `show`, y dentro sus formas, fundidas en una silueta |
| `text` | `at` · `anchor` · `width` · `lines` · `size` · `weight` · `color` · `opacity` · `align:` `left` `center` `right` · `line_height` · `family` · `measure` · `show` |
| `image` | `at` · `size` · `opacity` · `tint` · `show` |
| `input` | `at` · `width` · `size` · `weight` · `color` · `opacity` · `family` · `placeholder` · `selection` · `show` |
| `group` | `pivot` · `rotate` · `scale: s` o `sx, sy` · `move: dx, dy` · `opacity` (se funden como una sola cosa) · `size` (para quien lo reparta) · `show` |
| `popup` | `at` (dentro de la superficie) · `size` · `open:` un hecho |
| `row` `column` | `at` · `anchor` · `gap` · `padding` · `align:` `start` `center` `end` · `fill` · `corner` · `opacity` · `cursor` · `show` · `view: w, h` · `step` · `content` · `wrap: n` |

`anchor` de un texto: `left` `center` `right` y `top` `center` `bottom`, uno o los dos (`anchor: left center`). De un reparto: `left` `center` `right` y `top` `middle` `bottom` —sin ancla, `at` es su esquina de arriba a la izquierda—. `cursor:` `default` `pointer` `text` `grab` `grabbing`.

Un **camino** es una línea quebrada o curva. `close` la cierra, y entonces se rellena —también cóncava, y también consigo misma cruzada—; sin cerrar, o con `stroke`, es una línea de ese grosor con las puntas redondas. `curve` es una Bézier cuadrática, y se parte en tantos tramos como largo sea el desvío. Por dentro es la misma distancia con signo que las demás formas: se funde con `blend`, y tiene sombra, filo, luz y borde como cualquiera.

```
path {
    at: 26, 40;  stroke: 2.5;  color: mint
    move 0, 56;  line 26, 36 + breath * 5;  line 52, 44;  line 78, 12
}
```

Un camino lleva **un solo trazo** (un `move`, el primero) y hasta 64 puntos ya aplanados; para varios, varios `path`. En un reparto hay que decirle lo que ocupa con `size:`, porque su caja no se sabe hasta evaluarlo.

**Listas más largas que lo que se despliega.** Un modelo despliega sus `max` fichas al cargar, y ese es el tope (256). Para una lista de miles, la escena declara solo la **ventana** —lo que se ve y un poco más— y dice cuánto mide la lista entera:

| | |
| --- | --- |
| `content: total * 34` | en un reparto con `view:`, el largo de verdad: el desplazamiento va sobre él, no sobre lo desplegado |
| `for r in rows from first` | la copia 0 es la ficha `first` de la lista de verdad, así que `r.index` es el número que le toca |
| `move: 0, first * 34` | pone las copias en su sitio de la lista entera |
| `list.scroll` | además de leerse, **se escribe** como cualquier propiedad: `on press top { list.scroll: 0 ~calm }` |
| `on change floor(list.scroll / 34) { emit slid(list.scroll) }` | así se entera la lógica de que hay que mandarle otro trozo, venga el movimiento de donde venga |

Un reparto con `view:` **se arrastra** sin declarar nada: al pulsarlo se apunta por dónde iba y sigue al ratón, con su muelle. Y se agarra **por dentro**: arrastrar no es cosa de la zona de más arriba, sino de cualquiera que estuviera debajo al pulsar, como la rueda. Así una lista se mueve agarrándola por una de sus filas.

**`wrap: 5`** convierte un reparto en una **rejilla**: cinco por línea y a la siguiente. La celda mide lo que el hijo más grande, y **lo que no se ve no deja hueco**, así que los demás se recolocan, con el muelle del reparto si lo lleva. Es lo que en Quickshell es un `Flow`.

`escenas/lista-larga` son cinco mil filas en dieciséis copias: 0,49 ms por frame, y los mismos dieciséis grupos y diecinueve zonas las haya que haya.

`clip [inset n] forma` recorta todo lo que venga después, hasta el final de su `group`. Hasta cuatro anidados recortan por su forma; los de más afuera, por su caja.

**Degradados.** En un `body`, `gradient:` toma de dónde a dónde va y luego sus colores, separados por comas. Cada color puede decir **dónde cae** (`sand 40%`); los que no lo digan se reparten por igual. De dos a ocho.

```
gradient: 0, 0, 0, 44, mint, coal                        // de un punto a otro
gradient: 0, 0 to 0, 44, mint, sand 30%, #e86a9a, coal   // lo mismo, con paradas
gradient: radial 100, 160 radius 60, ink, mint 40%, coal // desde un centro hacia fuera
```

**Una forma con nombre es una zona** si alguna regla la nombra, si lleva `active`, o si se declaró con `zone`. Un nombre puesto solo para leerse mejor no para el clic. Una zona hereda las transformaciones de los grupos donde esté, y **lo que no está —un `show:` falso, una ficha que no existe— no es zona**. Un `row` o `column` con nombre también lo es: su caja entera, debajo de las de sus hijos.

## 9. Repartos

`row` y `column` colocan a sus hijos uno detrás de otro: el sitio de cada uno es una expresión, así que si uno crece o desaparece, los demás se mueven. Con `~muelle` en la cabecera, viajan a su sitio en vez de saltar. Cada hijo tiene que saber cuánto ocupa: una forma con `size` o `radius`, un texto (se mide solo), una imagen, un `group` o un componente con `size:`, otro reparto, o `space n`. `show: expr` en un hijo decide si está: ocupa y se ve, o ni lo uno ni lo otro.

**Listas más largas que su hueco.** `view: w, h` dice lo que se ve; lo de dentro puede ser más largo y **se corre con la rueda**, recortado y sin pasarse de lo que hay. `step:` es cuánto por muesca (60 por defecto), y el muelle del reparto es con el que viaja. Además de `width`, `height` y `count`, publica `list.content` —cuánto hay— y `list.scroll` —por dónde va—, que es lo que hace falta para pintar una barrita al lado:

```
column list { at: 16, 16;  view: 250, 208;  gap: 6
    for r in rows { Row(r) }
}
box { from: 276, 16 + list.scroll / max(list.content, 1) * 208
      size: 4, 208 * 208 / max(list.content, 208);  corner: 2;  color: ink;  opacity: 35% }
```

Con `view:`, hacia fuera ocupa lo que se ve, no lo que lleva dentro.

`between { box { size: 272, 1; color: ink } }` pone eso **entre cada dos hijos que estén**: si uno desaparece, su raya también, y nunca queda una al principio ni al final. **No abre otro hueco**: va centrada en el `gap` que ya hay entre sus vecinos, así que la distancia entre dos hijos es el `gap` más lo que ocupe ella. Con una sola cosa dentro, esa cosa dice cuánto ocupa; con varias, el `between` hace de grupo y lo dice él (`between { size: 10, 12; … }`). `between i { … }` le da su posición —1 tras el primer hijo, 2 tras el segundo…—, para que la primera pueda ser distinta: `opacity: if(i == 1, 50%, 12%)`. Un reparto con nombre publica, además de `lista.width` y `lista.height`, **`lista.count`**: cuántos hijos están ahora mismo. Los tres se pueden leer también antes de donde se declara.

## 10. Componentes, `repeat`, `for`

`component Nombre(parámetros) { size: w, h; … }` declara; `Nombre(argumentos)` pone una copia. `size:` dice cuánto ocupa, para quien lo reparta. Lo que una copia declara —`prop`, formas con nombre, reglas— es suyo.

**Un componente dice qué necesita.** Cada parámetro puede llevar tipo y valor por defecto: `component Row(r: record, chosen: event, tone: color = mint, height: number = 30)`.

| Tipo | Lo que se le pasa | Dentro |
| --- | --- | --- |
| `number` | una expresión | vale en cualquier expresión |
| `color` | `#fff`, un `let` de color, `mix(…)` | donde va un color |
| `text` | `"entre comillas"` (con huecos, si quiere) o el nombre de un texto vivo | `text nombre { … }`, y en un hueco: `"{nombre}"` |
| `record` | una ficha: la de un `for`, o `rows.0` | `r.campo`, `r.index` |
| `event` | el nombre de un suceso de la escena | `emit nombre(…)` y `on nombre { … }` hablan de **ese** suceso |
| `image` | el nombre de una imagen | `image nombre { … }` |
| `bool` | una expresión (`true`, `false`, `count > 3`) | vale en cualquier expresión |
| `gesture` | el nombre de un gesto | `play nombre` |
| `spring` | el nombre de un muelle, o `spring(170, 12)` | `~nombre` |

Así un componente de biblioteca no da por hecho que la escena tenga un suceso que se llame de cierta manera: lo pide. Los argumentos van **por posición y luego, si se quiere, por nombre** (`Row(r, choose, height: 40)`); desde el primero con nombre, todos con nombre. Los que tienen valor por defecto se pueden omitir, y van al final. Lo que falte, sobre, se repita o no sea del tipo es un fallo donde se usa, que enseña la firma entera: `'Row' is missing 'chosen' (an event): it is Row(r: record, chosen: event, tone: color = …)`. El valor por defecto se lee donde se usa el componente, así que `= mint` es el `mint` de esa escena.

Sin tipo (`component Dot(tone)`), el parámetro es lo que parezca el argumento: es como se escribían antes, y sigue valiendo.

**Un hueco para hijos: `children`.** Lo que una copia trae dentro de su bloque —además de propiedades como `show:`— va donde su componente diga `children`:

```
component Card(title: text) {
    size: 300, 40 + inside.height            // mide lo que mida lo que le metan
    body { color: #1b1c1c; box { from: 0, 0; size: 300, 40 + inside.height; corner: 12 } }
    text "{upper(title)}" { at: 12, 18; anchor: left center; size: 11; color: ink }
    column inside { at: 12, 32; gap: 4;  children }
}

Card("Avisos") {
    text title { size: 14; color: ink }      // el `title` de la escena, no el parámetro de Card
    repeat i in 0..2 { text "fila {i}" { size: 13; color: ink } }
}
```

Dentro de un `row` o `column`, cada hijo ocupa su sitio en el reparto (y un `repeat` o un `for` de fuera se despliega como los de dentro); suelto, `children { move: x, y }` es un grupo. **Los hijos se leen con los nombres de quien los escribió**: un componente ni ve ni pisa lo que le meten, y un parámetro suyo no tapa nada de fuera. **Varios huecos, con nombre.** Un componente tiene como mucho un `children` sin nombre y los que quiera con él: `children header`, `children footer`. En la copia, un bloque con ese nombre es lo que va a ese hueco —**aunque la escena tenga un componente que se llame igual: dentro de la copia, gana el hueco**—, y lo demás va al que no lo tiene. Un hueco no se puede llamar como una palabra del lenguaje.

```
Panel {
    header { text "Avisos" { … } }
    for n in notes { text n.title { … } }        // al `children` sin nombre
    footer { box ok { … };  box no { … } }
}
```

Un hueco que no existe es un fallo que dice cuáles hay (`'Panel' has no slot called 'heder': it has header, footer. Did you mean 'header'?`), y meterle algo a un componente sin `children` también: no un silencio.

Un fallo **dentro** de un componente dice también desde dónde se usó —`(inside 'Badge', used at escena.plm:6)`—, porque a menudo lo que está mal es lo que se le pasó.

`repeat i in 0..5 { … }` despliega cinco vueltas al cargar (512 como mucho); dentro, `i` es un número y `$i` se sustituye en los nombres.

`for r in rows { … }` despliega una vuelta por ficha que quepa en el modelo. Dentro, `r.campo` es el campo de esa ficha —un texto donde va un texto vivo, un número en cualquier expresión— y `r.index` su posición desde 0. **Cada vuelta solo existe si la lista llega hasta ahí.** Vale dentro de un reparto, suelto, y dentro de una `popup`.

## 11. Textos con huecos

Dentro de un texto entre comillas que sea el contenido de un `text` o el argumento de un componente:

| | |
| --- | --- |
| `{nombre}` | un texto vivo, o el campo `text` de una ficha |
| `{expr}` · `{expr, n}` | una expresión, con n decimales (0 si no se dice) |
| `{upper(nombre)}` · `{lower(nombre)}` | ese texto, en mayúsculas o minúsculas |
| `{? … }` | un tramo que solo está si ninguno de los textos de dentro está vacío |
| `{{` · `}}` | una llave de verdad |

Los nombres de un hueco se resuelven donde está escrita la cadena, no donde se use.

## 12. Capas

`layer nombre [~muelle] { reclamaciones }`. **Gana la primera reclamación que se cumple**, de arriba abajo; cuando deja de cumplirse se ve la siguiente, sola. Una reclamación es `nombre` seguido de cuándo —`while expr`, `for 700ms after suceso, otro`, `from suceso until suceso`, o nada (por defecto)— y, si quiere, un bloque con su coreografía: adónde va cada propiedad, con qué muelle y con qué retraso. El destino es una expresión que se evalúa cuando le llega la hora. `capa.reclamacion` vale 1 mientras gana, y se puede leer en cualquier expresión.

## 13. Reglas

`on disparador { efectos }` y `every 2s..7s [while expr] { efectos }`.

| Disparador | Cuándo |
| --- | --- |
| `press zona` · `press right zona` · `press middle zona` | se pulsa. Usar `right` en alguna regla quita la salida de emergencia del prototipo (el botón derecho cierra) |
| `release zona` | se suelta lo que se pulsó ahí, esté donde esté ya el ratón |
| `hold zona for 500ms` | lleva ese rato pulsada |
| `enter zona` · `leave zona` | el ratón entra o sale |
| `hover zona for 320ms` · `away zona for 420ms` | lleva ese rato encima; estuvo encima y lleva ese rato fuera |
| `scroll zona` | la rueda, sobre cualquier zona que tenga debajo. Se lee en `wheel` |
| `drag zona` | se mueve con el botón puesto; sigue aunque se salga, hasta soltar. `local.x`, `drag.dx`. Como la rueda, vale para cualquier zona que estuviera debajo al pulsar, no solo la de arriba: así una lista se arrastra agarrándola por una fila |
| `change expr` | esa cuenta deja de valer lo que valía. Al nacer no cuenta: se dispara al cambiar |
| `key Escape` · `key Ctrl+k` | una tecla; la superficie tiene que pedir teclado. Modificadores: `Ctrl+` `Alt+` `Super+` |
| `submit campo` | Intro dentro de ese `input` |
| `focus` · `blur` | la superficie gana o pierde el teclado |
| `drop zona` | sueltan sobre ella algo arrastrado desde otra aplicación |
| `idle for 14s` | nadie toca nada en ese rato |
| `nombre_de_suceso` | ocurre ese suceso: lo emite la lógica, otra regla, un gesto, o viene de fuera |

**Cualquier regla admite `while expr`** al final de su cabecera: se mira en el momento de dispararse. En `idle` y `every` decide además si el rato cuenta.

| Efecto | |
| --- | --- |
| `prop: valor ~muelle after 70ms` | esa propiedad va hacia ahí |
| `hecho = expr` | se evalúa al dispararse |
| `toggle hecho` | |
| `emit suceso` · `emit suceso(expr)` | con una carga, que le llega a la lógica |
| `impulse prop -620` | un empujón: suma a la velocidad del muelle |
| `play gesto` | lo pide; se le concederá o no, según su clase |
| `focus campo` · `blur` | le da el cursor de escribir a un `input`, o lo quita |

## 14. Lo que lleva sola

| | |
| --- | --- |
| `blink eyelid every 2.4s..6s for 170ms` | de 1 a 0 y vuelta, de vez en cuando |
| `wave breath = amplitud at 1.7` | amplitud · sin(1.7 t) |
| `spin angle by 0.9` | += 0.9 por segundo |
| `follow chip.w = label.width + 32` | persigue a la expresión, con su muelle |
| `look gx, gy at cx, cy reach 5, 3.2 within 140 rest rx, ry` | dos propiedades que tiran hacia el ratón |

## 15. Gestos

Un gesto es una línea de tiempo sobre las propiedades de la pose (`pose`). `gesture nombre clase { fotogramas }`; clases, de menos a más: `ambient` < posturas < `reflex` < `asked` < `state`. **Un gesto solo corta a otro de su clase o inferior.** Un fotograma es una duración, y si quiere una curva, `hold 60ms` (aguanta ahí) y `emit suceso`; su bloque dice adónde va cada propiedad, y lo que no nombre vuelve a su base. Sin bloque, es la vuelta a la base. `posture nombre while expr { … }` se repite sola mientras sea verdad.

## 16. Ejemplos comprobados

Estos se compilan con `./probar.sh`.

Una lista que viene de datos, con un componente, textos con huecos, y una regla por fila:

```plm
language 0.1
scene Reference1 {
    surface { size: 320, 220; anchor: top; margin: 40 }
    let ink = #f5f7f5
    model notes max 4 { app: text; title: text; body: text; urgency: number = 1 }
    event opened ->

    text "{notes.total} avisos" { at: 20, 20; anchor: left center; size: 15; weight: 600; color: ink }
    column list ~calm { at: 20, 40; gap: 6
        for n in notes { Note(n) }
    }
    component Note(n) {
        size: 280, 40
        prop lit = 0 ~quick
        box hit { from: 0, 0; size: 280, 40; corner: 10; color: mix(#1b1c1c, #2b2d2d, lit); cursor: pointer }
        box { from: 0, 8; size: 3, 24; corner: 1.5; color: mix(#9ed6bd, #e8776a, n.urgency == 2) }
        text "{upper(n.app)}  {n.title}{? · {n.body}}" { at: 12, 20; anchor: left center; size: 13; width: 260; lines: 1; color: ink }
        on enter hit { lit: 1 ~quick }
        on leave hit { lit: 0 ~quick }
        on press hit { emit opened(n.index) }
    }
}
```

Una capa que decide, una emergente, y el teclado solo mientras hace falta:

```plm
language 0.1
scene Reference2 {
    surface { size: 300, 60; anchor: top; keyboard: on_demand while open }
    fact open = false
    fact busy = false
    event saved
    prop glow = 0
    prop tint = 0

    layer mood ~quick {
        done    for 600ms after saved { tint: 1 ~lively }
        working while busy            { tint: 0.5 ~calm }
        idle                          { tint: 0 ~slow }
    }
    box button { at: 150, 30; size: 120, 34; corner: 17; color: mix(#2e2f2f, #9ed6bd, max(tint, glow)); cursor: pointer }
    text "Guardar" { at: 150, 30; anchor: center; size: 14; color: #f5f7f5; opacity: 60% + mood.idle * 40% }

    popup confirm { at: 90, 56; size: 120, 16 + choices.height; open: open
        body { color: #1b1c1c; box { from: 0, 0; size: 120, 16 + choices.height; corner: 10 } }
        column choices { at: 8, 8; gap: 4
            box yes { size: 104, 28; corner: 7; color: #9ed6bd }
            box no  { size: 104, 28; corner: 7; color: #3a3b3b }
        }
    }

    on press button  { toggle open }
    on press yes     { open = false; emit saved }
    on press no      { open = false }
    on key Escape    { open = false }
    on hover button for 200ms while not open { glow: 1 ~quick }
    on leave button  { glow: 0 ~quick }
}
```

Una pose, un gesto y lo que lleva solo: la cara es el ejemplo, pero sirve para cualquier cosa que tenga estados y se mueva entre ellos.

```plm
language 0.1
scene Reference3 {
    surface { size: 200, 200; anchor: center }
    pose eyes = 14
    pose look.y = 0
    prop lid = 1
    prop breath = 0
    prop gaze.x = 0 ~gentle
    prop gaze.y = 0 ~gentle
    fact searching = false
    event shutter

    body { color: #151616; rim: 5%; shadow: 0, 8, 24, 30%
        ellipse face { at: 100, 100 + breath; radius: 60 } }
    group {
        clip inset 3 ellipse { at: 100, 100; radius: 60 }
        box { at: 82 + gaze.x, 96 + gaze.y + look.y; size: 9, eyes * lid; corner: 4.5; color: #f5f7f5 }
        box { at: 118 + gaze.x, 96 + gaze.y + look.y; size: 9, eyes * lid; corner: 4.5; color: #f5f7f5 }
    }

    blink lid every 2.4s..6s for 170ms
    wave breath = 1.2 at 1.7
    look gaze.x, gaze.y at 100, 100 reach 6, 4 within 160

    gesture nod reflex {
        130ms out_quad              { look.y: 4; eyes: 10 }
        170ms out_back hold 60ms emit shutter { eyes: 15 }
        160ms
    }
    posture scanning while searching {
        400ms in_out_sine { look.y: -3 }
        400ms in_out_sine { look.y: 3 }
    }
    on press face { play nod }
}
```

## 17. El vocabulario, tal como lo consulta el compilador

Esto es la salida de `pleamar --gramatica`, copiada. No es una segunda lista: son las mismas tablas (`src/lenguaje/vocabulario.rs`) que el compilador consulta para aceptar o rechazar una palabra. `./probar.sh` compara este bloque con lo que imprime el programa —si alguien añade una palabra y no la apunta aquí, falla— y comprueba además que **cada palabra aparece en alguna prueba**.

```vocabulario
language: 0.1
statements: surface permissions model service spring prop pose fact event text image measure let zone body ellipse box arc line path input clip group popup component children repeat for row column space between layer on every blink wave spin follow look gesture posture
library: let spring component permissions fact text model service event image prop pose gesture posture layer
properties.surface: size anchor margin level reserve screens keyboard open
properties.permissions: run services
properties.shape: rotate stroke color opacity blend active show cursor
properties.ellipse: at radius scale
properties.box: at from size corner
properties.arc: at radius span width
properties.line: from to width
properties.path: at size
properties.body: color gradient rim light shadow border opacity show
properties.text: at anchor width size weight color opacity lines align line_height family measure show
properties.image: at size opacity tint show
properties.input: at width size weight color opacity family placeholder selection show
properties.group: pivot rotate scale move opacity size show
properties.popup: at size open
properties.children: move
properties.layout: at anchor gap padding align fill corner show opacity cursor view step content wrap
functions: min max abs floor ceil clamp smooth mix if vel
text_functions: upper lower
triggers: press release scroll drag hold enter leave hover away idle key submit focus blur drop change
effects: toggle emit impulse play focus blur
curves: linear in_quad out_quad in_cubic out_cubic in_out_sine out_back
frame: hold emit
classes: ambient reflex asked state
field_types: text number bool image
fact_types: number bool
model: list
path: move line curve close
documented: surface permissions model service spring prop pose fact event text image measure let zone body ellipse box arc line path input clip group popup component children repeat for row column space between layer on every blink wave spin follow look gesture posture import scene library language
services: clock clock.seconds audio battery network media window
services.clock: hour minute second day month year weekday time date
services.clock.seconds: hour minute second day month year weekday time date
services.audio: volume muted
services.battery: present percent charging
services.network: online kind name strength
services.media: playing title artist album player
services.window: title class
parameter_types: number bool color text record event image gesture spring
springs: lively calm quick slow gentle pose
units: px % deg ms s
cursors: default pointer text grab grabbing
surface.anchor: top bottom left right top_left top_right bottom_left bottom_right center
surface.level: background bottom top overlay
surface.keyboard: none on_demand exclusive
text.align: left center right
layout.align: start center end
```

`properties.shape` son las comunes a `ellipse`, `box`, `arc` y `line`; `properties.layout`, las de `row` y `column`.

## 17.1. En el editor

`pleamar --lsp` es un servidor de lenguaje por la entrada y la salida, con **este mismo compilador** detrás: los fallos con su sitio mientras se escribe, qué palabras valen aquí, y qué significa la que está bajo el cursor. `pleamar --resaltado vim` y `--resaltado vscode` escriben el fichero de sintaxis, sacado del vocabulario de arriba. Los dos, y cómo se instalan, están en `editor/`.

## 18. Lo que esta versión no tiene

Para no buscarlo aquí: `import … as`, salto de línea en los repartos, horas y plurales en los huecos, y escribir en el campo de una ficha desde una regla. Todo está, con su plan, en [[pleamar · 08 Limitaciones conocidas]].
