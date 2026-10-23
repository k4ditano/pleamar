---
name: pleamar-desktop
description: Use the user's desktop in a pleamar-wm session — read and act on any window (a browser, an editor, a chat, a terminal) with the agent's own cursor and keyboard, while the user's mouse and keyboard stay theirs. Use when asked to do something in an app that is on screen ("go to reddit", "open X in the browser", "fill this form", "save that post", "look at what is on my screen", "run this in that terminal", "what did that terminal print"), and the session is pleamar-wm (`pleamar-wm agent windows` answers).
---
<!-- pleamar skill {VERSION}. Written by `pleamar --install-skill` and rewritten when pleamar updates: put your own notes in another skill. -->

# Using the desktop through pleamar-wm

pleamar-wm gives an agent hands of its own: a pointer and a keyboard on a seat
apart from the user's. What you do lands in the window you name even if it is
under another one or does not have the keyboard, and the user keeps working
meanwhile. They see you: your cursor is drawn (mint, with «agent» beside it),
the monitor you work on glows at its edges, and the window you read or touch
gets an outline.

It needs `agent on` in `~/.config/pleamar/session.conf` (and a session started
after that line was there). If `pleamar-wm agent windows` exits 69, tell the
user that line is what is missing; do not add it yourself unless they ask.

## The loop: what can it do, read, act, check

1. **Find the window and what it does**: `pleamar-wm agent windows`. Each
   window says `does ...`:
   - `tree`: it says what is on it (most programs: GTK, Qt, Firefox,
     Chromium): read it and act on its elements by `@ID`.
   - `scene`: a pleamar window: read it and act on its items by `@name`.
   - `text`: a terminal: read it as text and `run` commands in it.
   - `look`: pictures and coordinates; every window has it.
   - `nothing (private)`: a rule keeps it from you: ask the user.
   Use only what it does; anything else is refused (exit 3) with what it
   can do. `pleamar-wm agent can PID` says it for one window, with its verbs.
2. **Open what is not open** with `pleamar-wm agent open COMMAND`
   (`--monitor N` when the user names one), never from your own shell: from
   there it opens wherever the user's pointer is and takes their keyboard.
3. **Read before you look**: `tree PID`. Look only for what reading cannot
   tell: layout, an image, a colour, or a window that only does `look`.
4. **Act by element**: `press PID @24`, `type PID @12 -- "text"`; on a
   terminal, `run PID "CMD"`. Each action answers with what it changed.
5. **Check what happened** from that answer; `wait PID COND` for what takes
   time; `tree PID --since` for what changed since your last read. Do not
   read the whole window, or look, again and again.
6. **Decide by the exit code** (below).
7. **Done**: `pleamar-wm agent done`. While you work the user's monitor
   glows; without it the glow waits a minute and a half in case you are
   only thinking.

| you want to | `tree` / `scene` | `text` (terminal) | `look` only |
| --- | --- | --- | --- |
| see what is there | `tree PID` | `tree PID` | `look PID` |
| find something | `find PID --text T` | `find PID --text T` | `look PID` |
| press | `press PID @X` | `run`, `key` | `click PID X Y` |
| enter text | `type PID @X -- "..."` | `type PID -- "..."`, `key PID enter` | `click`, then `type PID -- "..."` |
| wait for it | `wait PID has --text T` | `wait PID prompt` | `wait PID settled 500ms` |
| what changed | the action's answer, `tree PID --since` | `tree PID --since` | `look` again |
| follow it | `listen PID` | `listen PID` | no |

## The words

- **A window**: `PID`, or `PID.N` for one of a program's several windows (a
  dialog is a window of its own: `windows` says «dialog of PID»).
- **An element**: `@24` (a number `tree` gave) or `@save` (a pleamar item's
  name; quote it when it has `#`: `'@hit#r3'`). A number stays the element's
  while it lives; one that is gone answers exit 4: read again.
- **A text to find**: always `--text "Save as"`, never a bare word. On
  `press`, `focus` and `look` it must be an element's whole name; several
  answer exit 6 with them listed (`--first` takes the first). `find --text`
  searches inside names and texts.
- **A file**: `-o FILE`.
- **A duration**: `500ms`, `5s`, `2m`; a bare number is seconds.
- **`--`** ends the options: what follows is text, even `@alice` or `--set`.
- **Text from stdin**: `type PID @12 -` and `run PID -` read it
  (`<<'EOF'`): nothing to quote twice.

## Reading

