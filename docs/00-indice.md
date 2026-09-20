# pleamar — Índice

**Qué es:** un runtime nuevo para escritorio —lo que sería «un Quickshell mejor»— y el lenguaje con el que se escriben sus escenas. Nació el 19 sep 2026 de una conversación sobre por qué QtQuick no puede animar mientras la lógica trabaja.

**Dónde está:** `~/Proyectos/pleamar` · [github.com/k4ditano/pleamar](https://github.com/k4ditano/pleamar) (privado). Estas notas tienen copia en `docs/` del repo; **si difieren, manda la de Edinot**.

## Las notas

| Nota | Para qué |
| --- | --- |
| [[pleamar · 01 Por qué existe]] | El problema, la idea y lo que se midió |
| [[pleamar · 02 Cómo funciona hoy]] | El runtime que ya corre: hilos, escena, shader |
| [[pleamar · 03 El lenguaje - borrador 0]] | El boceto en castellano de antes de implementarlo. Histórico; manda la 09 |
| [[pleamar · 04 Prueba - cabe Marea]] | El `ExpressionController` de Marea contra el lenguaje |
| [[pleamar · 05 Decisiones y preguntas abiertas]] | Lo decidido, con su porqué, y lo que falta por decidir |
| [[pleamar · 06 Bocetos A-B-C]] | Las tres sintaxis que se compararon. Histórico. |
| [[pleamar · 07 Qué falta para igualar a Quickshell]] | El inventario honesto, por tramos, y el orden de trabajo |
| [[pleamar · 09 El lenguaje v0]] | **La guía del lenguaje**: se lee de corrido, con el porqué de cada cosa |
| [[pleamar · 10 La lógica en Luau]] | La frontera vista desde la lógica: qué puede hacer un `.luau`, y su caja de arena |
| [[pleamar · 11 Referencia del lenguaje 0.1]] | **La referencia**: léxico, gramática en EBNF, cada elemento con lo que acepta, y el número de versión. Sus ejemplos se compilan con `./probar.sh` |
| [[pleamar · 08 Limitaciones conocidas]] | Todo lo que está a medias, con su gravedad, para ir tachándolo |

## Estado (19 sep 2026)

- ✅ **El render anima solo.** Con la lógica bloqueada 600 ms, 38 frames a ~17 ms. El mismo ensayo en QML: un hueco de 600 ms.
- ✅ **Una escena son datos.** Propiedades, expresiones, lista de dibujo, comportamientos y zonas. Dos escenas (Marea, isla) sobre el mismo render.
- ✅ **Se eligió la dirección del lenguaje:** árbol (A) + lo declarativo de (B), Luau solo para la lógica.
- ✅ **Se probó contra la Marea real.** Cabe, con tres piezas que el boceto no tenía: hechos y sucesos, capas, gestos.
- ✅ **Capas, gestos, hechos, sucesos y reglas en el runtime** (todavía escritos en Rust). Probado con la cara de Marea: el disco rojo aguanta buscar y confirmar mientras graba; al dejar de grabar, la lupa sale sola. Y Marea se abre, realza su botón y se cierra con la lógica bloqueada 5 s.
- ✅ **Frontera de plataforma** (`src/plataforma/`): todo lo de Wayland vive detrás, y `./portable.sh` comprueba que el núcleo compila para Linux, Windows y macOS. Pasa.
- ✅ **Texto de verdad e imágenes**: `cosmic-text` (ligaduras, árabe, japonés, emoji, líneas, puntos suspensivos, alineado), glifos a la escala del monitor, **textos vivos** que cambia la lógica, SVG/PNG/JPEG, iconos por nombre y teñido. Marea y la isla ya no usan mapas de bits.
- ✅ **Medir texto desde una expresión** y **taller de texto** en su propio hilo: ni un frame lento después del primero.
- ✅ **El lenguaje, v0**: un fichero `.plm` → escena. Tokenizador, parser, comprobación de nombres con «¿querías decir…?», errores con línea y flecha, y **recarga en caliente** que no pierde nada. `escenas/marea.plm` y `escenas/cara.plm` corren sin una línea de Rust.
- ✅ **Componentes, `repeat` y reparto** (`row`/`column`): cada copia con sus propios nombres, zonas y reglas; cada hijo va a su hueco con un muelle. `escenas/bandeja.plm` es una lista de avisos que crece y encoge. Y el fichero se lee en cuatro vueltas: el orden es el de quien lee.
- ✅ **La lógica en Luau**: un `.luau` al lado de la escena, en caja de arena (sin `io`, 64 MB, corte a los 2 s), con hechos, textos, sucesos con carga, temporizadores y órdenes del sistema. Se recarga en caliente. `bandeja.luau` mueve una lista desde datos.
- ✅ **La primera barra de verdad** (`escenas/barra.plm` + `barra.luau`): los escritorios de Hyprland, la ventana activa, la hora y el volumen, a todo el ancho del monitor. Con ella llegaron los **servicios del sistema** detrás de `plataforma/` (`sys.watch`, `sys.call`), `spawn`/`kill`, `size: full`, `screen.width`, `anchor:` en los repartos y `text number(…)`.
- ✅ **La entrada que faltaba**: rueda, arrastrar (con coordenadas locales, y sigue fuera de la zona), soltar, mantener pulsado, botón derecho y del medio, teclas, y forma del cursor. La barra ya cambia el volumen: rueda, arrastre y silencio.
- ✅ **Escribir, soltar y mandar desde fuera**: `input` (un campo que edita el render: cursor, selección, portapapeles), `on submit`, `on focus` / `on blur`, `on key Ctrl+k`, `on drop` para lo que sueltan otras aplicaciones, teclado solo mientras hace falta, y `pleamar --decir lanzador "emit toggle"` para que un atajo del compositor abra una escena. Con ello, **un lanzador** (`escenas/lanzador.plm` + `.luau`) sobre el servicio `apps`.
- ✅ **Renderer por elementos**: un quad por elemento con su caja. 600 formas cuestan 0,42 ms por frame frente a 3,66 ms del intérprete por píxel; 2000, 0,57 ms. Con él llegaron **aro/trazo, arco, segmento, giro** (propio y heredado), **degradado lineal, borde**, recortes anidados (hasta cuatro) y fundido real entre elementos. Y después, cerrando sus limitaciones: **transformaciones afines que se componen** (giro, escala, traslación), **opacidad de grupo** con capa intermedia, caja exacta para texturas giradas y zonas de ratón bajo transformaciones. Todo a la vista en `--escena muestrario`.
- ✅ **Superficies de verdad**: una por monitor —y por los que se enchufen después—, escala fraccional (probada a 2 y a 1,5 en un monitor virtual), y una región de entrada que sigue a las zonas: lo transparente deja pasar el clic. La escena declara su superficie (tamaño, ancla, nivel, margen, reserva).
- ✅ **Servicios del sistema de verdad**: `audio`, `battery`, `network` y `media` (MPRIS), con los mismos nombres y tablas en todos los sistemas; la barra ya no llama a `pactl`. Lo que se lanza muere con el programa aunque lo maten. Y cuatro limitaciones menos: medidas de un reparto legibles antes de declararlo, **imágenes que elige la lógica** (`image … = from un_texto`: el lanzador tiene iconos), `--decir … "get nombre"` que contesta, y la repetición de tecla del sistema.
- ✅ **Permisos para la lógica** (`permissions { run: …; services: … }`: sin declarar, nada) y **un render sin topes**: los almacenes de la tarjeta crecen solos (6000 formas a 17 ms).
- ✅ **Notificaciones y bandeja**: `notifications` (pleamar es el servidor; `escenas/bandeja` enseña las de verdad) y `tray` (con los iconos de Telegram y ChatGPT pintados desde sus píxeles). Con esto B1 queda cerrado: audio, batería, red, música, aplicaciones, escritorios, ventana, notificaciones y bandeja.
- ✅ **Superficies emergentes** (`popup`: un `xdg_popup` que enseña otro trozo de la misma escena) y **los menús de la bandeja** (`sys.ask("tray.menu", …)`, con submenús). `escenas/iconos` es la bandeja entera: los iconos de Telegram y ChatGPT, y sus menús de verdad.
- ✅ **Modelos y `for`**: datos con forma que cruzan la frontera (`model rows max 14 { label: text; enabled: bool = true }`, `for r in rows { Row(r) }`, `model.rows = lista`). Las tres escenas con listas, reescritas sin un hueco a mano. Y **la primera batería de pruebas del lenguaje** (`pruebas/`, `./probar.sh`), que vigila también los mensajes de error.
- ✅ **Textos con huecos**: `text "{n.title}{? · {n.body}}"`, con expresiones (`{volume * 100} %`), `upper`/`lower`, tramos que desaparecen si su texto está vacío, y fallos que señalan el carácter exacto dentro de la cadena.
- ✅ **Varios ficheros**: `library` e `import`, con fallos que dicen en qué fichero y recarga en caliente de lo importado. `escenas/comun/` tiene la paleta y la fila de menú. La batería de pruebas del lenguaje va por 22.
- ✅ **El lenguaje, por escrito**: la nota 11 es su referencia, versión **0.1** (`language 0.1`, `pleamar --version`), sacada del compilador y con sus ejemplos compilados en cada `./probar.sh` y su vocabulario comparado con el que consulta el compilador (`pleamar --gramatica`): 62 comprobaciones. Escribirla destapó dos huecos, ya cerrados: no había `==`, y `while` solo valía en dos reglas.
- ✅ **Componentes que dicen qué necesitan**: parámetros con tipo y valor por defecto, argumentos por nombre, y sucesos como parámetro (`MenuRow(r, chosen: choose)`). Un fallo dentro de un componente dice desde dónde se usó. Con **huecos para hijos** (`children`, con nombre si hacen falta varios), `between` y `lista.count` en los repartos, y **bibliotecas `strict`**, que solo leen lo que piden.
- ✅ **Tipos**: hechos `bool` y enumerados (`fact mode: low | normal | critical`), con sus nombres en las expresiones, en los huecos y en la lógica; y campos `image` y `list` (fichas dentro de fichas).
- ✅ **Plugins**: una biblioteca con su `.luau` al lado, con su frontera bajo su nombre (`Clock.now`), su propio estado de Luau —que solo ve lo suyo— y sus propios permisos. Probado con un plugin hostil. `escenas/plugins/reloj` es el primero.
- ⬜ que el usuario apruebe los permisos (U1), ventanas normales y bloqueo de sesión, IME, ventanas normales y menús: ver la nota 07.

## Siguiente paso

**En rojo otra vez, y a propósito:** con plugins, que nadie apruebe los permisos (U1) ya importa. **El tramo del lenguaje está hecho**: modelos y `for`, textos con huecos, `import`, referencia con versión y vocabulario vigilado, componentes con contrato (tipos, huecos, `strict`), tipos (hechos `bool` y enumerados, campos `image` y `list`) y plugins. Lo que sigue: **aprobar permisos** (U1), un hilo por plugin (U3), comprobar tipos en las expresiones (G18), copias que nazcan en marcha (G12), ventanas y bloqueo (S7), una instancia por monitor (S2), IME (E7). **Por probar con manos de verdad**: emergentes y menú de la bandeja (S11), el lanzador con su bind (E8), la rueda (E1), notificaciones y bandeja con k4 parado (B12, B14).
