# Language reference — version 0.1

**What this note is.** The complete, exact description of what the language accepts. [[pleamar · 09 El lenguaje v0]] is the guide —read straight through, with the reason behind each thing—; this is where a doubt gets looked up. It comes from the compiler (`src/lenguaje/`), not from memory, and **it cannot fall behind without `./probar.sh` saying so**: its whole examples compile, and its vocabulary (§17) is compared against the one the compiler consults.

```sh
pleamar --version                  # pleamar 0.1.0 · language 0.1
pleamar --comprobar scene.plm      # reads it, with whatever it imports; says whether it is fine, exits
./probar.sh                        # pruebas/*.plm, escenas/*.plm and the examples in this note
```

## 1. Version

The language has a number of its own, apart from the program's: **0.1**. The first changes when something already written stops being valid; the second, when something is added. A file can say which one it needs, on its first line:

```
language 0.1
```

If it asks for a different first number, or a second one higher than the program understands, it is an error on load —`this file asks for language 0.7, and this pleamar understands 0.1`— and not a half-built scene. Without that line, it is read with whatever is there. While the first is 0, nothing is promised: this is a language still being made.

## 2. What it guarantees

It is a **declarative language that always terminates**. No free loops, no recursion, no variables that mutate: `repeat` and `for` unfold on load, with a ceiling. Everything written is run by the renderer, on its own, at the cadence of the screen; the logic (Luau, apart) only sets facts, texts, models and events. **Every name is checked on load**: a misspelt one is an error with its file, its line, its arrow and a "did you mean…?", never a failure while running.

## 3. Lexicon

| | |
| --- | --- |
| Comment | `//` to the end of the line |
| End of statement | a newline or `;`. Inside parentheses, a newline ends nothing |
| Name | letters, digits, `_` and `.`; starts with a letter or `_`. Dots are part of the name: `orb.x`, `note.0.title`. Inside a `repeat`, `$i` is replaced by the number of the turn: `hit.$i` |
| Number | `40`, `0.5`, `-3`. With a unit: `40px` (= 40), `34%` (= 0.34), `138deg` (to radians) |
| Duration | `320ms`, `14s`. Where a duration is expected, a number without a unit is not valid |
| Color | `#151616` or `#fff` |
| Text | `"in quotes"`. With slots, see §11 |
| Symbols | `{ } ( ) , : ; = ~ + - * / < > <= >= == != .. -> % \|` |

