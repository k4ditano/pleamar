# How it works today

What already runs in `~/Proyectos/pleamar`. Rust, `wgpu` 30, `cosmic-text` for the text, `resvg` and `image` for the images; and, on Linux only, `smithay-client-toolkit` 0.21 (layer-shell).

## Three threads

| Thread | Does | Does not do |
| --- | --- | --- |
| **Platform** (`plataforma/wayland.rs`) | One layer-shell surface per monitor (and for the ones plugged in later), its scale, and the mouse | Nothing else |
| **Logic** (`logica.rs`) | Runs a `Guion`: receives named events, reports facts and events, asks for gestures | Does not animate. Knows nothing about coordinates. Blocks on purpose for the trial |
| **Renderer** (`render.rs`) | Owner of the springs and of the clock. Evaluates, paints and reports | Does not know what it is painting |

The mouse goes **to the renderer**, which is the one that knows what is underneath; the logic gets it already as `Entra("view")` or `Pulsa("orb")`.

## A scene is data (`escena.rs`)

- **Properties** with a name. Each one is a spring (position, velocity, target). If the scene is replaced, the ones with the same name survive.
- Pure **expressions** (`Expr`): constants, properties, velocities, `+ − × ÷`, `min`, `max`, `abs`, `suave`. In Rust they are written with overloaded operators: `orbe_x + 62.0`.
- **Draw list**, in order:

| Instruction | What it does |
| --- | --- |
| `Grupo` | Starts a body; optionally with a shadow |
| `Forma` | Adds an ellipse or a box, blended into what came before (`fusion` = radius of the smooth minimum) |
| `Relleno` | Paints the accumulated body: shadow, color, light and rim |
| `Recorte` | What follows is clipped to this shape |
| `Plano` | A loose shape in flat color |
| `Textura` | A piece of the atlas, placed on screen |

- **Behaviors** the renderer runs by itself: `Parpadeo`, `Onda`, `Avance` (a hand going round), `Mirada`.
- **Zones**: a named shape and an `activa` condition.
- **Facts and events**: the boundary. The logic sends `Hecho("recording", true)`, `Suceso("confirmed")` or `Gesto("nod")`, and nothing else. Facts are read in expressions (`Expr::H`), with `y`, `o`, `no`, `mayor`.
- **Layers**: claims in order of priority — `while <expr>`, `N ms after <event>`, `from … until`, or the default one. The first one that holds wins. Each claim has a **presence** (a property that goes to 1 when it wins), to paint according to who is in charge, and it can **pin** properties with their spring and their delay: that is a choreography.
- **Gestures**: keyframes with a duration, a curve (`OutBack`, `InQuad`…) and a hold, over the **pose** properties. Whatever a keyframe does not name goes back to its base; when it ends, the springs pick the pose up. **Classes**: `Estado > Pedido > Reflejo > Postura > Ambiente`; one only cuts off another of its own class or lower. A **posture** repeats on its own while something is true.
- **Rules**: `Entra`, `Sale`, `Pulsa`, `Encima{durante}`, `Fuera{durante}`, `Quieto{durante}`, `Cada{a..b}`, `Al(suceso)` → effects (`Animar`, `Hecho`, `Alternar`, `Suceso`, `Impulso`, `Gesto`). **The renderer runs all of it.**
- **Reduced motion** (`--movimiento-reducido`): the springs settle, each gesture shows the keyframe furthest from the base held still, and what carries itself goes quiet — a `blink` stays open, a `wave` rests at the middle of its travel and a `spin` stops where it was. Nothing is left going round on its own.

## The renderer: one quad per element (`render.rs`, `forma.wgsl`, `formas.rs`)

Every frame, the renderer walks the draw list and **composes** it into two tables it uploads to the GPU:

- **shapes**, with their expressions already evaluated (16 numbers each);
- **elements** (48 numbers): a *body* —one or several blended shapes, with their paint, border, light and shadow— or a piece of the atlas. Each one carries its **bounding box**, computed on the CPU from the same geometry, widened by the shadow and the blend and cut down by its clips.

