# Qué falta para poder hacer «cualquier cosa», como con Quickshell

**Fecha:** 2026-09-19 · **Respuesta corta:** no, ni de lejos. pleamar demuestra una idea; Quickshell hereda quince años de QtQuick. Esta nota es el inventario honesto, y separa **lo que es solo trabajo** de **lo que obliga a decidir arquitectura ahora**.

## Inventario

Leyenda: ✅ hay · 🟡 hay un trozo · ⬜ no hay

### Ventanas y superficies

| Quickshell | pleamar |
| --- | --- |
| `PanelWindow`: anclas, márgenes, zona exclusiva, capa, foco | ✅ la escena declara tamaño, ancla, margen, nivel y reserva; sin foco de teclado |
| Una ventana por monitor (`Variants`), monitores que van y vienen | 🟡 una por monitor y en caliente; todas pintan la misma escena, sin instancia por monitor |
| Escala HiDPI y fraccional | ✅ probada a 2 y a 1,5 |
| Redimensionar la superficie según el contenido | ⬜ |
| Región de entrada dinámica (clic a través de lo transparente) | ✅ sigue a las zonas activas (por cajas) |
| `FloatingWindow`, `PopupWindow` (menús anclados) | ⬜ |
| `WlSessionLock` (pantalla de bloqueo) + PAM | ⬜ |
| Varias ventanas por configuración | ⬜ |

### Dibujo

| Quickshell / QtQuick | pleamar |
| --- | --- |
| Rectángulo redondeado, elipse | ✅ |
| Fundir formas (cuello de agua) | ✅ **y QtQuick no lo tiene** |
| Sombra, luz, filo | ✅ |
| Borde / trazo, degradados | ✅ degradado lineal; falta radial |
| Anillo, arco, segmento | ✅ |
| Trazados libres (`Shape`, SVG) | ⬜ |
| Giro, escala y traslación de un grupo | ✅ afines que se componen; el ratón acierta bajo ellas |
| Opacidad de grupo, recortes anidados | ✅ capa intermedia (hasta 4 a la vez); recortes anidados (hasta 4) |
| **Texto dinámico**: fuentes, ajuste de línea, elipsis, emoji, RTL | ✅ `cosmic-text`; falta texto rico y edición |
| Imágenes (PNG/JPG/SVG), iconos del tema, GIF | 🟡 PNG, JPEG, SVG e iconos por nombre; sin GIF ni búsqueda de tema de verdad |
| Desenfoque, máscaras, `MultiEffect` | ⬜ |
| `ShaderEffect` (shader propio) | ⬜ |
| `Canvas` imperativo | ⬜ |
| Vídeo, `ScreencopyView` (ventanas vivas) | ⬜ |

### Composición

| Quickshell / QtQuick | pleamar |
| --- | --- |
| Anclas, `Row`/`Column`/`Grid`, `RowLayout` con reparto | 🟡 `row`/`column` con hueco, relleno y alineado, animados; sin «ocupa lo que quede» ni salto de línea |
| `Repeater`, `ListView`/`GridView` con virtualización | 🟡 `repeat` de capacidad fija con `show:`; sin modelos ni virtualización |
| Scroll, inercia, `Flickable` | ⬜ |
| `Loader` (no instanciar lo que no se ve) | ⬜ |
| Componentes reutilizables, importar ficheros, singletons | 🟡 `component` con parámetros; sin importar ficheros |
| Recarga en caliente | ✅ y sin perder valores, velocidades, hechos ni textos |

### Entrada

| Quickshell / QtQuick | pleamar |
| --- | --- |
| Encima, fuera, pulsar | ✅ con hit-test exacto sobre la forma |
| Realce sin pasar por la lógica | ✅ **y QtQuick no lo garantiza** |
| Botones del ratón, rueda, arrastrar, mantener pulsado | ✅ |
| Forma del cursor | ✅ `cursor:` |
| Teclado, foco, atajos | 🟡 teclas como sucesos (`on key Escape`); sin modificadores ni atajos globales |
| Entrada de texto, selección, portapapeles, IME | ⬜ |
| Arrastrar y soltar (Marea lo usa) | ⬜ |
| Atajos globales, captura de foco para cerrar al pulsar fuera | ⬜ |

### Datos y sistema

🟡 Hyprland (escritorios y ventana activa, por sus sockets) y `Process` (`run`, `spawn`). ⬜ lo demás: PipeWire, MPRIS, bandeja + sus menús, servidor de notificaciones, UPower, Bluetooth, red, lista de aplicaciones, `Process`, `FileView`, `Socket`, IPC, HTTP, reloj, ajustes persistentes.

### Lenguaje y herramientas

