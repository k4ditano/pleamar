# The language — draft 0


> The code in this note was first written with Spanish keywords and names; it is translated here, and where a draft word later became a real keyword, that keyword is used. It is still the draft: none of it is meant to be valid syntax today. The language that came out of it has English keywords.

> **Historical.** This is the sketch, first written with Spanish keywords, from before implementing it. What actually works is in [The language — the guide](09-language-v0.md).

**Status:** draft, to discuss. None of this has a parser yet. It comes out of sketches A+B ([Three sketches for the language](06-sketches.md)) corrected by the test against Marea ([Test: does the real Marea fit in the language?](04-marea-test.md)).

**Still unnamed**, and with no file extension. The keywords were written in Spanish while that got decided ([Decisions and open questions](05-decisions.md)).

## What it is

A **declarative, non-Turing-complete domain-specific language** for describing desktop scenes: what is seen, how it moves and what it reacts to. Of the CSS, QML or Slint family, not the Lua or Rust one.

One sentence sums it up: **everything declared here is run by the renderer, on its own, at the cadence of the screen; the logic does no more than report what happens.**

## Principles

1. **Boring syntax, new semantics.** Braces, `name: expression`, like QML. It only departs from that where it brings something QML does not have.
2. **What is declared cannot hang.** No free loops, no recursion, no mutating variables. Everything terminates; everything can be checked before starting.
3. **Errors come out at load time, with line and column.** Naming a property that does not exist is not a runtime failure.
4. **The boundary with the logic is visible.** What runs in the renderer and what runs in the logic are not mixed up when reading the file.
5. **It is written with long comments.** Marea's `ExpressionController` is a third prose, and that is the best thing it has.

## The pieces

### 1. Elements — the tree (sketch A)

What is seen. A child is placed relative to its parent and clipped to it.

```
Body {
    color: #151616;  rim: 5%
    shadow: 0 10, blur 30, 34%

    Ellipse orb { x: 360 ~lively;  y: 90 ~lively;  radius: 28 + breath }
    Box panel   { left: orb.x + 62;  width: 0 ~calm;  height: 0 ~calm;  radius: 24
                  blend: 0 ~quick }
}
```

- `Body` blends its shapes into a single silhouette (smooth minimum) and fills it. `blend` is the radius of the water neck.
- Today there are `Ellipse`, `Box`, `Text` (by atlas). Missing: `Ring`, `Arc`, `Segment` and rotation.

### 2. Properties and springs

`name: value` is a constant or a binding. **`~spring` turns it into an animated property**: it has position and velocity, and when something changes its target, it heads there with that spring.

```
x: 360 ~lively            // a named spring
height: 0 ~spring(150, 23)   // stiffness, damping
```

Properties have a **global name** (`orb.x`). If the file reloads, the ones with the same name keep value and velocity: the animation does not jump.

### 3. Expressions

Pure, no effects. The renderer evaluates them every frame.

- Arithmetic, comparison, `and`/`or`/`not`, ternary `c ? a : b`.
- Functions: `min`, `max`, `clamp`, `abs`, `mix(a, b, t)`, `smooth(a, b, x)`, `random(a, b)`, and for color `hsl()`, `lighten()`.
- **`vel(prop)`**: the velocity of a spring. The logic could never know it; the renderer can.
- Units: `ms`, `s`, `%`, implicit `px`. Colors `#rrggbb`.

### 4. Facts and events — the boundary with the logic

```
fact recording, searching, sleeping: bool
fact count: int = 0
event confirmed, urgent_notice
event view_event  ->                  // this one goes out: from the scene to the logic
```

A **fact** is true for a while; an **event** happens in an instant. **It is the only thing that crosses**: the logic sets facts and emits events; the scene emits events back. The logic does not touch properties, poses or timers.

### 5. Layers — who wins

A layer is a slot many things claim. **The first claim that holds wins**; when it stops holding, the next one is seen on its own.

```
layer shape {
    rec        while recording
    notice     700ms after urgent_notice
    happy      620ms after confirmed
    magnifier  while searching
    eyes
}
```

Ways to claim: `while <condition>`, `<time> after <event>`, `from <event> until <event>`, and nothing at all (the default).

A claim can **set properties**, and then it is what sketch B called a *state*:

```
layer card {
    open while open? { orb.x: 140;  panel.width: 406;  panel.height: 190;  content: 1 }
    rest             { orb.x: 360;  panel.width: 0;    panel.height: 0;    content: 0 }
}
```

And a **transition** says with which spring and which delay it goes from one to the other:

```
transition rest -> open {
    panel.blend: 96 ~quick, and at 560ms 0 ~calm
    panel.width: ~calm at 70ms
    content:     ~calm at 300ms
}
```

### 6. Gestures — timelines

The other way to animate. A spring follows a value; a gesture tells a story.

```
pose rest { eyes: 14; width: 6; gap: 16; turn: 0; sx: 1; sy: 1; lift: 0; look: 0 0 }

gesture nod class reflex {
    130ms OutQuad { look: 0 4; eyes: 10 }
    170ms OutBack { eyes: 15 }
}
posture working while tool_running { … }
```

Rules:
1. Every keyframe starts from the rest pose: whatever it does not name goes back to its value.
2. When it ends it returns to the base pose of wherever it is (`rest`, `asleep`…).
3. **Classes**: `state > asked > reflex > posture > ambient`. A gesture only interrupts another of its own class or lower.
4. It can take parameters (`point(side)`), `emit` events in a keyframe and `claim` a layer for as long as it lasts.
5. `still { … }` declares its still face.

### 7. Rules — what makes things change

```
hover view                      => highlight: 1 ~quick          // the :hover of CSS
hover orb for 320ms             => open? = yes
away from set for 420ms         => open? = no
press view                      => open? = no, impulse orb.y -620, emit view_event
idle 14s while rest             => sleeping = yes
every 2.5s..7s while ambient    => blink;  17%: again at 250ms
```

Triggers: `hover`, `away from`, `press`, `idle`, `every a..b`, `on <event>`. Modifiers: `for`, `while`, `at most every`. The zones are the elements themselves: the renderer hit-tests with the same formula it paints them with.

### 8. Reduced motion

A runtime switch, not a per-scene one. When on: the springs settle instantly and each gesture shows its `still` face for as long as it would have lasted. In Marea today that is 56 hand-written branches.

### 9. The logic

Apart, in **Luau**, on a thread that can stall without it showing. It receives events, sets facts and emits events. Nothing else.

```lua
on("view_event", function() open_calendar() end)
on_change("notifications", function(n) fact.notices = #n end)
```

## What is left to design

- **Components and repetition.** `repeat i in 0..12 { … }` and reusable elements with parameters. Without this there is no notification list.
- **Layout.** Today the positions are hand-written expressions. Rows, columns and "take whatever is left".
- **Real text**: dynamic content, not a fixed bitmap.
- **Several files**: importing gestures, poses and components. Plugins are this.
- **Types**: `bool`, `int`, `real`, `color`, `symbol`, `time`. Inferred or written?
- **What happens when two layers set the same property.**