It is painted with **a single** instanced call: one quad per element, in order, with premultiplied blending. A pixel only runs the shapes of the element that covers it. What is invisible (`alfa` 0, empty boxes) does not even produce a quad.

| Primitive | Notes |
| --- | --- |
| `Elipse`, `Caja` | as before |
| `Arco` | like an "∩"; `apertura` is a half-angle in radians |
| `Segmento` | a line with round ends |
| `.trazo(grosor)` | the outline only: a circle becomes a **ring** |
| `.girada(ángulo)` | about its center; positive is clockwise |
| `Transformar` | rotates, scales and moves everything that follows around a pivot. It is a **stack that composes**: affine matrices multiplied together, so a rotation inside a group that scales inside another that rotates does what one expects. Mouse zones can live under the same transforms (`zona_bajo`) |
| `Opacidad` | everything inside is painted **apart, on a layer**, and blended as one single thing. Without a layer, blending piece by piece lets what is behind show through what is in front. It only spends a layer while it is half-blended (neither 0 nor 1); up to four at a time |
| `Recorte` | now a **stack** (up to four): a child is clipped to its parent and to its grandparent |
| `Pintura::Lineal`, `borde` | a gradient between two points; an inner border on the body |

`formas.rs` holds the geometry once for three uses: encoding it for the GPU, knowing what is under the mouse, and computing boxes.

**Measured** (RTX 2060, 720×300, `--escena enjambre --sin-vsync`, ms per frame):

| shapes | per-pixel interpreter (before) | per element |
| --- | --- | --- |
| 12 | 0.27 | 0.25 |
| 60 | 0.53 | 0.28 |
| 200 | 1.37 | 0.30 |
| 600 | 3.66 | 0.42 |
| 2000 | did not fit (1024 ceiling) | 0.57 |

Mind what this table says: **the old interpreter was not doing as badly as feared** at this surface size; 600 shapes still fit in a frame with room to spare. Where it would have broken is full screen (9.6 times the pixels) or on an integrated GPU. The new one, on top of that, has no ceiling any more.

## Text and images (`texto.rs`)

Everything that ends up as a piece of the atlas. Three pure-Rust crates that exist on all three systems: `cosmic-text`, `image` and `resvg`.