```sh
pleamar-wm agent tree PID                 # what is shown, one element a line: @ID role "name" X,Y WxH states, under its container
pleamar-wm agent tree PID --within @40    # only what is inside @40
pleamar-wm agent tree PID --role link     # only links (any role)
pleamar-wm agent tree PID --max 80        # at most 80 lines; the rest counted in `... more`
pleamar-wm agent tree PID --hidden        # what is not shown too (scrolled away, collapsed)
pleamar-wm agent tree PID --focused       # the element with the focus, and what holds it
pleamar-wm agent tree PID --since         # only what changed since your last tree of it: + came, - went, ~ changed
pleamar-wm agent find PID --text "Save"   # the elements that say it, with what holds them
pleamar-wm agent at PID X Y               # the element at that point of the picture
```

- `tree` answers the whole window and its version first (`tree 7`);
  `--since` names the read it compares with. Long lists of rows without
  names fold into one line; named rows never do: `find --text` or
  `--within` to reach what a fold hides.
- Boxes are in the window's picture coordinates: what `look` shows at
  (x, y) is where `click PID x y` lands.
- A program started before the session's accessibility bus, or one that
  builds its tree lazily, may say little: then `look`.

## Acting

```sh
pleamar-wm agent press PID @24                  # your cursor glides to it and clicks it (scrolled into view first when it can be)
pleamar-wm agent press PID --text "Save"        # the element named that
pleamar-wm agent press PID @24 --action 0       # its own action, without the pointer: for what cannot be clicked (covered, no box)
pleamar-wm agent focus PID @15                  # the focus in the window to it
pleamar-wm agent type PID @12 -- "Ana Lopez"    # focus it, then type, as a person would
pleamar-wm agent type PID @12 --set -- "..."    # replace its whole text at once (long text; a field that checks each key may not notice)
pleamar-wm agent key PID enter                  # tab escape backspace space up down left right delete home end pageup pagedown f1...f12
pleamar-wm agent hotkey PID ctrl+l
pleamar-wm agent press PID @24 --until has --text "Saved" 10s   # act, then wait for what should follow
```

- **Each action answers what it did and what it changed**: `pressed @24`,
  then the changes since just before it, as `tree --since` gives them (`+`
  came, `-` went, `~` changed; a terminal: its new lines). A page that
  navigated answers `full` and its first lines; a window that keeps drawing
  (a spinner) says `still drawing` and the changes so far. `--no-wait` acts
  and answers only what it did, when you do not need to know.
- **`--until COND`**: after a timeout (exit 5) the action did happen; do
  not do it again, read or wait.
- **Several steps on one window** in one call:

  ```sh
  pleamar-wm agent do PID <<'EOF'
  type @12 -- "Ana Lopez"
  type @13 -- "ana@example.com"
  press @24 --until has --text "Thanks" 10s
  EOF
  ```

  One command a line, quoted as in a shell, `#` for comments; it stops at
  the first that fails, says which, and exits with its code.
- **Coordinates** are for a window that only does `look`, or when `press`
  cannot (no box, covered): `look PID @X` to see it, then `click PID X Y`,
  `scroll PID X Y down 5`, `drag PID X1 Y1 X2 Y2`.
- `focus PID` (no element) gives the window the user's keyboard and shows
  its workspace: it moves what they see; rarely needed, say so if you do.

## Waiting

```sh
pleamar-wm agent wait PID has --text "Saved" [10s]   # until something shown says it; answers with it
pleamar-wm agent wait PID gone --text "Loading"      # until nothing does
pleamar-wm agent wait PID @24 gone                   # until that element goes (a spinner, a dialog's button)
pleamar-wm agent wait PID @24 not busy               # until it loses a state (or `@24 checked`, `@24 enabled`)
pleamar-wm agent wait PID focus @12                  # until the focus is there
pleamar-wm agent wait PID changed                    # until anything shown changes
pleamar-wm agent wait PID settled 500ms              # until nothing changed, nor the picture, for that long
pleamar-wm agent wait PID dialog                     # until the window opens a dialog
pleamar-wm agent listen PID [30s]                    # a line each time the focus moves, its text changes, a live region updates, the page announces something, or a dialog opens
```

