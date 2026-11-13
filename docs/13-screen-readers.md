# Screen readers

A pleamar scene tells a screen reader (Orca, on Linux) what it shows and lets it act, through **AccessKit** and, on Linux, **AT-SPI**. It is the same tree an agent reads with `describe` ([12](12-agents.md)), seen as the person would; nothing has to be written for it, though `label:`, `role:`, `value:`, `checked:` and `selected:` make it better, as they do for an agent.

## What a reader is told

Each surface (and each popup) is a window of its own, named as `describe` names it (`main`, or the surface's name), with in it every zone and every loose text on screen, in the order they were written. A list's rows hang from the list.

| `describe` | AccessKit (AT-SPI) | What it has |
| --- | --- | --- |
| `button` | Button (push button) | its name; Click |
| `toggle` with `checked:` | Switch (toggle button, pressed when on) | its name, on/off, its `value:`; Click |
| `toggle` without | Button | Click |
| `tab` | Tab | selected; Click |
| `link` | Link | Click |
| `item` | ListItem | selected; Click |
| `slider` | Slider | its value (a number as a number, `"50%"` as words); Increment, Decrement |
| `field` | TextInput (entry) | its words; Focus, Click, SetValue |
| `field`, `secret: true` | PasswordInput | never its words; Focus, Click |
| `list` | List | its rows |
| `region` | Group | — |
| `window` (another program's) | Window | its title |
| loose words | Label | the words |

Each has its box, in the surface's logical pixels; one that is inactive is said to be disabled. The field being written in has the reader's attention (its focus); with none, the window has it, or whatever the reader last put its attention on.

## What a reader can do

What it asks is done the way an agent's `press` is ([12 §5](12-agents.md)): the zone is scrolled into sight if its list hides it, and a hand presses the point of it that is on top. There is no agent cursor, because there is no agent.

- **Click**: a press.
- **Focus** on a field: it is pressed, so it is written in. On anything else it only moves the reader's attention.
- **SetValue** on a field: its words replaced, as `type` does.
- **Increment / Decrement** on a slider: a notch of the wheel, if the scene listens to it there; if not, a drag of a twentieth of its length.

**`agent: no` does not apply to a reader.** It is there so that what only a person may decide —sharing the screen, deleting, «Stop» for an agent— is never done by an agent. A reader is not an agent: it is how that person sees and presses, and leaving those zones out would leave them without the buttons that matter most. So a zone kept from agents is in the reader's tree and can be pressed, and a field kept from agents says its words to the reader. A **secret** field never says its words to anyone.

The price: AT-SPI is read by any program running as the person, and an agent that drives AT-SPI could press what `agent: no` keeps from it. Nothing can tell it apart from a reader. That is why **what is private is `agent: hidden` or `secret`**, and those never reach AT-SPI: a zone or surface hidden from agents is not in the reader's tree, and a secret field's words are never in it. Neither are the lock screen and a surface kept out of captures (see the limits below).

## What it costs

**Nothing while no reader is listening.** Each surface only registers with AccessKit when it appears. AccessKit's own thread then waits on the session bus for AT-SPI to be turned on. The render looks at one flag per round. No tree is built and nothing is sent.

When a reader turns up, AccessKit asks for the whole tree. The render then gathers what is on screen while it draws its next frame (the same look an agent's `describe` takes) and sends it. After that, each time something changed it looks again, at most every 100 ms (`READER_EVERY`), and sends only the nodes that differ from what the reader already has. A node that goes away is dropped from its parent's children, so the parent is sent again.

Measured on an RTX 2060, in a headless pleamar-wm with AT-SPI on the bus but no reader, exp.11 against exp.11 with this (8 starts of each, alternating):

- First frame of a warm start: median 276 against 280 ms, which is within noise on that machine.
- Peak memory: 172 against 173 MB.
- A scene moving every frame: 16.94 ms between frames both ways. The render's CPU per round was 0.115 against 0.118 ms composing and 0.74 against 0.71 painting, also within noise.
- With an AT-SPI client reading the tree every second: one run, composing 0.12 against 0.11 ms.
- The binary grows by 2.9 MB (37.7 to 40.6 MB, +7.7 %): AccessKit, its AT-SPI side and their async runtime. zbus and zvariant were already there, in the same versions.

The `accessibility` feature is on by default. Without it (`--no-default-features --features luau`) the same calls compile to nothing.

## Checking it

`tests/screen-reader.plm` has one of each kind: a button, one kept from agents, a field, a slider, a toggle and a list. `tools/reader/atspi-check.py` is a minimal AT-SPI client (PyGObject's Atspi, as Orca uses). It prints the tree of the application that shows a word, presses buttons through AT-SPI's Action interface, and waits for words to appear in the tree:

```sh
dbus-run-session -- sh -c '
  gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus \
    --method org.freedesktop.DBus.Properties.Set org.a11y.Status IsEnabled "<true>"
  pleamar --scene tests/screen-reader.plm & sleep 4
  python3 tools/reader/atspi-check.py --press "Press me" --press Quiet --expect "pressed 1 · quiet 1"'
```

The scene has to be on a compositor: on a desktop, or in a headless pleamar-wm started inside the same `dbus-run-session`. A film (`--film`) has no reader. `src/a11y.rs`'s unit tests build the tree of `tests/agent.plm` and check its roles, names, states and actions, and that a change sends only what changed.

## Not done yet

- **Windows (UIA) and macOS (NSAccessibility)**: only compiled. AccessKit has both adapters, but pleamar does not open windows there yet (see [08](08-limitations.md), «Cross-platform»).
- **The lock screen** is not told. A person who cannot see it cannot unlock it with a reader. It needs a decision on what a reader may read there, and testing with Orca.
- **Moving through text**: AccessKit's text runs (characters, caret, selection) are not given, so a reader reads a field's words whole but cannot move through them. The same goes for announcements of something that changed on its own (live regions).
- **A slider's value as a number** only when its `value:` is one: `"50%"` is read as words, and SetValue with a number is not done.
- **Scrolling a list from the reader** (ScrollUp/Down, ScrollIntoView) is not done; pressing a row scrolls it into sight.
- **Where the window is**: on Wayland nobody knows, so boxes are relative to the surface, as AccessKit expects there.
- **Tried with**: an AT-SPI client under `dbus-run-session` with a headless pleamar-wm, on Linux. Not tried: Orca itself, a real desktop, popups.