- **`Instr::Texto`**: fixed content or **live** (a named text the logic changes with `c.texto("aviso.título", …)`), a point and an `ancla` —which part of the text lands on it: (0.5, 0.5) centers it—, an optional width for breaking into lines, and an `Estilo` (family, size, weight, color, line height, alignment, maximum lines with an ellipsis).
- The **typesetter** shapes the text —ligatures, right to left, fallback fonts, color emoji— and keeps the layout: as long as text, style and width do not change, it is not redone. Each glyph is painted once, **at the scale of the finest render surface**, into a 2048² atlas it shares with the images. A letter is a mask the shader tints; an emoji brings its own color.
- Each glyph is one more element: it is clipped, transformed and blended like everything else. The text origin is rounded to real pixels so that it does not come out soft.
- **`Instr::Imagen`**: a path or an icon by name (`Fuente::Icono("firefox")`; finding it is the platform's business). SVGs are painted at the exact size for the scale; with `tinte`, the shape is painted in a single color: what a symbolic icon wants.
- **The workshop.** Shaping, painting glyphs and decoding images happen on its own thread, which is also the one that reads the system fonts at startup. The renderer **asks and does not wait**: while a layout is on its way it shows the last one there was in that spot, and whatever is already in the scene is ordered before the window even exists. First frame at ~170 ms, and none slow after it.
- **Measuring.** A `Texto` with `mide` leaves its width and its height in two read-only properties. With `Comportamiento::Sigue` —a property that chases an expression with its spring— a box grows with its label.
- There is a permanent snitch: any frame longer than 2.4 periods is printed to the console with its time.

## The platform boundary (`src/plataforma/`)

The only part of the program that knows what Wayland is. A platform puts up the surfaces a scene asks for and hands them to the renderer as render surfaces; it tells it about the mouse, the scale and the monitors coming and going; it gives it a `Ventana` with one method (`region_de_entrada`); and it knows how to find an icon by its name. Today there is only `wayland.rs`. On any other system the core compiles and says it does not know how to put up windows yet. **`./portable.sh` checks it against Linux, Windows and macOS**, and it is the guard that nothing belonging to one system slips out of here.

## Services (`plataforma/mod.rs`, `plataforma/hyprland.rs`)

What happens in the system reaches the logic through `plataforma::servicio(nombre, avisar)`: a thread that listens and reports with a `Valor` —a small JSON: null, yes/no, number, text, list, map— that Luau receives as a table. `plataforma::orden(nombre, args)` is the way back. Today Hyprland answers, spoken to over its two sockets with `std` and nothing else: one to ask and to command (`j/workspaces`, `dispatch …`), another over which it tells what is happening. A system without that service says it does not have it.

Besides Hyprland: `plataforma/sistema.rs` (audio, battery and network: one thread per service, which only reports if something changes) and `plataforma/mpris.rs` (what is playing, over D-Bus with `zbus`, listening to signals). Two services do not listen but **are** the server, both with `zbus`: `plataforma/avisos.rs` (the notifications: whoever holds the name `org.freedesktop.Notifications` receives them, and if it already belongs to someone else the service says it is not there) and `plataforma/bandeja.rs` (the tray: watcher if nobody else is, host to whatever watcher there is if there is one). Whatever has to be said to the bus outside the call that caused it —a signal, an expiry— is done by a separate thread, over a channel.

Everything that is spawned goes through `plataforma::morir_con_el_padre`: on Linux, `PR_SET_PDEATHSIG`.

## Surfaces (`plataforma/wayland.rs`, `gpu.rs`)

The scene declares the surface it wants: size in **logical** pixels, anchor, margin, level (background, bottom, top, overlay), how much room it reserves, and on which screens (`Todas` or a list). The Wayland thread puts one on each monitor it applies to, removes them when the monitor goes away and puts them up when one arrives. Each one reaches the renderer as a **render surface** when the compositor configures it.

- **Scale.** With `wp_fractional_scale` and `wp_viewporter`: the logical size does not change, and the render surface is painted with `size × scale` real pixels. The shader divides the position by the scale and smooths the edges in real pixels, so at scale 2 nothing comes out soft —except the text, which is a bitmap—. Without those protocols, it falls back to the old integer scale.
- **Pace.** Only one render surface —the one on the fastest monitor— waits for the screen; the rest present without blocking. If they all waited, a 60 Hz monitor would hold back a 165 Hz one.
- **Input region.** On every frame in which they change, the boxes of the active zones are handed to the compositor as the input region, right before presenting: the rest of the surface, even though it is its own, lets the click through.
- The GPU (device, pipeline, buffers) is created with the first render surface that arrives.

## Typing (`render.rs`, `texto.rs`)

An `input` is edited by **the renderer**, not by the logic: the key arrives, `Edicion` changes the text and the caret, and it shows in that same frame even if the logic has been stuck for a second. With every layout the workshop returns where each letter falls (`cursores`), and with that the caret is placed, the selection is painted and it is known which letter is under the mouse. The logic receives the text already changed (`Evento::Texto`) and the Enter (`Evento::Envia`).

Key repeat is ours (400 ms, then one every 33): that way it is the same on every system. The clipboard is `arboard`, which exists on all three.

## Commands from outside (`plataforma/mod.rs`)

A thread listens on `$XDG_RUNTIME_DIR/pleamar-SCENE.sock`; `pleamar --decir SCENE "emit toggle"` writes one line and leaves (2 ms). A question (`get open`) goes to the renderer with a channel back (`ARender::Pregunta`) and is answered over the same socket. The command enters the renderer as `ARender::SucesoDeFuera`, through the same door as the logic's events. It is `cfg(unix)`: on Windows it will be a named pipe.

## Dropping from another application (`plataforma/wayland.rs`)

`wl_data_device`: when a drag enters, it looks at whether it brings `text/uri-list` or `text/plain`, and when it is dropped it is read on a separate thread —whoever offers it may take a while— and arrives as `ARender::Soltado`. The renderer looks at which zone is underneath and fires `on drop zone`.

## Several files (`lenguaje/mod.rs`)

`import` is resolved **before** compiling, by splicing trees: each file is tokenized and parsed on its own, what the libraries declare goes in front of the scene's body, and `obra` compiles a single tree without knowing there was more than one. So that an error knows which file it comes from without loading every token with a name, the line number carries it inside: line 12 of the third file is 2,000,012, and it is undone when the error is shown. `leer_fichero` returns, along with the scene, the list of files it is made of: that is what hot reload watches.

## Models (`lenguaje/obra.rs`, `logica_luau.rs`)

The renderer does not know there are lists. `model rows max 14 { label: text; enabled: bool }` declares, underneath, fourteen live texts `rows.k.label`, fourteen facts `rows.k.enabled`, plus `rows.count` and `rows.total`. `for r in rows` unfolds fourteen copies, each one in a scope where `r` is another name for `rows.k` (the same aliasing mechanism as components) and with a condition: `rows.count > k`. That condition turns off the drawing, the slot in the layout and the zones. In Luau, `model.rows = list` hands each field to its text or to its fact and sends only what changed.

It is a deliberate decision: the language is already the definitive one —a list is written and walked— and the implementation can change underneath (copies born at runtime, G12) without touching a scene.

## Popups (`plataforma/wayland.rs`, `gpu.rs`, `forma.wgsl`)

A `popup` is not another scene: it is **another window onto the same one**. What is inside is compiled under a translation that takes it far away (10,000 px per popup), and its render surface carries an *origin*: the shader subtracts that origin when placing the quads and adds it back when computing each pixel. Everything else —properties, zones, rules, the atlas— is shared, and that is why a menu animates and is clicked just like the bar it comes out of. When composing, an element survives if it touches the main surface or the piece some open popup is showing.

The renderer decides when (the `open:` fact and the geometry, looked at after the rules) and calls `plataforma::emergente`; the platform creates the `xdg_popup` from that same thread —Wayland's objects allow it—, and when the compositor configures it, the render surface reaches the renderer like any other. The mouse inside a popup arrives with the origin already added. On closing, the renderer first lets go of what it was painting and then the surface is destroyed.

## At rest

If no property is moving and no behavior is alive, the renderer does not paint: it waits for a message, for a delay to come due, or for the next blink. Zero frames.

## How it is tested

```sh
./target/release/pleamar                 # Marea; right click closes it
./target/release/pleamar --escena isla   # the island
./target/release/pleamar --escena cara   # Marea's face: layers and gestures, with its script
./target/release/pleamar --escena muestrario   # gradient, border, nested transforms, rotated texture, group opacity
./target/release/pleamar --demo --segundos 9
./target/release/pleamar --ingenuo       # the logic blocks the renderer, like QtQuick
./target/release/pleamar --raton "360,90@500 500,172@1100 pulsa@3200 fuera@4500"
```

It comes up on `HDMI-A-1`. The graph at the bottom is one bar per frame; the red band is the time with the logic blocked. **Do not measure while capturing with `grim`**: it causes dropped frames.

## Known debts

- All of them are in [[pleamar · 08 Limitaciones conocidas]]. The heaviest ones:
- Scenes are written in Rust and compiled with the program.
- No layout.
- A group with opacity inside another one does not get a layer of its own: it multiplies. And if more than four are blending at once, the extra ones multiply too.
- With a different scale on each axis, the smoothing of the edge is approximate.
- The gradient and the light live in the group's space, not in the shape's: if the shape rotates *by itself* (`.girada`), the gradient does not rotate with it.
- The first frames after waking up do not wait for vsync: the animation clock should follow the presentation time.
- One frame in ~400 drops to 33 ms. Not investigated.
