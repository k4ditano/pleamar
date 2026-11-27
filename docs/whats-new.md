# What's new

The highlights of each line of releases; every release, with what changed, is in [Releases](https://github.com/k4ditano/pleamar/releases).

- **Language 0.12 (exp2): a window with a frame of its own does what a
  compositor's frame does.** `minimize self`, `maximize self`, `restore self`,
  `toggle maximize self` and `toggle fullscreen self` ask the compositor for the
  scene's own window; `self.maximized`, `self.fullscreen` and `self.active` say
  what it did, and `self.can_minimize`… what it can do; `on double bar` hears a
  double press, so a title bar maximizes on it. A compositor scene tells a program
  it has maximized it with `maximized:` on its `window`. Scenes from before read the same.
  Also: a part brought with `include` finds `lib:` and `import scene` as the
  scene does, and `--say … "fact x 1" "emit go"` reaches the logic in that order.
- **Language 0.11 (exp2): installed libraries.** `import "lib:tide/tokens"
  version "1.2"` brings a library installed once for several programs, looked
  for with `--lib`, `PLEAMAR_LIB_PATH`, a `lib/` next to the scene and
  `~/.local/share/pleamar/lib`; each library says its name, version and language
  in a `pleamar-lib` manifest, `--check` says which copy was used, and
  `pleamar lib vendor` copies them next to a program that carries its interface.
- **0.8 (experimental, on the `exp2` branch)**: several scenes in one process
  that talk to each other, checked at both ends (`import scene`, `uses scene`,
  events that say what they carry); text as people and agents write it
  (markdown with tables, links, pictures and code blocks, fields of several
  lines with undo, an input method, mentions and paste); lists with identity
  and windows that draw their own frame, popups out of popups, `pleamar:ui`'s
  menus and tables; colours with their transparency; a real blur and a finer
  glass; it runs on more graphics cards and says why when one cannot; and
  `--film` to see a scene without a screen. Try it with
  `git clone -b exp2 https://github.com/k4ditano/pleamar`.
- **0.6 — what the language would not let you write**: a quote inside a text
  (`\"`), a block that opens on the line after its header, and a colour chosen
  with `pick` or `if` inside a `let`.
- **0.5 — a round costs what changed**: a big scene only works out again what
  changed in it (a pleamar-wm round went from 1.2 ms to 0.4 ms), edges are
  sharper and blended as light would blend them, layouts can hold a chat
  (`stick: end`, `place:`, `tight:`), `motion.reduced` and `color.scheme` come
  from the desktop, and a scene paints on the same card the compositor paints with.
- **0.5.1 – 0.5.3**: the mouse's side buttons reach a scene, and apps and icons
  installed while a shell runs show up without restarting it.

Every release, with what changed: [Releases].


[Releases]: https://github.com/k4ditano/pleamar/releases