Words of the language (they cannot be used as a name of one's own without confusing the reader, though the compiler does not forbid it): `language import scene library surface permissions model service spring prop pose fact event text image measure let zone body ellipse box arc line path input clip group popup component repeat for row column space layer on every blink wave spin follow look gesture posture`, and inside their statements `in max while for after from until at by reach within rest every inset right middle move curve close via as radial radius to change wrap true false and or not`.

## 4. Grammar

In EBNF: `[ x ]` is optional, `{ x }` zero or more times, `|` alternatives, `"x"` literally. `end` is a newline or `;`.

```
file         = [ "language" number end ] { "import" text end } ( scene | library ) ;
scene        = "scene" name "{" { statement } "}" ;
library      = "library" name [ "strict" ] "{" { let | spring | component | boundary | inner } "}" ;
boundary     = permissions | fact | event | live_text | model | image_decl ;    (* lives under the library's name: `Clock.now` *)
inner        = property | gesture | layer ;                                     (* what moves inside; also under its name *)

statement    = declaration | drawing | structure | layer | rule | behaviour | gesture ;

declaration  = surface | permissions | model | spring | property | fact | event
             | live_text | image_decl | figure_decl | measure | let | zone ;
surface      = "surface" [ name ] "{" { element_prop | statement } "}" ;   (* named: one of several, with its own things inside *)
permissions  = "permissions" "{" { ( "run" | "services" ) ":" text { "," text } end } "}" ;
model        = "model" name [ "max" number ] "{" { field | list } "}" ;
service      = "service" service_name [ "as" name ] "{" { field } "}" ;     (* the services, and their fields, are in §17 *)
field        = name ":" type [ "=" literal ] end ;
list         = "list" name [ "max" number ] ( "{" { field | list } "}"          (* records inside the record *)
                                            | "depth" number ) ;                (* …or like the outer one, down to that depth: a tree *)
type         = "text" | "number" | "bool" | "image" number "," number | enum ;
enum         = name "|" name { "|" name } ;
spring       = "spring" name "=" number "," number ;
property     = ( "prop" | "pose" ) name "=" number [ "~" spring_ref ] ;
fact         = "fact" name [ ":" ( "number" | "bool" | enum ) ] "=" ( number | "true" | "false" | name ) ;
event        = "event" name [ "->" ] ;
live_text    = "text" name "=" text ;
image_decl   = "image" name "=" ( "icon" text | "file" text | "from" name ) "," number "," number ;
figure_decl  = "figure" name "=" "file" text ;                                  (* un svg, por sus capas *)
measure      = "measure" name ;
let          = "let" name "=" ( expr | color ) ;
zone         = "zone" shape ;
spring_ref   = name | "spring" "(" number "," number ")" | duration ;   (* ~620ms: gets there in that long *)

drawing      = body | shape | text | image | figure | field | clip | group | popup ;
body         = "body" "{" { element_prop | shape } "}" ;
shape        = ( "ellipse" | "box" | "arc" | "line" ) [ name ] "{" { element_prop } "}"
             | "path" [ name ] "{" { element_prop | step } "}" ;
step         = "move" point | "line" point | "curve" point "via" point | "close" ;
text         = "text" ( text | name | "number" "(" expr [ "," number [ "," text ] ] ")" ) "{" { element_prop } "}" ;
image        = "image" name "{" { element_prop } "}" ;
figure       = "figure" name [ "." name ] "{" { element_prop } "}" ;            (* entera, o una capa *)
field        = "input" name "{" { element_prop } "}" ;
clip         = "clip" [ "inset" number ] shape ;
group        = "group" "{" { element_prop | statement } "}" ;
popup        = "popup" name "{" { element_prop | statement } "}" ;

structure    = component | copy | children | slot_block | repeat | for | layout | space | separator ;
component    = "component" name [ "(" [ parameter { "," parameter } ] ")" ] "{" { element_prop | statement } "}" ;
parameter    = name [ ":" param_type ] [ "=" argument ] ;
param_type   = "number" | "bool" | "color" | "text" | "record" | "event" | "image" | "gesture" | "spring" ;
copy         = Name [ "(" [ arguments ] ")" ] [ "{" { element_prop | statement } "}" ] ;   (* the statements are its children *)
children     = "children" [ name ] [ "{" { element_prop } "}" ] ;                          (* only inside a component *)
slot_block   = name "{" { statement } "}" ;                                                (* in a copy: what goes into the slot of that name *)
separator    = "between" [ name ] "{" { element_prop | statement } "}" ;                   (* only inside a layout; with several things, it needs `size:` *)
arguments    = argument { "," argument } { "," name ":" argument }
             | name ":" argument { "," name ":" argument } ;
argument     = expr | color | text | name ;
repeat       = "repeat" name "in" integer ".." integer "{" { statement } "}" ;
for          = "for" name "in" name [ "from" expr ] "{" { statement } "}" ;
layout       = ( "row" | "column" ) [ name ] [ "~" spring_ref ] "{" { element_prop | statement } "}" ;
space        = "space" expr ;

layer        = "layer" name [ "~" spring_ref ] "{" { claim } "}" ;
claim        = name [ when ] [ "{" { transition } "}" ] ;
when         = "while" expr | "for" duration "after" events | "from" events "until" events ;
events       = name { "," name } ;
transition   = name ":" expr [ "~" spring_ref ] [ "after" duration ] end ;

rule         = "on" trigger [ "while" expr ] "{" { effect } "}"
             | "every" duration [ ".." duration ] [ "while" expr ] "{" { effect } "}" ;
trigger      = "press" [ "right" | "middle" ] zone_ref | "release" zone_ref | "scroll" zone_ref
             | "drag" zone_ref | "hold" zone_ref "for" duration
             | "enter" zone_ref | "leave" zone_ref
             | ( "hover" | "away" ) zone_ref "for" duration
             | "idle" "for" duration
             | "key" key | "submit" name | "focus" | "blur" | "drop" zone_ref
             | name ;                                (* an event *)
key          = name { "+" name } ;                   (* Escape · Ctrl+k · Super+Alt+s *)
effect       = transition | name "=" expr | "toggle" name
             | "emit" name [ "(" expr ")" ] | "impulse" name expr
             | "play" name | "focus" name | "blur" ;

behaviour    = "blink" name "every" duration [ ".." duration ] "for" duration
             | "wave" name "=" expr "at" number
             | "spin" name "by" expr
             | "follow" name "=" expr
             | "look" name "," name "at" point "reach" number "," number "within" number [ "rest" point ] ;

gesture      = "gesture" name ( "ambient" | "reflex" | "asked" | "state" ) "{" { frame } "}"
             | "posture" name "while" expr "{" { frame } "}" ;
frame        = duration { curve | "hold" duration | "emit" name } [ "{" { name ":" expr end } "}" ] ;
curve        = "linear" | "in_quad" | "out_quad" | "in_cubic" | "out_cubic" | "in_out_sine" | "out_back" ;

element_prop = name ":" value { "," value } end ;       (* which ones are valid, depending on the element: §8 *)
point        = expr "," expr ;

expr         = or ;
or           = and { "or" and } ;
and          = not { "and" not } ;
not          = "not" not | sum [ ( "<" | ">" | "<=" | ">=" | "==" | "!=" ) sum ] ;
sum          = product { ( "+" | "-" ) product } ;
product      = unary { ( "*" | "/" ) unary } ;
unary        = "-" unary | "(" expr ")" | number | duration | "true" | "false"
             | name | function "(" [ expr { "," expr } ] ")" ;
color        = "#" hex | name | "mix" "(" color "," color "," expr ")" ;
```

`Name` in `copy` is that of an already declared component: by convention capitalised, which is what tells it apart at a glance from a statement of the language.

## 5. Files, order and names

**A file is a scene or a library.** A scene is opened; a library is imported. `import "path.plm"` goes before `scene` or `library`, and the path is relative **to the file that imports it**. A library imported by two routes is read once; a circle is an error that says its route. A library only declares: `let`, `spring` and `component`. What is imported behaves as if it were written at the start of the scene.

**`library Name strict { … }`**: its components can only read what they ask for by parameter, what they declare themselves, what belongs to their library (and to whatever it imports), and the names that always exist. Reading a fact, a color or an event of the scene without asking for it is an error on load —`'Nosy' belongs to a `strict` library and reads 'secret', which is the scene's, without asking for it`—: that way someone else's library does not depend on what things are called in the scene, nor does it touch them. Without `strict`, a component sees everything belonging to whoever uses it, which is the comfortable thing for one's own libraries.

**A plugin is a library with its logic next to it**: `clock.plm` and `clock.luau`. It can also declare its own boundary —`fact`, `text`, `model`, `event`— and its `permissions`:

```
library Clock strict {
    permissions { run: "date" }           // ITS logic's, not the scene's
    text now = "--:--"
    fact seconds = false
    event tapped ->
    component Clock(tone: color = ink) {
        row face { padding: 9; fill: coal; corner: 14;  text now { size: 14; color: tone } }
        on press face { emit tapped }
    }
}
```

- Its boundary **lives under the name of the library**: inside it is written `now`; from the scene, `Clock.now`. Two plugins can each have their own `count`, and neither treads on the scene's.
- Its logic runs in **its own Luau state**, and there `text.now` is `Clock.now`: it cannot name —cannot read, write or emit— anything that is not its own, neither the scene's nor another plugin's. It does not hear the keyboard, or the mouse, or anybody else's events; it does not ask for gestures or move the writing cursor.
- What it touches of the system is set by **its** `permissions`. The scene's are no good to it, and its own are no good to the scene. On start-up this is printed: `logic  · plugin 'Clock' · permissions · commands: date · services: none`.
- A scene may have no logic of its own and use plugins that do. Saving a plugin's `.plm` or `.luau` reloads hot, like everything else.

Asking for permissions without a `.luau` next to it is an error: there is nobody to use them.

**A plugin's permissions are approved by whoever uses it.** Declaring them is not having them: `pleamar --aprobar scene.plm` shows what each plugin of that scene asks for, and asks. What is approved is stored outside the plugin, with the fingerprint of its logic and of what it asked for: if either of the two changes, it goes back to unapproved. **Unapproved, a plugin runs with no permissions at all**, and its errors say why and how to approve it. An interpreter (`sh`, `python`…) comes out flagged: it is asking for everything. The scene one opens does not go through this: opening it is already deciding.

A library can also bring **what moves inside** —`prop`, `pose`, `gesture`, `posture`, `layer`— and **images and figures** (`image logo = file "logo.png", 16, 16`, `figure hat = file "hat.svg"`: the path is relative to the file that writes it, so the piece travels with it). All under its name, like its boundary. What it cannot do is draw outside a component, or hold loose rules: that belongs to the scene.

**The scene talks to a plugin by emitting one of its events** (`on press button { emit Face.cheer }`), which the plugin's components hear (`on cheer { … }`) and its logic too (`on("cheer", …)`); and it can read and set the facts of its boundary (`Face.happy = false`): the scene is the owner. A plugin has no surface of its own: if something needs one, it is a scene.

**The file is read in four passes** —declarations; `let` and layers; drawing; rules—, so the order of what is written is whatever suits the reader: a rule can come before the shape it names, and a `prop` at the end. Two exceptions: a `let` has to come before whoever uses it, and **things are painted in the order they are written** (and of two zones, the one declared later ends up on top).

**Every name is global**, except inside a component or inside one turn of a `repeat` or a `for`: there, what is declared belongs to that copy (two copies of `Note` each have their own `lit` and their own `hit` zone), and the inner things are looked up first —parameters, the component's `let`— and then the outer ones. A `let` of the scene with the name of an imported one treads on it: that is how a tone is changed. Two components with the same name do not coexist.

**Names that always exist**, read like facts: `screen.width`, `screen.height` (what the real surface measures), `screen.index` (which monitor copy this is, with `screens: each`; 0 otherwise), and during a rule, the mouse's: `pointer.x`, `pointer.y` (on the surface), `local.x`, `local.y` (inside the zone), `drag.dx`, `drag.dy` (since the press), `wheel` (notches; positive is upwards). And the `demo` event, which `--demo` fires.

## 6. Declarations

| Statement | What it declares |
| --- | --- |
| `surface { … }` · `surface panel { …; …drawing… }` | The windows it asks for. See below |
| `permissions { run: "date"; services: "audio", "audio.*" }` | What the logic may touch of the system. Undeclared, nothing. **Listening is not commanding**: `"audio"` allows knowing the volume (`sys.watch`, `sys.ask`); changing it needs `"audio.volume"`, or `"audio.*"` |
| `service clock as now { time: text; hour: number }` | A system service, by its name. Whatever it reports fills `now.time` and `now.hour` **without a line of logic**. See below |
| `model rows max 14 { label: text; enabled: bool = true; depth: number }` | A list of records that the logic fills. `max`: how many fit (1 to 256; 16 if unsaid). Creates `rows.count`, `rows.total` and, per record, `rows.K.field` |
| `prop orb.x = 360 ~lively` | An animated property: a spring. Without `~`, `lively` |
| `pose eyes = 14` | A property of the pose: the kind a gesture leads by the hand |
| `fact open = false` · `fact tries: number = 3` · `fact mode: low \| normal \| critical = normal` | Something that is true for a while. The logic and the rules set it. See **Types**, below |
| `event confirmed` · `event view_event ->` | Something that happens. With `->`, it also reaches the logic |
| `text notice.title = "Meeting"` | A live text: the logic changes it, or an `input` |
| `image fox = icon "firefox", 48, 48` | An image, and the largest logical size it is painted at. `icon "name"`, `file "path"`, or `from some_text`: whichever that text says (an icon name, or a path if it starts with `/`) |
| `figure hat = file "hat.svg"` | An svg **as geometry**: its layers become paths, each one named by the `id` of its group in the file. The path is relative to the file that writes it, so a library takes its pieces with it |
| `measure label` | Creates `label.width` and `label.height`, filled by the text that carries `measure: label` |
| `let panel.x = orb.x + 62` · `let mint = #9ed6bd` | A name for an expression, or for a color. A small one is substituted where it is named; **a big one is computed once a frame** and what is named is that, so a chain of them —each naming the one before— costs a sum and not a product |
| `spring bouncy = 170, 12` | A spring of one's own: stiffness, damping. From the house: `lively`, `calm`, `quick`, `slow`, `gentle`, `pose`. Inline: `~spring(170, 12)`, or **`~620ms`**: the spring that gets there in that long without overshooting |
| `zone box whole { at: …; size: …; active: expr }` | A zone that is not painted |

**Several windows in one process.** `surface { … }` with no name is the scene's, and draws whatever is loose. With a name, `surface panel { … }` is one of several and **carries inside it what it draws**:

```
surface { size: full, 40; anchor: top; screens: all }
box knob { … }
on press knob { toggle open }

surface panel {
    size: 300, 160;  anchor: top_right;  margin: 48, 12, 0, 0;  level: overlay
    open: open                                   // it is there while that is true: a fact, or any expression
    body { … };  box close { … }
    on press close { open = false }
}
```

They all **share properties, facts, models and rules**: a bar and its panel talk through a fact, without a round trip through the system and without knowing about each other. Inside, each one looks at a different slice of the same plane, just like a popup. A closed surface is not seen and cannot be pressed. `open:` takes any expression, so a surface can stay while what it holds finishes leaving: `open: tuck > 0.01`, with `tuck` a spring, closes the window once the thing has slid out of it and not before. **A named surface starts clean**: a loose `clip` of the scene ends where a named surface begins, instead of reaching into another window.

**A surface does not grow with what it draws**, and a shadow counts: `shadow: 0, 18, 44` reaches 62 px below its shape, so a card that ends 20 px from the bottom edge has its shadow cut there, and the cut is a straight line where a fade should be. Declare the surface with that room; the input region lets the click through the empty part, so nothing is lost by declaring it large. When a shape that is well clear of an edge has its shadow cut against it, the renderer says so, once, as soon as nothing is moving:

```
render · a shadow is cut: it needs 30 px below more than this 760 x 520 surface has. The shape fits; its shadow does not
render · this scene does not keep up: 32.3 ms a frame against the 16.7 the screen gives, and 31.0 of those go in reading the scene, not in drawing it
render · a drawing is cut: it needs 36 px below more than this 820 x 580 surface has. Almost all of it is inside, so it looks like the surface is the one that fell short
```

The second one is for the shape itself, which is what happens when a panel grows
past what its surface has. It is only said of what **almost** fitted —three
quarters of it inside, so a shape that hangs off an edge on purpose is left
alone— and never of an edge the surface is anchored to, where there is no room
to ask for. It waits until it has been cut for three seconds, or until nothing
is moving: crossing an edge on the way somewhere is not a layout mistake.

**Types.** For the renderer everything is numbers; types are for whoever writes and for whoever talks to the scene from outside. A fact is a number, a yes or no (`bool`; with no type, that is what one born `true` or `false` is) or an **enum**: `fact mode: low | normal | critical = normal`. The names of its values are valid in any expression (`mode == critical`, `mode = low` in a rule) and they are their position: `low` is 0. **An enum is compared against its own values, and the compiler checks it**: `mode == fast`, if `fast` belongs to another one, is an error that says which ones are valid; and arithmetic is not done with an enum (`mode + 1` means nothing; with a yes or no it does: `r.separator * 21`). The same name can be in two enums: compared against its fact, each one is its own; on its own, if it means different numbers, it is an error that asks for the long form, `mode.normal`, which is always valid. In a slot of a text, an enum is shown by its name: `"mode: {mode}"` → `mode: critical`. The logic reads and writes them as what they are —`fact.open` is `true`, `fact.mode` is `"critical"`—, and so does `--decir`.

The fields of a model have those types and two more: **`image w, h`** —an icon name or a path, and the image that says: `image r.icon { … }` without declaring anything else— and **`list`**, records inside the record:

```
model menu max 8 {
    label: text
    icon: image 20, 20
    kind: plain | checked | danger = plain
    list items max 6 { label: text; enabled: bool = true }
}
for m in menu { …  for it in m.items { text it.label { … } } }
```

An inner list is walked with `for it in m.items`, and it has its `m.items.count` and its `m.items.total`. **`list children max 6 depth 2`**, with no block, are records like the outer one, nested inside each other down to that depth (1 to 6): a tree, like the menu of an application. It is walked with as many `for` as levels are to be shown. Everything unfolds on load: 8 × 6 is 48 records, and the ceiling across all the lists of a model is 4096.

**Services, with no logic.** `service` asks the system for something by its name and says which of the fields that service brings it wants. Each field is an ordinary fact or text —of whatever type it is given— with the name in front, and it fills itself whenever the system reports something:

```
permissions { services: "clock" }                      // without permission it is not set up
service clock { time: text = "--:--"; date: text }     // without `as`: clock.time, clock.date
service clock.seconds as tick { second: number }       // with `as`: tick.second
text clock.time { size: 14; color: ink }
```

What each service brings is in the vocabulary (§17), and **asking it for what it does not have is an error on load**, with its "did you mean…?". What the numbers mean:

| | |
| --- | --- |
| `audio.volume` | 0 to 1 |
| `battery.percent` | 0 to 100 |
| `brightness.present` · `brightness.level` | the screen's backlight, 0 to 1. A machine with none says `present = false` |
| `network.strength` | 0 to 100 |
| `clock.hour` · `minute` · `second` · `day` · `month` · `year` | as they are read |
| `clock.weekday` | 0 is Sunday |
| `clock.time` · `clock.date` | already written out: `10:41`, `Sun 20 Sep` |
| `audio.input` · `audio.input_muted` | the microphone: the same two, for what comes in |
| `network.kind` | `none`, `wired` or `wifi` |
| the `bool` ones | `muted`, `charging`, `present`, `online`, `playing` |
 Whatever does not come in a report stays as it was. The ones that bring lists —`apps`, `tray`, `notifications`, `workspaces`— are not asked for this way: that is a model, and the logic hands it out with `sys.watch`.

**Which speaker and which microphone.** `audio` also reports, to `sys.watch`
and `sys.ask` only, **the devices there are**: `outputs` and `inputs`, each one
`{ id, name, default }`. They are lists, so they are not declared in `service`;
the logic spreads them into a model. `sys.call("audio.default", id)` switches to
one, and the report comes back with the new `default` set. A desktop sound panel
needs this: without it only the volume of whatever was already there can be
moved.

**Saying goodbye.** `session` is commands only —it reports nothing— and it is
what a desktop needs to close itself: `sys.call("session.lock")`, `"suspend"`,
`"logout"`, `"reboot"`, `"poweroff"`, behind `services: "session.*"`. The
permission matters more here than anywhere else, and for a different reason:
**these do not undo**. The worst an `audio.volume` can do is deafen you for a
second. Underneath they are `loginctl` and `systemctl`.

The permissions are the scene's, and they are the same as `sys.watch`'s: `services: "clock"`. Without them the scene loads all the same, says why on the console and that field stays as it was born. `clock` reports when the minute changes and `clock.seconds` every second; neither of the two asks anybody for the time nor wakes the machine up to see whether it is time yet.

**Files.** A scene has **its own folder**, and does not leave it: no paths, no `..`, like `require`. It is used from the logic, with permission `services: "files"` to read and `"files.write"` to write:

| | |
| --- | --- |
| `sys.ask("files.read", "settings.json")` | whatever it says, or `nil` if it is not there |
| `sys.ask("files.read", n, "json")` | that same thing, already as a table. If the file is broken, it is an error with its place, not a half table |
| `sys.ask("files.exists", n)` · `sys.ask("files.list")` · `sys.ask("files.folder")` | whether it is there · what is there · where |
| `sys.call("files.write", n, text)` · `sys.call("files.remove", n)` | write (all or nothing: first alongside, then into place) · remove |
| `sys.call("files.write", n, { tone = 2 })` | a table is saved as JSON, with its newlines: what is saved can also be read by hand |
| `sys.watch("files:settings.txt", f)` | reports when that file changes, including when somebody else touches it |

**The environment and the clipboard**, with their permissions (`services: "env"`, `"clipboard"`, `"clipboard.set"` to write):

| | |
| --- | --- |
| `sys.ask("env", "HOME")` | an environment variable, or `nil` |
| `sys.ask("clipboard")` | whatever has been copied, as text |
| `sys.call("clipboard.set", t)` | copy that. On Wayland, whoever copies has to stay alive: the platform takes care of that |

A plugin has its own, under its name: what it saves the scene does not see, nor the other way round.

**One per monitor.** `screens: each [max N]` repeats the surface on every monitor (4 at most, unless said otherwise), and **each copy has its own**: its properties, its zones and its rules. Inside:

| | |
| --- | --- |
| `$screen` | its number, to interpolate into a name: `mon.$screen.active`, like `$i` in a `repeat` |
| `screen.index` | the same, as a number |
| `screen.name` | the name of **its** monitor, set by the renderer (`HDMI-A-1`) |
| `screen.width` · `screen.height` | what **its** monitor measures |
| `screens.count` | how many monitors are showing something |

With the scene's surface (the one with no name), what is repeated is the loose drawing. `--pantalla A,B` spreads the copies across those monitors, which is how two are rehearsed without having two.

```
surface { size: full, 44; anchor: top; screens: each }
model mon max 4 { active: number = 1; title: text }     // one record per monitor, from the logic
text mon.$screen.title { … }
repeat i in 1..10 { Desk(i, mon.$screen.active) { show: ws.$i.there } }
```

**An ordinary window.** `kind: window` asks for a window with its frame and its cross, one of those the compositor places, instead of a panel stuck to an edge; `title:` is what it shows. What belongs to a panel —`anchor`, `level`, `reserve`, `screens`, `margin`— is no good to it, and **`screen.width` and `screen.height` are what it measures**, not its monitor: they change when somebody stretches it, so a window is drawn against them.

```
surface { size: 460, 320;  kind: window;  title: "pleamar · settings" }
box { from: 0, 0;  size: screen.width, screen.height;  color: coal }
```

**`surface`**: `size: width, height` (`full` as the width is the whole monitor) · `kind:` `panel` `window` `lock` · `title:` (a window only) · `anchor:` `top` `bottom` `left` `right` `top_left` `top_right` `bottom_left` `bottom_right` `center` — or **the name of a fact whose values are anchors** (`fact corner: top_left | top_right = top_right`, `anchor: corner`), and then it moves from edge to edge while it runs, without being recreated · `margin: n` or `top, right, bottom, left` · `level:` `background` `bottom` `top` `overlay` · `reserve: n` (the room windows leave it) · `rate: 60` (at most that many frames a second, on any monitor: what a scene costs is then the same on a 60 Hz screen and on a 165 Hz one; without it, the monitor's) · `screens: all` or `"HDMI-A-1", "DP-3"` · `keyboard:` `none` `on_demand` `exclusive`, and with `while expr` it only asks for it while that is true.

**A lock screen: `kind: lock`.** It is not a surface painted over everything: it is `ext-session-lock`, where the *compositor* guarantees that nothing else is seen or touched while it lasts, on every monitor. So it does not exist until its `open:` is true —which is mandatory: without it the session would be locked from the start— and it goes when `open:` stops being true. Its `size:` is the box that gets centred on each monitor; what lies around it shows too, so paint the backdrop large. `lock.held`, a name that always exists, is 1 once the compositor **confirms** the session is locked: a drawn padlock certifies nothing, this does. The password goes in an `input` with `secret: true` and is checked by the logic, `sys.ask("auth.check", text.password)`.

```plm
language 0.1
scene Lock {
    surface { size: 200, 50; anchor: top }
    fact locked = false
    surface screen_lock {
        kind: lock
        size: 600, 300
        open: locked
        box { from: -4000, -4000; size: 8600, 8300; color: #101214 }
        text "locked" { at: 300, 140; anchor: center; size: 26; color: #ffffff; opacity: lock.held }
    }
    box lock_it { at: 100, 25; size: 180, 36; corner: 12; color: #9ed6bd; cursor: pointer }
    on press lock_it { locked = true }
    on still locked for 5s while locked { locked = false }
}
```

If whoever locks dies with the lock held, the session **stays locked**. That is on purpose —the opposite would mean killing the locker unlocks— and it is the reason to try a lock screen inside a nested compositor first, never against a live session.

## 7. Expressions

They are numbers. **True is more than 0.5**; `true` is 1 and `false` is 0. The renderer evaluates them, on every frame that needs it.

From weakest to strongest: `or` · `and` · `not` · `< > <= >= == !=` (they do not chain: `a < b < c` is not valid) · `+ -` · `* /` · `-` in front. `==` means "equal to within a thousandth": these are numbers with a decimal point, and a spring never quite arrives.

| Function | |
| --- | --- |
| `min(a, b)` `max(a, b)` `abs(x)` | |
| `floor(x)` `ceil(x)` | to the integer below or the one above |
| `sin(deg)` `cos(deg)` | sine and cosine, **in degrees**: what it takes to put something on an arc. Animate the angle and not the x and the y, and the thing travels along the arc instead of cutting across it |
| `clamp(x, a, b)` | x, between a and b |
| `smooth(a, b, x)` | from 0 to 1 while x goes from a to b, easing in and out |
| `mix(a, b, t)` | between a and b. Also between two colors |
| `if(cond, a, b)` | a if the condition holds, b if not: only that side is evaluated. With a spring as the condition (`if(hot, a, b)`), it goes between the two, like `mix(b, a, hot)` |
| `vel(prop)` | the velocity of a spring, which only the renderer knows |

Valid as a name: a `let`, a `prop`, a `fact`, a measure (`label.width`), how much a named layout takes up and how many children it has in view (`list.width`, `list.height`, `list.count`: they can also be read before the point where it is declared), the numeric field of a record (`r.depth`, `r.index`, `rows.count`), and the presence of a claim (`shape.rec`: 1 while it wins).

## 8. Drawing

Each element accepts these properties and no others; another one is an error, with a suggestion.

| Element | Properties |
| --- | --- |
| `ellipse` | `at` · `radius` · `scale: sx, sy` |
| `box` | `at: cx, cy` or `from: x, y` · `size: w, h` · `corner` |
| `arc` (like "∩") | `at` · `radius` · `span` (the whole angle it covers) · `width`. It opens **upwards and symmetrically**; a progress ring is `span: p * 360deg` with `rotate: p * 180deg` |
| `line` | `from` · `to` · `width` |
| `path` | `at` (what its points hang from) · `size: w, h` (what it takes up in a layout), and inside it its steps: `move x, y` (once, the first one) · `line x, y` · `curve x, y via cx, cy` · `close`. Closed, it is filled; open, or with `stroke`, it is a line |
| …and every shape | `color` · `opacity` · `rotate` · `stroke` (the outline only) · `blend` (inside a `body`: how much it melts into what came before) · `active` · `cursor` · `show` |
| `body` | `color` or `gradient` (below) · `rim` · `light: amount, from_y, height` · `shadow: dx, dy, blur, alpha[, color]` · `border: width, #color` · `glass` · `opacity` · `show`, and inside it its shapes, melted into one silhouette |
| `text` | `at` · `anchor` · `width` · `lines` · `size` · `weight` · `color` · `opacity` · `align:` `left` `center` `right` · `line_height` · `family` · `measure` · `show` |
| `image` | `at` (its **top-left corner**, not its centre: it is a rectangle of pixels, not a shape) · `size` · `opacity` · `tint` · `show` |
| `figure` | `at` (where the piece's centre goes) · `size: w, h` or `scale:` (without either, one unit of the svg is one pixel) · `pivot: x, y` (in the svg's units, from its centre: the point it **turns** around, which does not move it) · `rotate` · `color` (instead of the one in the file) · `opacity` · `blend` · `stroke` · `show` |
| `input` | `at` · `width` · `size` · `weight` · `color` · `opacity` · `family` · `placeholder` · `selection` · `secret` · `show` |
| `group` | `pivot` · `rotate` · `scale: s` or `sx, sy` · `move: dx, dy` · `opacity` (they melt as a single thing) · `size` (for whoever lays it out) · `show` |
| `popup` | `at` (inside the surface) · `size` · `open:` a fact |
| `row` `column` | `at` · `anchor` · `gap` · `padding` · `align:` `start` `center` `end` · `fill` · `corner` · `opacity` · `cursor` · `show` · `size: w, h` · `view: w, h` · `step` · `content` · `wrap: n` |

`anchor` of a text: `left` `center` `right` and `top` `center` `bottom`, one or both (`anchor: left center`). Of a layout: `left` `center` `right` and `top` `middle` `bottom` —with no anchor, `at` is its top left corner—. `cursor:` `default` `pointer` `text` `grab` `grabbing`.

**A shadow can be of any colour, and everything about it is an expression.**
Black if no colour is said, which is what a shadow is on paper. On a desktop of
dark windows a black shadow has nothing to darken and reads as dirt, and a
light halo (`shadow: 0, 0, 8, 22%, #9ed6bd`, no offset, hugging the outline)
paints outside the silhouette, where it shows. Often what a small shape needs
there is its own `rim`, which is light **inside** the silhouette and touches
nothing behind it. And since the offset, the blur and the alpha are expressions
too, a shadow can show up only when there is something to cast one: `shadow: 0,
2 * open, 12 * open, 32% * open` gives a card the shadow of a card and leaves
the thing it grew out of with none. With the count at zero there is no shadow
to work out, and the renderer does not look at it.

**`glass` turns a body into glass**, from `0%` to `100%`, and like everything
else it is an expression, so it can come and go. Three things happen. The fill
becomes a tint: its `color` is still there, but what is behind shows through.
The edge catches the light: a thin highlight on the side facing the top left,
a softer one on the opposite side, and the glass getting lighter towards its
edge, like glass seen side-on; all of it comes from where the edge points,
which a signed distance knows. And the body's `shadow` stops showing through
it: only around it. On top of that, **the compositor is asked to blur what is
behind the silhouette**, following its shape in 2 px strips —a round thing
gets round blur, not a square—, through the standard `ext-background-effect`
protocol (Hyprland and KWin have it). Nobody has to write a blur rule for the
compositor: the scene asks. Where the compositor cannot do it, the glass is
still a tint with its light, just with nothing blurred behind. How strong the
blur is belongs to the compositor's settings. A loose shape takes `glass` too
(`box { …; color: #4a5057; glass: 100% }`), and so does the `fill` of a `row`
or a `column`: that is how a glass card holds glass plates —a mid grey at 36 %
over dark glass lightens it just enough, and each plate gets its own lit edge.
Inside a `body`, the body says it, not its shapes. Below 30 % of glass (times
opacity) no blur is asked for. What glass cannot do is bend what is behind it
like a lens: that needs the pixels behind, and on Linux only the compositor has
them.

```
body {
    color: #1b2127
    glass: 100%
    shadow: 0, 10, 28, 30%
    box { at: 250, 160; size: 420, 220; corner: 36 }
    ellipse { at: 520, 90; radius: 46; blend: 26 }
}
```

**A figure is an svg read as geometry, not as a stamp.** An image is rasterised
into an atlas: it shows, but it is a sticker —it melts into nothing, it cannot
be tinted by parts nor animated by layers, and scaling it is pixels—. `figure
hat = file "hat.svg"` reads the same file as **paths**, so each layer is a shape
like any other: inside a `body` it is one silhouette with it, it takes rim,
light and shadow, and it turns around its own pivot.

`figure hat { … }` draws the whole piece and `figure hat.brim { … }` one of its
layers, **in its place inside the piece**: two layers drawn separately still fit
together, which is what lets one of them turn while the others stay. What comes
across is filled and stroked paths with their colour; gradients, masks, filters
and text do not, and it says so when it reads the file instead of pretending.

A **path** is a broken or curved line. `close` closes it, and then it is filled —concave too, and crossing itself too—; unclosed, or with `stroke`, it is a line of that width with round caps. `curve` is a quadratic Bézier, and it is split into as many segments as the detour is long. Inside it is the same signed distance as the other shapes: it melts with `blend`, and it has shadow, rim, light and border like any other.

```
path {
    at: 26, 40;  stroke: 2.5;  color: mint
    move 0, 56;  line 26, 36 + breath * 5;  line 52, 44;  line 78, 12
}
```

A path carries **a single stroke** (one `move`, the first) and up to 64 already flattened points; for several, several `path`. In a layout it has to be told what it takes up with `size:`, because its box is not known until it is evaluated.

**Lists longer than what is unfolded.** A model unfolds its `max` records on load, and that is the ceiling (256). For a list of thousands, the scene declares only the **window** —what is seen and a little more— and says how long the whole list is:

| | |
| --- | --- |
| `content: total * 34` | in a layout with `view:`, the real length: the scrolling runs over it, not over what is unfolded |
| `for r in rows from first` | copy 0 is record `first` of the real list, so `r.index` is the number that belongs to it |
| `move: 0, first * 34` | puts the copies in their place within the whole list |
| `list.scroll` | besides being read, it **is written** like any property: `on press top { list.scroll: 0 ~calm }` |
| `on change floor(list.scroll / 34) { emit slid(list.scroll) }` | that is how the logic learns it has to be sent another slice, wherever the movement comes from |

A layout with `view:` **is dragged** without declaring anything: on being pressed it notes where it was and follows the mouse, with its spring. And it is grabbed **from inside**: dragging is not the business of the topmost zone, but of any zone that was underneath at the press, like the wheel. That is how a list is moved by grabbing it by one of its rows.

**`size: w, h`** says how big the layout is, instead of it being however much its children came to. Its background and its zone are that size even if the children do not reach it, and —this is what it is for— it is what **`grow:`** shares out. Across the axis it is also the box that `align:` aligns in, so `align: center` in a card with `size: 173, 66` centres its contents in those 66 and not around the tallest child.

**`grow: 1`** on a child asks for the room that is left along the layout's axis, divided among those who ask in proportion to what they asked (`grow: 2` takes twice as much). What is left is the layout's size minus its padding, its gaps and whatever the children that do not grow take. Without `size:` on the layout it is an error: there is no *left over* if nobody said of how much.

A child that grows gets **the slot**; what it then draws is still its own business, so a `box` inside a `group` does not stretch. What does follow is **a text**: a growing text, or the texts inside a growing `group` or `column` that did not say their own `width`, take it from there. That is the case that bites — a name running under the switch beside it instead of ending in an ellipsis:

```
row card { size: 164, 66; gap: 10; padding: 12; fill: #1b1b1c; corner: 14
    group { size: 22, 22;  …the icon… }
    column { grow: 1; gap: 2
        text title { size: 13; color: ink }
        text subtitle { size: 11.5; lines: 1; color: #8b8f95 }   // it ends in "…" on its own
    }
    group { size: 40, 23;  …the switch… }
}
```

**`wrap: 5`** turns a layout into a **grid**: five per line and on to the next. The cell is as big as the largest child, and **what is not seen leaves no gap**, so the rest move up, with the layout's spring if it has one. It is what a `Flow` is in Quickshell.

`escenas/lista-larga` is five thousand rows in sixteen copies: 0.49 ms per frame, and the same sixteen groups and nineteen zones however many there are.

`clip [inset n] shape` clips everything that comes after, to the end of its `group` —or, loose in the scene, up to the next named `surface`—. Up to four nested ones clip by their shape; the outer ones, by their box.

**Gradients.** In a `body`, `gradient:` takes where it goes from and to and then its colors, separated by commas. Each color can say **where it falls** (`sand 40%`); the ones that do not say it are spread evenly. From two to eight.

```
gradient: 0, 0, 0, 44, mint, coal                        // from one point to another
gradient: 0, 0 to 0, 44, mint, sand 30%, #e86a9a, coal   // the same, with stops
gradient: radial 100, 160 radius 60, ink, mint 40%, coal // from a center outwards
```

**A named shape is a zone** if some rule names it, if it carries `active`, or if it was declared with `zone`. A name put there only to read better does not stop a click. A zone inherits the transforms of the groups it is in, and **what is not there —a false `show:`, an `opacity:` that has reached zero, a record that does not exist— is not a zone**: what cannot be seen cannot be pressed. A `row` or `column` with a name is one too: its whole box, underneath those of its children.

## 9. Layouts

`row` and `column` place their children one after another: the place of each one is an expression, so if one grows or disappears, the rest move. With `~spring` in the header, they travel to their place instead of jumping. Every child has to know how much it takes up: a shape with `size` or `radius`, a text (it measures itself), an image, a `group` or a component with `size:`, another layout, or `space n`. `show: expr` on a child decides whether it is there: it takes up room and is seen, or neither of the two.

**Lists longer than the room they have.** `view: w, h` says what is seen; what is inside can be longer and **moves with the wheel**, clipped and never past what there is. `step:` is how much per notch (60 by default), and the layout's spring is the one it travels with. Besides `width`, `height` and `count`, it publishes `list.content` —how much there is— and `list.scroll` —where it is— which is what is needed to paint a little bar alongside:

```
column list { at: 16, 16;  view: 250, 208;  gap: 6
    for r in rows { Row(r) }
}
box { from: 276, 16 + list.scroll / max(list.content, 1) * 208
      size: 4, 208 * 208 / max(list.content, 208);  corner: 2;  color: ink;  opacity: 35% }
```

With `view:`, outwards it takes up what is seen, not what it carries inside.

`between { box { size: 272, 1; color: ink } }` puts that **between every two children that are there**: if one disappears, so does its line, and there is never one at the start or at the end. **It does not open another gap**: it goes centred in the `gap` that is already between its neighbours, so the distance between two children is the `gap` plus what it takes up. With a single thing inside, that thing says how much it takes up; with several, the `between` acts as a group and says it itself (`between { size: 10, 12; … }`). `between i { … }` gives it its position —1 after the first child, 2 after the second…—, so the first one can be different: `opacity: if(i == 1, 50%, 12%)`. A named layout publishes, besides `list.width` and `list.height`, **`list.count`**: how many children are there right now. All three can also be read before the point where it is declared.

## 10. Components, `repeat`, `for`

`component Name(parameters) { size: w, h; … }` declares; `Name(arguments)` places a copy. `size:` says how much it takes up, for whoever lays it out. What a copy declares —`prop`, named shapes, rules— is its own.

**A component says what it needs.** Each parameter can carry a type and a default value: `component Row(r: record, chosen: event, tone: color = mint, height: number = 30)`.

| Type | What is passed to it | Inside |
| --- | --- | --- |
| `number` | an expression | valid in any expression |
| `color` | `#fff`, a color `let`, `mix(…)` | wherever a color goes |
| `text` | `"in quotes"` (with slots, if it likes) or the name of a live text | `text name { … }`, and in a slot: `"{name}"` |
| `record` | a record: that of a `for`, or `rows.0` | `r.field`, `r.index` |
| `event` | the name of an event of the scene | `emit name(…)` and `on name { … }` talk about **that** event |
| `image` | the name of an image | `image name { … }` |
| `bool` | an expression (`true`, `false`, `count > 3`) | valid in any expression |
| `gesture` | the name of a gesture | `play name` |
| `spring` | the name of a spring, or `spring(170, 12)` | `~name` |

That way a library component does not take it for granted that the scene has an event called in a certain way: it asks for it. Arguments go **by position and then, if wanted, by name** (`Row(r, choose, height: 40)`); from the first one with a name, all with a name. The ones that have a default value can be omitted, and they go at the end. Whatever is missing, extra, repeated or not of the type is an error where it is used, showing the whole signature: `'Row' is missing 'chosen' (an event): it is Row(r: record, chosen: event, tone: color = …)`. The default value is read where the component is used, so `= mint` is the `mint` of that scene.

With no type (`component Dot(tone)`), the parameter is whatever the argument looks like: it is how they used to be written, and it is still valid.

**A slot for children: `children`.** What a copy brings inside its block —besides properties like `show:`— goes wherever its component says `children`:

```
component Card(title: text) {
    size: 300, 40 + inside.height            // as big as whatever is put into it
    body { color: #1b1c1c; box { from: 0, 0; size: 300, 40 + inside.height; corner: 12 } }
    text "{upper(title)}" { at: 12, 18; anchor: left center; size: 11; color: ink }
    column inside { at: 12, 32; gap: 4;  children }
}

Card("Alerts") {
    text title { size: 14; color: ink }      // the scene's `title`, not Card's parameter
    repeat i in 0..2 { text "row {i}" { size: 13; color: ink } }
}
```

Inside a `row` or a `column`, each child takes its place in the layout (and a `repeat` or a `for` from outside unfolds like the ones inside); loose, `children { move: x, y }` is a group. **Children are read with the names of whoever wrote them**: a component neither sees nor treads on what is put into it, and a parameter of its own covers nothing from outside. **Several slots, with names.** A component has at most one `children` with no name and as many with one as it likes: `children header`, `children footer`. In the copy, a block with that name is what goes into that slot —**even if the scene has a component called the same: inside the copy, the slot wins**—, and the rest goes to the one that has no name. A slot cannot be called like a word of the language.

```
Panel {
    header { text "Alerts" { … } }
    for n in notes { text n.title { … } }        // to the `children` with no name
    footer { box ok { … };  box no { … } }
}
```

A slot that does not exist is an error that says which ones there are (`'Panel' has no slot called 'heder': it has header, footer. Did you mean 'header'?`), and so is putting something into a component with no `children`: not a silence.

An error **inside** a component also says where it was used from —`(inside 'Badge', used at scene.plm:6)`—, because often what is wrong is what was passed to it.

`repeat i in 0..5 { … }` unfolds five turns on load (512 at most); inside, `i` is a number and `$i` is substituted into the names.

`for r in rows { … }` unfolds one turn per record that fits in the model. Inside, `r.field` is the field of that record —a text wherever a live text goes, a number in any expression— and `r.index` its position from 0. **Each turn only exists if the list reaches that far.** Valid inside a layout, loose, and inside a `popup`.

## 11. Text with slots

Inside a text in quotes that is the content of a `text` or the argument of a component:

| | |
| --- | --- |
| `{name}` | a live text, or the `text` field of a record |
| `{expr}` · `{expr, n}` | an expression, with n decimals (0 if unsaid) |
| `{expr, time}` | that many seconds, the way a clock writes them: `1:07`, and `1:02:07` past the hour |
| `{upper(name)}` · `{lower(name)}` | that text, in upper or lower case |
| `{? … }` | a stretch that is only there if none of the texts inside it is empty |
| `{{` · `}}` | a real brace |

The names of a slot are resolved where the string is written, not where it is used.

## 12. Layers

`layer name [~spring] { claims }`. **The first claim that holds wins**, from top to bottom; when it stops holding, the next one is seen, on its own. A claim is a `name` followed by when —`while expr`, `for 700ms after event, other`, `from event until event`, or nothing (the default)— and, if it likes, a block with its choreography: where each property goes, with which spring and with which delay. The destination is an expression evaluated when its turn comes. `layer.claim` is 1 while it wins, and it can be read in any expression.

## 13. Rules

`on trigger { effects }` and `every 2s..7s [while expr] { effects }`.

| Trigger | When |
| --- | --- |
| `press zone` · `press right zone` · `press middle zone` | it is pressed. Where a scene uses `right`, the prototype's emergency exit (the right button closes) only fires where the click lands on no zone at all |
| `release zone` | what was pressed there is released, wherever the mouse is by then |
| `hold zone for 500ms` | it has been held down that long |
| `enter zone` · `leave zone` | the mouse enters or leaves |
| `hover zone for 320ms` · `away zone for 420ms` | it has been over it that long; it was over it and has been away that long |
| `scroll zone` | the wheel, over any zone underneath it. Read in `wheel` |
| `drag zone` | it moves with the button down; it keeps going even if it leaves, until release. `local.x`, `drag.dx`. Like the wheel, it works for any zone that was underneath at the press, not only the topmost one: that is how a list is dragged by grabbing it by a row |
| `change expr` | that computation stops being worth what it was worth. Being born does not count: it fires on changing |
| `still expr for 1.1s` | that computation has been worth the same for that long. The reverse of `change`, and like it, being born does not count. Every change puts the clock back to zero, so a run —the volume key pressed six times— is one wait and not six, and what it fires happens once when the run ends |
| `key Escape` · `key Ctrl+k` | a key; the surface has to ask for the keyboard. Modifiers: `Ctrl+` `Alt+` `Super+` |
| `submit field` | Enter inside that `input` |
| `focus` · `blur` | the surface gains or loses the keyboard |
| `drop zone` | something dragged from another application is dropped on it |
| `idle for 14s` | nobody touches anything for that long |
| `event_name` | that event happens: the logic emits it, or another rule, or a gesture, or it comes from outside |

**Any rule accepts `while expr`** at the end of its header: it is looked at at the moment of firing — **at the state the frame began with**, so two rules that fire in the same frame both see the same one, and the one declared last is the one whose value stays. In `idle` and `every` it also decides whether the wait counts.

| Effect | |
| --- | --- |
| `prop: value ~spring after 70ms` | that property heads there. Without `~`, **with its own spring**: the one it was declared with |
| `fact = expr` | evaluated on firing |
| `toggle fact` | |
| `emit event` · `emit event(expr)` | with a payload, which reaches the logic |
| `impulse prop -620` · `impulse pop left * 5` | a shove: it adds to the velocity of the spring. The amount is evaluated on firing, so it can depend on what is going on: a countdown's bounce shrinks with the number left |
| `play gesture` | it asks for it; it will be granted or not, depending on its class |
| `focus field` · `blur` | gives the writing cursor to an `input`, or takes it away |

## 14. Movement that carries itself

| | |
| --- | --- |
| `blink eyelid every 2.4s..6s for 170ms` · `blink lid every 5.2s for 120ms` | from 1 to 0 and back, every now and then. Without `..`, always the same wait. The period runs **start to start**: "every 5.2 s" is every 5.2 s, not 5.2 s after the eye closed |
| `wave breath = amplitude at 1.7` | amplitude · sin(1.7 t) |
| `spin angle by 0.9` | += 0.9 per second |
| `follow chip.w = label.width + 32` | chases the expression, with its spring |
| `look gx, gy at cx, cy reach 5, 3.2 within 140 rest rx, ry` | two properties that pull towards the mouse |

With reduced motion (`--movimiento-reducido`) the first three go quiet: `blink`
stays open, `wave` rests at the middle of its travel and `spin` stops where it
was. What carries itself is exactly what must not be left going round on its
own. `follow` and `look` are not loops —they chase something— so they stay,
with their springs settling at once like every other one.

## 15. Gestures

A gesture is a timeline over the properties of the pose (`pose`). `gesture name class { frames }`; classes, from weakest to strongest: `ambient` < postures < `reflex` < `asked` < `state`. **A gesture only cuts off another of its own class or lower.** A frame is a duration, and if it likes a curve, `hold 60ms` (holds there) and `emit event` —which fires when the frame **begins**: to say "done", give it a short frame of its own at the end—; its block says where each property goes, and whatever it does not name returns to its base. With no block, it is the return to the base. `posture name while expr { … }` repeats on its own while that is true.

**A spring can be said in time.** `~620ms` is the spring that arrives in 620 ms
and does not bounce: critically damped, worked out from the time asked for.
Animation contracts are written in milliseconds, and this is how they get
transcribed without stopping being springs — they keep their speed, they can be
interrupted halfway, and they survive a hot reload. Measured: `~620ms` reaches
99 % at 617 ms.

**A gesture outranks whatever is ambient.** While it is holding a pose, any
`blink`, `wave` or `spin` on that same pose goes quiet, **and its clock stops with
it**: after the gesture, it picks up where it left off instead of firing at once
everything it owed. That is what lets a breath carry its own blink without the
regular one landing on top of it.

## 16. Checked examples

These compile with `./probar.sh`.

A list that comes from data, with a component, text with slots, and one rule per row:

```plm
language 0.1
scene Reference1 {
    surface { size: 320, 220; anchor: top; margin: 40 }
    let ink = #f5f7f5
    model notes max 4 { app: text; title: text; body: text; urgency: number = 1 }
    event opened ->

    text "{notes.total} alerts" { at: 20, 20; anchor: left center; size: 15; weight: 600; color: ink }
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

A layer that decides, a popup, and the keyboard only while it is needed:

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
    text "Save" { at: 150, 30; anchor: center; size: 14; color: #f5f7f5; opacity: 60% + mood.idle * 40% }

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

A pose, a gesture and movement that carries itself: the face is the example, but it works for anything that has states and moves between them.

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

## 17. The vocabulary, just as the compiler consults it

This is the output of `pleamar --gramatica`, copied. It is not a second list: these are the same tables (`src/lenguaje/vocabulario.rs`) the compiler consults to accept or reject a word. `./probar.sh` compares this block with what the program prints —if somebody adds a word and does not write it down here, it fails— and it also checks that **every word appears in some test**.

```vocabulario
language: 0.1
statements: surface permissions model service spring prop pose fact event text image figure measure let zone body ellipse box arc line path input clip group popup component children repeat for row column space between layer on every blink wave spin follow look gesture posture
library: let spring component permissions fact text model service event image figure prop pose gesture posture layer
properties.surface: size anchor margin level reserve screens keyboard open kind title rate
properties.permissions: run services
properties.shape: rotate stroke color opacity blend glass active show cursor grow
properties.ellipse: at radius scale
properties.box: at from size corner
properties.arc: at radius span width
properties.line: from to width
properties.path: at size
properties.body: color gradient rim light shadow border glass opacity show
properties.text: at anchor width size weight color opacity lines align line_height family measure show grow
properties.image: at size opacity tint show grow
properties.figure: at size scale rotate pivot color opacity blend stroke show grow
properties.input: at width size weight color opacity family placeholder selection secret show
properties.group: pivot rotate scale move opacity size show grow
properties.popup: at size open
properties.children: move
properties.layout: at anchor gap padding align fill glass corner show opacity cursor view step content wrap size grow
functions: min max abs floor ceil sin cos clamp smooth mix if vel
text_functions: upper lower
triggers: press release scroll drag hold enter leave hover away idle key submit focus blur drop change still
effects: toggle emit impulse play focus blur
curves: linear in_quad out_quad in_cubic out_cubic in_out_sine out_back
frame: hold emit
classes: ambient reflex asked state
field_types: text number bool image
fact_types: number bool
model: list
path: move line curve close
documented: surface permissions model service spring prop pose fact event text image figure measure let zone body ellipse box arc line path input clip group popup component children repeat for row column space between layer on every blink wave spin follow look gesture posture import scene library language
services: clock clock.seconds audio battery brightness network media window
services.clock: hour minute second day month year weekday time date
services.clock.seconds: hour minute second day month year weekday time date
services.audio: volume muted input input_muted
services.battery: present percent charging
services.brightness: present level
services.network: online kind name strength
services.media: playing title artist album player
services.window: title class monitor
parameter_types: number bool color text record event image gesture spring
springs: lively calm quick slow gentle pose
units: px % deg ms s
cursors: default pointer text grab grabbing
surface.anchor: top bottom left right top_left top_right bottom_left bottom_right center
surface.level: background bottom top overlay
surface.kind: panel window lock
surface.keyboard: none on_demand exclusive
text.align: left center right
layout.align: start center end
```

`properties.shape` are the ones common to `ellipse`, `box`, `arc` and `line`; `properties.layout`, those of `row` and `column`.

## 17.1. In the editor

`pleamar --lsp` is a language server over the input and the output, with **this same compiler** behind it: the errors with their place while it is being written, which words are valid here, and what the one under the cursor means. `pleamar --resaltado vim` and `--resaltado vscode` write the syntax file, taken from the vocabulary above. Both, and how they are installed, are in `editor/`.

## 18. What this version does not have

So as not to look for it here: `import … as`, a line break in the layouts, times and plurals in the slots, and writing to the field of a record from a rule. It is all there, with its plan, in [[pleamar · 08 Limitaciones conocidas]].
