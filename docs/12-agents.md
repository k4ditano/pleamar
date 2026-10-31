# Scenes an agent can read — design

**Status:** steps 1 to 7 (`describe`, acting by name, `wait`, `watch`, `label:`, `agent:`, and pleamar-wm's `agent tree/press`) implemented on 6 Oct 2026; the rest is design. Each step is measured with [the agent race](../tools/agent-race/results.md). Agreed with Abel on 6 Oct 2026 as the first step towards pleamar as a way to give **any program** its interface. This note is the first stretch of that road: what the scene already knows, handed to whoever drives it from outside.

## 1. Why

An agent that uses a desktop today works with pixels. `pleamar-wm agent look PID` gives it a picture, it guesses where the button is, `click PID 412 38` presses there, and then it looks again to find out what happened. The help of `pleamar-wm agent` ends with "look before each click: a page moves under you". It is slow, it spends most of its tokens on pictures, and it breaks when something moves.

Accessibility trees (AT-SPI, UIA) were meant to fix that, and they do so half-way: in GTK, Qt or a browser the tree is whatever each widget remembered to say, so agents end up falling back on the picture.

**A pleamar scene does not need to be guessed.** The compiler knows every zone with its name, every field, every text, every event and which rule fires it. The render knows, every frame, where each one is, whether it is active and whether something covers it. Nothing has to be added to a scene to get a tree: it is complete by construction, because it is the same data that draws it.

## 2. What it adds

Four things, all on the socket `--say` already uses:

1. **`describe`**: the scene as a tree of what can be read and touched.
2. **Acting by name**: `press save`, `type search factura`, through the same path a hand takes.
3. **`wait`** for a condition, and **`watch`** what changes: no more taking pictures in a loop to see whether it has finished.
4. **`agent: no`**: what the scene keeps for a person's hand.

And three small properties for when what is derived is not enough: `label:`, `role:`, `value:`.

## 3. The tree

Implemented (step 1, 6 Oct 2026). This is what it answers for a small notes window, with three rows in sight and two scrolled out of it:

```
$ pleamar --say notes describe
main · window «Notes» 640×480 · scale 1
  field   query   «Search notes»  ""  at 16,9 240×27
  field   pin     «PIN»  "(hidden)"  at 300,9 120×27
  list    list    at 16,60 608×120
    item    hit#r0  «October invoice»  at 16,60 608×38
    item    hit#r1  «Shopping list»  at 16,100 608×38
    item    hit#r2  «pleamar ideas»  at 16,140 608×38
    item    hit#r3  «Fourth note»  · off view  at 16,180 608×38
    item    hit#r4  «Fifth note»  · off view  at 16,220 608×38
  text            «No note open»  at 16,390 96×20
  button  new     «New note»  at 452,428 80×36
  button  save    «Save»  · inactive  at 544,428 80×36
  button  delete  «Delete»  · a person's  at 360,428 80×36
  button  close   at 600,14 20×20
```

`describe json` gives the same as data, with the rows of a list in its `children`. Asking costs one frame: the answer is made after the next list is composed, when the render knows where every text went (17 ms for Marea's 1169 zones).

| Part | Where it comes from |
| --- | --- |
| **header** | One per surface or popup on screen: its name (`main` for the scene's own), what it is (`window «title»`, `panel`, `popup`), its size and its scale. A copy per monitor says `(screen 0)` there, and its names lose the `#screen0` every one of them carries |
| **name** | The zone's or the field's own, as the scene knows it: `knob.2` for one written `knob.$k`, `hit#r3` for the `hit` of the fourth copy of a `for r`, `touch#TrayIcon431` for one inside a copy of a component. It is the name `press` will take (step 2) |
| **role** | Derived: an `input` is a `field`; a stack with `view:` a `list`; a zone with `on drag`, or with `on wheel` and no press, a `slider`; one with a press, `cursor: pointer` or `carries:`, a `button`, or an `item` if it is in a copy of a `for` or a `repeat`; another program's window held by the scene a `window`; anything else a `region`. And the words drawn outside every zone —a title, a status line— are a `text`, with no name: they are read, not touched. `role:` says it when the rules cannot: `button`, `toggle`, `slider`, `tab`, `link`, `item`, `list`, `region` |
| **label** | `label:` if it has one. If not, the texts drawn inside its box this frame, in reading order: each text goes to the smallest **active** zone it falls in, so a button's word is the button's and not the panel's around it, and a closed menu still in its place does not take the words of what is drawn there. A field without text says its `placeholder`, and a zone with nothing drawn in it what the scene's `NAME.title` says, if there is one (another program's window, `win.3.title`). **Nothing is guessed from what is near**: in Marea the slider's name is drawn under its icon, not under the slider, and a guess would have named the icon. When the word is not inside, `label:` says it |
| **value** | A field's text: `(hidden)` with `secret: true` or `agent: no`. For the rest, `value:`: a text with holes (`value: "{volume * 100} %"`) or a sum (`value: volume`), and a fact with names says its name (`critical`) |
| **state** | `inactive` when its `active:` is false **and** something is drawn in it (a greyed out button; an inactive zone with nothing in it is a closed panel's, and is left out), `covered by X` when another zone is on top at its centre, `off view` when its list has scrolled it out, `a person's` with `agent: no`; and with `checked:` `checked` or `not checked`, with `selected:` `selected` |
| **box** | In the surface's logical pixels, with the scale in the header: `pleamar-wm` turns it into the pixels of its `look` |
| **nesting** | The rows of a list hang from it, the ones in sight and the ones scrolled out, and so does whatever else falls in its window. Groups and components add no level: they are drawing, not meaning |

**What is not described.** A closed surface; a zone that is not there (`show:` false, or inside a hidden group); a surface with `captures: hidden`, because what is kept out of a screenshot is kept out of this too; anything with `agent: hidden` (section 10); and **never a `kind: lock`**: an agent does not get to read or touch the lock screen.

**A zone that says nothing is pointed at.** The first time the scene is asked, each zone that can be pressed and has neither `label:` nor a word inside it is named in the log, once, by the name it was written with: `agent  · 'close' can be pressed, but nothing says what it is: give it a \`label:\``. It is in the log and not in the compiler because whether a word falls inside is only known when it is drawn.

## 4. Acting by name

Implemented (step 2, 6 Oct 2026).

| Command | What it does |
| --- | --- |
| `press NAME [left\|right\|middle\|back\|forward] [N]` | The hand goes to the zone, presses and lets go, N times (1 to 3; 60 ms apart, so two are a double click) |
| `hold NAME` | Pressed for as long as its `on hold` asks, and a little more |
| `drag NAME DX DY` | Pressed at it, moved by DX, DY over half a second in twelve steps, let go |
| `wheel NAME N` | N notches over it; positive, upwards |
| `type FIELD TEXT` | The field gets TEXT as if it were typed over what it had: the logic hears `text:FIELD` |
| `key NAME` | A key, as the keyboard sends it: `escape`, `enter`, `tab`, `ctrl+z`, `a`. The field being typed in gets it first, then the `on key` rules |

`submit FIELD TEXT` was there before and stays: it sets the text and presses Enter without going through the field.

**It goes through the same path as a hand**, not round it. In a pleamar-wm session the agent's own cursor glides there first (section 7); then a `press` is the hand entering the zone in one frame, the button going down in the next, coming up 40 ms later, and the hand leaving: the zone's `hover` and `pressed` springs move, the touch ripples where a finger would, `pointer.x` and `local.x` have their values, its rules fire in their order and the logic hears `press:save` as it always does. While it lasts the pointer is the hand; the user's comes back after. That is the point: a scene does not have to be written twice, and what an agent does is what a person would have done. `emit` and `fact` stay for scripts that want to skip all that.

**The name** is the one `describe` gives: `hit#r3`, `knob.2`, and in a copy per monitor without its `#screen0`.

**A row scrolled out of its list is brought into sight first**, with the list's own spring, so whoever watches sees it glide there; then it is pressed. The answer starts with `scrolled hit#r4 into sight`.

**It refuses what a person could not do either**, says why, and does nothing: `? save is inactive`, `? save is covered by dialog`, `? delete is for a person's hand (agent: no): ask them to`, `? there is nothing called 'sve' on screen`. Something with `agent: hidden` is not found at all. A stack with `view:` is not covered by its own rows: its wheel and its drag reach it through them, as a hand's do.

**What happened** is the answer, read from when the hand leaves until the scene has been still for 150 ms —its rules, its logic— and never more than 1.2 s: the events that fired, the facts and texts that changed (a secret field's, or one kept for a person, only as «changed»), the lists that scrolled, and the surfaces and popups that opened or closed.

```
$ pleamar --say notes "press hit#r4"
scrolled hit#r4 into sight
pressed hit#r4
  event  open
  fact   sel: -1 → 4
  text   status: "No note open" → "Opened: Fifth note"
  scroll list: 0 → 82
```

Most of the time the agent does not need to look again. A press answers in 0.3 to 0.5 s, most of it the scene being given time to answer.

## 5. Waiting and watching

Implemented (step 3, 6 Oct 2026).

`wait CONDITION [TIMEOUT]` answers as soon as the condition holds, or says it did not after TIMEOUT (`2s`, `500ms`; 5 s if unsaid, a minute at most), with what the names it reads are worth then. The condition reads facts, texts and properties by name; numbers, quoted texts, `true`, `false` and an enum's values; `== != > < >= <=`, `has` for a text that holds another, `and`, `or`, `not` and brackets. It is checked against the scene's names first, so a typo gets a "did you mean…?" instead of a wait that never ends. It is looked at every frame, so it answers in the frame it becomes true:

```
$ pleamar --say notes 'wait status == "Saved"'
yes, after 794 ms
$ pleamar --say notes "wait dirty == true 1s"
? not after 1.0 s: dirty is false
$ pleamar --say notes 'wait sttus has "x"'
? there is no fact, text or property called 'sttus': did you mean 'status'?
```

It is not the language's own expressions: those need the compiler's names (`let`s, components), which a running scene no longer has. It is the part an agent asks about: what the scene says and holds.

`watch [SECONDS]` writes a line for each thing that happens, with when, for that long (10 s if unsaid): what a person or an agent presses, the events, and the same lines as "what happened". It costs nothing when nothing happens, and since every connection to the socket is served apart, an agent can watch with one and act with another:

```
$ pleamar --say notes "watch 3"
watching for 3 s
+0.50s  press  hit#r2
+0.50s  event  open
+0.52s  fact   sel: -1 → 2
+0.52s  text   status: "No note open" → "Opened: pleamar ideas"
done watching
```

### Following names, for a program: `watch 0 lay. free.`

What a person reads is no good to a program, and a scene that changes its mouse fact at every move would bury what it wants. Given **how the names start**, `watch` follows only those facts and texts, and writes one line each time one is worth something else:

```
$ pleamar --say wm watch 0 lay. free. agt. remote.
fact lay.0 3
fact free.1 1
text agt.5 claude · ~/x
fact remote.on true
```

- **`watch SECONDS START…`**: `0` is for as long as someone reads; any other number, that long (`30`, `30s`). Unquoted after the scene it is still one command (`--say wm watch 0 lay.`), and quoted as ever (`--say wm "watch 0 lay."`). `watch 0` with no start is an error; `watch` and `watch 30` alone are the `watch` for a person, as before.
- **The lines**, one per change, nothing else —no heading, no time, no last line—:
  - `fact NAME VALUE`: the value as `--say … fact` takes it back: `3`, `0.25`, `true`, an enum's name.
  - `text NAME VALUE`: everything after the second space, on one line: `\` is written `\\`, a line break `\n`, a carriage return `\r`.
  - `hidden NAME`: a text kept from agents (`secret: true`, `agent: no`) changed; what it says is not given.
  - `? …`: a note, never a value: `? no fact or text starts with 'X'`, once at the start, for a start that matches nothing (it may be a typo); or, alone, a `watch` asked wrong.
- **What they are worth to begin with** is not a change: only what changes from then on is written. With **`--now`** among the starts (`watch 0 --now lay.`), first what each one is worth, and then the changes: a program that needs both does not ask with `get` and miss what changed in between.
- **Each change once, in the frame it happens.** Several in the same frame, one line each, in the scene's order. A value that goes and comes back within a frame is not one.
- **A reader that does not keep up** is not given more and more memory: 1024 lines wait for it at most. Past that, what changes waits in the scene and is written when there is room, as it is then: what it was worth in between may be lost, never where it ended.
- **It ends cleanly when the reader goes.** Nothing is written while nothing changes, so every second the scene looks whether the other end of the socket is still there, and stops following when it is not; `pleamar --say` does the same with whoever reads what it prints (`… | head -1`). After a `watch` for some seconds, the connection just closes.
- **The scene loaded again** (hot reload): the same names are followed in the new one without repeating what was said; names new to it are written with what they are worth.
- **What it costs.** Nobody following: a look at an empty list each frame —no allocation, no lock—. Someone following: a comparison per followed name per frame, and a line built only for what changed; not the whole scene compared, as the `watch` for a person does.

## 6. `agent: no`: for a person's hand

```
box delete { from: …; size: …; agent: no }
box pay    { from: …; size: …; agent: no; label: "Pay {total} €" }
```

A zone with `agent: no` is described (an agent knows it is there and can tell the person to press it) but cannot be pressed through the socket. On a field, its value is not given either. **No toolkit has this today**, and it is what lets someone hand a program to an agent while keeping the final word over paying, deleting or sending.

**What it is and what it is not.** Through the socket it is a promise the commands keep. It is **not** a wall against a program that runs as the same user: that one could fake input in many ways. What makes it a wall is the compositor:

- **pleamar-wm** gives the agent **its own seat** (`cua-agent`). pleamar listens to the pointer of every seat and knows by its name which is an agent's; pleamar-wm, seeing the program has a pointer on that seat, sends the agent's input through it and not through the user's. **A press on an `agent: no` zone from an agent's seat is not let through**, nor the release after it; the log says so and `watch` writes `kept delete`. So clicking its pixels with `pleamar-wm agent click` does not get round it. Implemented (step 6, 6 Oct 2026), tried live: a click by pixels on «New note» went through, one on «Delete» did not.
- **Other compositors** have no agent seat: there it is only the promise.

## 7. In pleamar-wm

Implemented (step 5, 6 Oct 2026).

- A scene answers **`hello`** with who it is: `pleamar 0.2.25 · scene notes · pid 4521 · language 0.3`. pleamar-wm asks every socket in its programs' folder (`PLEAMAR_SOCKETS`) and so knows which window is which scene.
- **`pleamar-wm agent windows`** marks them: `· pleamar scene notes: tree, press`.
- By the window's PID, as everything else in `agent`: **`tree PID [json]`**, **`press PID NAME`**, **`wait PID CONDITION`**, **`watch PID [SECONDS]`**, and **`say PID ORDER`** for the rest (`type`, `drag`, `hold`, `wheel`, `key`). The agent does not need to know what the scene is called.
- **The press is seen, and the cursor is never behind it**: the scene's hand itself, before it goes down, glides the session's agent cursor (the mint one with «agent» beside it) to the point it will press, in 60 to 180 ms by how far, and waits for it to arrive (300 ms at most); in a drag the cursor goes with each step. So it happens however the press was asked —`pleamar-wm agent press`, `pleamar --say … press`—, and if the user has pressed «Stop» the press is refused: `? the user stopped the agent`.
- Every other window keeps `look` and `click`.
- **pleamar-wm's own shell is a pleamar scene**, so its socket (`session`, and `wm` beside its programs) answers `describe` and `press` too: each window as a `window` with its title, its title bar, its buttons («Close Notes», «Minimize Notes»), the dock's items by their program's name, the workspaces. Its zones say so with `label:`; the screen-sharing «Share» and the agent's own «Stop» are `agent: no`. (A desktop with no window draws nothing, and then there is nothing to tell.)

## 8. What comes after, and why the tree is shaped like this

- **Screen readers.** The same tree, handed to AccessKit, becomes AT-SPI on Linux, UIA on Windows and NSAccessibility on macOS. That is why the roles are a subset of AccessKit's (`button`, `slider`, `field`, `list`, `item`, `region`; later `toggle`, `tab`, `link`), and why `label:` is a text that translates.
- **MCP.** `pleamar mcp SCENE` would offer `describe`, the actions, `wait` and `watch` as tools, and each event with `->` as a tool of its own. Every pleamar program would be an MCP server with no work.
- **A program in any language** driving a scene over this same socket (step 3 of the road): the commands above are its first half.

## 9. Order of work

| # | Piece | Checked by |
| --- | --- | --- |
| 1 ✅ | `describe` with derived roles and labels, `label:`, `agent:`, the warning | `tests/agent*.plm` and `label-not-a-text.plm` in `run-tests.sh`; `src/agent.rs`'s tests build the tree of `tests/agent.plm`; by hand, a notes window and Marea in a headless pleamar-wm |
| 2 ✅ | `press`, `hold`, `drag`, `wheel`, `type`, `key`, with their refusals and "what happened" | `src/agent.rs`'s tests (reading an action, being refused); every action by hand on the race's window in a headless pleamar-wm; the race, live |
| 3 ✅ | `wait` and `watch` | `a_wait_reads_the_scene_as_it_is`; by hand in a headless pleamar-wm, watching while pressing; the race now saves, which takes 0.8 s |
| 4 ✅ | `agent: no` and `agent: hidden` over the socket | `a_hand_is_refused_what_a_person_could_not_do_either` |
| 5 ✅ | `hello`, and pleamar-wm's `agent tree/press/wait/watch/say` | In the live session: `windows` marking the race's window, `tree`, `press` with the cursor gliding there; the race |
| 6 ✅ | The seat: pleamar hears every seat and reads its name, pleamar-wm then uses the agent's for pleamar windows, the press is not let through | Live: `agent click` on «Delete» left `dirty` as it was, and `watch` said `kept delete`; on «New note» it went through |
| 7 ✅ | `role:`, `value:`, `checked:`, `selected:` | `a_scene_says_what_it_holds`, `role-not-a-role.plm` |

Steps 1 to 4 are pleamar alone and work on any compositor. 5 and 6 need both.

## 10. Decided

- **`agent: hidden`** exists: a zone, a field or a surface with it is not described at all, and cannot be acted on. For what is private but not secret, like the text of a notification. `agent: no` is seen and not touched; `agent: hidden` is not even seen.
- **Off view, it scrolls by itself**, as a person would: `press row.40` moves its `view:` with its own spring until the row is in sight, and then presses. Whoever is watching sees the list glide to it, not jump. The answer says so: `scrolled rows to row.40`.
- **Who may ask: the socket's owner.** It is the user's Unix socket in the runtime directory, as `--say` is today. Nothing more is asked for.