| Quickshell | pleamar |
| --- | --- |
| Lenguaje de escenas | ✅ v0: todo el modelo, componentes, `repeat`, reparto, errores y recarga en caliente |
| Lógica en un lenguaje de script | ✅ Luau en caja de arena, con recarga en caliente |
| Muelles propiedad del render | ✅ **y QtQuick no** |
| Animación por fotogramas, estados, transiciones | ⬜ diseñado, sin hacer |
| Movimiento reducido, i18n, accesibilidad | ⬜ |
| LSP, resaltado, depurador | ⬜ |

## Las cuatro cosas que son arquitectura, no solo trabajo

### 1. El renderer actual no escala — ✅ hecho el 19 sep

El shader recorre **toda** la lista de dibujo en **cada** píxel. Con 12 instrucciones va sobrado; con una lista de veinte notificaciones, cincuenta iconos y texto, no. Hay que pasar a **un quad por elemento** (o por `Cuerpo`), con su caja envolvente: cada píxel ejecuta solo las formas que le tocan, y el fundido se queda dentro de cada `Cuerpo`, que es donde tiene sentido. Es técnica conocida. **Conviene hacerlo antes de amontonar primitivas sobre el intérprete de ahora.**

Buena noticia: capas, gestos y hechos **no dependen de esto**. Solo producen valores de propiedades; les da igual quién pinte.

### 2. El texto es la pieza más grande — ✅ lo básico, hecho el 19 sep

Texto de verdad es: dar forma a los glifos (ligaduras, árabe, emoji en color), fuentes de reserva, ajuste de línea, elipsis, y un atlas de glifos en la GPU que se va llenando. En Rust existe (`cosmic-text` o `parley`, con un atlas para `wgpu`), así que no se parte de cero. **La entrada de texto con IME es otro proyecto encima.**

### 3. Listas, layout y contenido que crece

Sin repetición ni layout no hay lista de notificaciones, ni lanzador, ni centro de control. Hace falta `repite`, componentes con parámetros y un motor de layout (`taffy` da flexbox y grid en Rust). Hay aquí una oportunidad: si **el layout produce destinos y los muelles van hacia ellos**, reordenar una lista se anima solo, como en iOS. En QML eso hay que montarlo a mano.

### 4. Las salidas de emergencia

Nunca vamos a tener todo lo que tiene Qt. Lo que hace que en QML «se pueda cualquier cosa» no es que lo traiga todo, sino que tiene escapatorias: `Canvas`, `ShaderEffect`, plugins en C++. pleamar necesita las suyas, y con tres se cubre casi todo lo raro:

- **`Lienzo`**: un elemento cuya textura la pinta otro —un plugin en su hilo o su proceso— y el render solo coloca y recorta. Cubre de golpe **juegos** (Añicos), **vídeo**, **miniaturas de ventanas** (Atalaya), **una terminal** (k4term) y el editor de vídeo. Es la versión seria de lo que hoy hace el atlas de texto.
- **`Sombreador`**: un fragmento de WGSL propio sobre un elemento. Todo es ya un shader; abrir la puerta es barato.
- **`Proceso`** en la lógica: lanzar órdenes y leer su salida, que es como se hace media shell.

## Por tramos

**Tramo 1 — «una isla o una Marea de verdad».** Lo que usa el 80 % de una shell:
superficies (varios monitores, escala, tamaño según contenido) · renderer por elementos · giro, trazo, degradado, anillo y arco · texto dinámico · imágenes e iconos · repetición, componentes y layout básico · rueda y teclado básico · el lenguaje con recarga en caliente · Luau · servicios: Hyprland, audio, MPRIS, batería, notificaciones, bandeja, `Proceso`, ficheros, IPC.

**Tramo 2 — «todo lo que hacen k4 y Marea hoy».** `Lienzo` y `Sombreador` · listas virtualizadas con scroll · menús emergentes · entrada de texto con IME · arrastrar y soltar · bloqueo de sesión con PAM · desenfoque.

**Tramo 3 — «para que lo use otra gente».** Accesibilidad · i18n y RTL · pintado por CPU como reserva · otras GPU y otros compositores · LSP, formateador y documentación.

El tramo 1 son meses. Los tres, años. Eso no ha cambiado desde el primer día; lo que ha cambiado es que ahora está escrito qué hay en cada uno.

## En qué orden

1. **Capas, gestos y hechos** en el runtime. Pequeño, valida el lenguaje y no depende del renderer.
2. **Renderer por elementos** + giro, trazo, degradado, anillo, arco + superficies de verdad (monitores, escala).
3. **Texto dinámico**, imágenes e iconos.
4. **El parser**, recarga en caliente, componentes, `repite` y layout.
5. **Luau** y el grafo de datos del sistema, servicio a servicio.
6. **`Lienzo`, `Sombreador`**, y el tramo 2.

Al terminar el 4 se podría escribir una barra sencilla entera en el lenguaje. Al terminar el 5, una que sirva para el día a día.