- It answers `ok` and what met it, or exit 5 (60 s at most; 10 minutes for
  a terminal's `prompt`). A Stop ends it (exit 8).
- `settled` never holds on a window that animates (a spinner, a blinking
  caret): prefer `has` or a state.

## Pictures

```sh
pleamar-wm agent look PID [-o FILE]               # the window as it drew itself, with its menus and nothing over it; prints FILE WxH
pleamar-wm agent look PID X Y W H [-o FILE]       # a part of it
pleamar-wm agent look PID @24 [--pad 8]           # one element (scrolled into view first)
pleamar-wm agent look PID --text "Save"           # the element named that
pleamar-wm agent look PID --words "some words"    # just those words, where an element's text says them
pleamar-wm agent look PID --marks [-o FILE]       # the window with every element you can act on outlined and its @ID on it, and their records
```

- A window under another one or on another workspace is looked at as it
  is, without `focus`. «drawn Ns ago»: the program has not drawn for a while
  and may show more once it does.
- A menu (a right click, a dropdown) is in the picture of its window, and
  its items are in its `tree`.

## Terminals

A terminal whose shell runs through pleamar-wm's layer (`does text`) is read
as text, with its history, not as a picture:

```sh
pleamar-wm agent tree PID                     # `mark M` and its text as it is
pleamar-wm agent tree PID --history           # all it kept
pleamar-wm agent tree PID --since M           # only what came after mark M
pleamar-wm agent run PID "cargo test" [5m]    # typed, Enter, and what it printed once the shell waits again
pleamar-wm agent run PID - <<'EOF'            # a command with quotes in it
grep -r "a b" .
EOF
pleamar-wm agent wait PID prompt              # until the shell waits for a command again
pleamar-wm agent wait PID has --text "Finished"
pleamar-wm agent type PID -- "text"           # into the shell; a newline is Enter
pleamar-wm agent key PID ctrl+c
```

- `run` answers the command's last 200 lines and how many came before
  (`find PID --text T` searches the rest). On a timeout it answers its
  mark: go on with `wait PID prompt` or `tree PID --since M`.
- A terminal with several shells (tabs, a terminal server) names them in
  `windows` (`shells 7044,7051`): say which with `PID:SHELL` (`7002:7044`);
  without it you get exit 6 and the list.
- A program that fills the terminal (an editor, `htop`, another agent's
  TUI) keeps no history of its own: `tree` gives its screen, and `--since`
  gives the screen again when it rewrote what you had read.
- A terminal you were not asked to use is the user's: read it if the task
  needs it, type in it only when they said to.

## Pleamar windows (`does scene`)

Their items are named, and acting on one answers with what happened:
`press PID @save`, `type PID @query -- "words"`, `say PID ORDER` for any other
order (`drag knob.1 0 -40`, `hold X`, `wheel X 3`), `wait PID expr 'status ==
"Saved"'` for one of their own expressions. An item the scene keeps for the
user's hand (`persons` in its states) is refused: ask them to press it.

## Exit codes

| exit | means | then |
| --- | --- | --- |
| 0 | done | |
| 1 | another error | read the message |
| 2 | private window, or the screen is locked | ask the user; never another way around it (a picture of the whole desktop, `focus`) |
| 3 | not for this window | use what it does: the message says |
| 4 | the element is gone | read the window again |
| 5 | timeout | after an action: the action happened, the condition did not |
| 6 | ambiguous | name one of the choices listed |
| 7 | the window is gone | `windows` again |
| 8 | the user stopped you | stop, and say where you left it |
| 64 | wrong words | the message gives the right form |
| 69 | no session | tell the user (`agent on` in session.conf) |

## What works best

- **The keyboard before the mouse**, where there is a shortcut: in a browser
  `hotkey PID ctrl+l`, then `type PID -- "address"` and `key PID enter`.
- **Any text can be typed**: accents, ñ and emoji too.
- Things that open on their own (a translation offer, a cookie banner, a
  video that plays) are the page's, not yours: close them if they are in the
  way, and mention them.

## What is the user's to decide

You are acting as the user, in their accounts. **Before anything that
publishes, sends, buys, deletes, follows, likes, accepts terms or changes a
setting, stop and ask**, even if it looks like part of the task. Drafts,
searches, reading, opening and saving for later are fine. Never type a
password or a payment detail; leave that to them. If a page asks for a login
or a captcha, stop there and say so.

The user can stop you at any moment (the «Stop» on the monitor's pill, or
`pleamar-wm agent stop`): then what you try exits 8. Stop there, do not try
again, and say where you left it. Reading still answers then; doing does not.

How fast your hands move is the user's choice (`pleamar-wm agent pace`:
default, fast, instant; the pill says it when it is not default). Leave it as
it is unless they ask: at `default` they can follow your cursor.

When you finish, say what you did, step by step, and what you left for them.

## Other agents' tools

`pleamar-wm agent mcp` serves the same verbs as MCP tools (no quoting, exit
codes as `exit N` on a failure, pictures returned inline): prefer its tools
(`pleamar-desktop`) when your client has them. They are there once the user
ran `pleamar-wm agent setup`; do not run it yourself unless they ask.

The same hands also speak `cua-inject v1`, the protocol of
[Cua Driver](https://github.com/trycua/cua): programs started in the session
get `CUA_INJECT_SOCKET` and `CUA_DRIVER_RS_ENABLE_WAYLAND=1`.
`pleamar-wm agent raw LINE...` speaks the protocol directly.
