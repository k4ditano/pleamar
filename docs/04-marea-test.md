# Test: does the real Marea fit in the language?


> The draft-language code in this note was first written with Spanish keywords and names; it is translated here, and where a draft word later became a real keyword, that keyword is used. It is still the draft, not valid syntax today. Names quoted from Marea's QML (`buscar`, `forma`, `caraLibre`, `_ultimoClic`…) are kept as they are in that file. The language that came out of it has English keywords.

**Date:** 2026-09-19 · **What was read:** `proyecto-marea/prototype/ExpressionController.qml`, all of it, at commit `ad95cf4`.

**Short verdict:** with sketch B's *flat states*, **it does not fit**. With three more pieces —**facts and events**, **layers with claims** and **gestures as timelines with a class**— **it fits whole, and it clears away two kinds of error** the file fights by hand today.

## 1. What the controller really is

It is not a state machine. It is five different things living together in a `QtObject`:

| What | How much | How it is done today |
| --- | --- | --- |
| A gesture player | 25 in-house gestures + the plugin ones | Keyframe sequences (`eyes`, `width`, `gap`, `tilt`, `sx`, `sy`, `lift`, `x`, `y`, `ms`, `hold`, `ease`). **They are already data.** |
| The eye shape (`forma`) | some 24 shapes, **43 writes** | A string anyone writes. Last writer wins. |
| Trimmings that are not `forma` | terminal ring, gauges, sweat, Remanso's pearl, ticks, orbit, glint, tint, lift 5 px | Loose booleans, each with its timer |
| A life of its own | blinking, gaze, breath at rest, sleeping breathing | Timers with randomness |
| Arbitration | who can override whom | Spread out: `reflejo()`, `ocupadaHasta()`, `caraLibre` (16 conditions), `insigniaCedida` in the view |

And three taxes paid across the whole file:

- **`reducedMotion` appears 56 times.** Almost every function has its "no motion" branch.
- **`model.sleeping` appears 35 times.** Almost every function starts by checking it is not asleep.
- **28 timers**, most of them to undo something after a while (`volverDeContenta`, `volverDeAviso`, `unsweat`, `unwiden`…).

A third of the file (679 lines) is comment, and it is the best thing it has: every design decision is reasoned out. **The language has to allow writing like that.**

## 2. The finding: the guard repeated eight times

This pattern appears **eight times**, almost letter for letter:

```js
function buscar(si) {
    //  Turn off only what was turned on here. Without this guard, opening ANY
    //  page wiped its face […] the red recording disc included.
    //  It is the only error that really matters in this class.
    if (!si && forma !== "lupa") return
    forma = si ? "lupa" : "ojos"
}
```

`buscar`, `soltando`, `grabar`, `sinDatos`, `comprobando`, `catalogando`, `enFaro`, `instalando`, `actualizando`. The code itself says what the problem is: **`forma` is a single variable and many want it.** The guard is a patch you have to remember to copy into every new function.

And there are places where it is missing. `confirmado()`, `noticeUrgent()`, `alDia()` and `noticeLanded()` write `forma = "contenta"` or `"aviso"` **without looking at what was there**, and their timer returns it to `"ojos"`. From what I read —I have not run it—, confirming something or getting an urgent notice *while recording* would take the red disc away: exactly the error the comments call "the only one that matters". If it is covered, it is in another file; I do not see it here.

**This is not a Marea bug, it is that the data model does not help.** And it is exactly what a language can fix at the root.

## 3. The three pieces needed

### 3.1 Facts and events — the boundary with the logic

Today the logic calls 41 functions of the controller (`onBuscando`, `onGrabacionCambiada`, `onPaquetesAlDia`…) and each one decides what to do with the face. In the language, the logic only **reports what happens**:

```
fact sleeping, recording, searching, cataloguing, in_lighthouse: bool
fact installing, updating, terminal_busy, do_not_disturb: bool
fact count: int = 0
fact dropping: symbol?           // nothing, or what kind of thing is handed over

event confirmed, urgent_notice, up_to_date, could_not, forgotten
event gesture(name, class)
```

A **fact** is something that is true for a while. An **event** happens in an instant. The logic touches nothing else: not `shape`, not poses, not timers.

### 3.2 Layers with claims — goodbye to the guard

A **layer** is a slot many can claim. The first claim that holds wins; when it stops holding, the next one is seen **on its own**.

```
layer shape {                         // from highest to lowest priority
    count        while count > 0
    rec          while recording
    camera       while in_camera
    drop         while dropping
    broken       from could_not until forgotten | installing | cataloguing
    notice       700ms after urgent_notice | quota_tight
    happy        620ms after confirmed | up_to_date | notices_in_a_row
    install      while installing
    update       while updating
    catalogue    while cataloguing
    lighthouse   while in_lighthouse
    magnifier    while searching
    clock        while checking_quotas
    no_data      while quotas_without_data and page == "bookings"
    from_plugin  while plugin_face and face.free
    eyes                              // what is left when nobody says anything
}
```

What changes:

- **The eight guards disappear.** Stopping a search cannot wipe the red disc: `rec` sits above it and never stopped claiming.
- **The `confirmado()`-while-recording bug cannot be written.** `happy` sits below `rec`; it claims, it is not seen, and it expires on its own.
- **The `volverDe…` timers disappear.** `700ms after urgent_notice` *is* the timer.
- **`wakeReconciliation` disappears.** It exists because face and model can fall out of sync; if the face *derives* from `sleeping`, they cannot.

⚠️ **The order above is mine**, deduced from the comments. It does not exist today —the last writer rules—, so it has to be reviewed line by line. That review is, in fact, the design the current file never had to make explicit.

What does **not** simplify as much: `caraLibre` has 16 conditions. Six become implicit through the order (recording, drop, forma ≠ ojos…); the other ten (`no_molestar`, gauges, sweat, tint, an open page with a face of its own…) are still a written condition. It is less, it is not magic.

### 3.3 Gestures: timelines with a class

Gestures **are already data** in Marea, so this is almost a transcription:

```
pose rest { eyes: 14; width: 6; gap: 16; turn: 0; sx: 1; sy: 1; lift: 0; look: 0 0 }

gesture nod class reflex {
    130ms OutQuad { look: 0 4; eyes: 10 }
    170ms OutBack { eyes: 15 }
}

gesture point(side) class asked {
    170ms         { look: 4*side 0; turn: 4*side; eyes: 18 }   hold 200ms
    190ms         { eyes: 16 }                                  hold 140ms
    110ms InQuad  { look: 4*side 0; turn: 5*side; eyes: 19; sx: 1.08; sy: 0.9; lift: 4 }
    …
}

posture working while tool_running { … }          // repeats on its own
```

Three rules of the language replace code that is hand-written today:

1. **Every keyframe starts from `rest`**: whatever it does not name goes back to its value. It is what `pose()` does with its twelve `=== undefined ?`.
2. **When it ends, it returns to the base pose of wherever it is** —`rest` or `asleep`—. It replaces `asentarLuego`, `_esPoseNeutra` and the `concat` that sticks the sleeping pose onto the end of any gesture.
3. **A gesture only interrupts another of its own class or lower**: `state > asked > reflex > posture > ambient`. It replaces `reflejo()`, `_ultimoFueReflejo`, `_finDelGesto` and `ocupadaHasta()`. It is the rule the comment on line 144 explains in twenty lines: "reflex gestures and asked-for ones are not worth the same".

A gesture can **emit events** in a keyframe —the camera needs `shot` exactly when the shutter closes— and **claim a layer** for as long as it lasts:

```
gesture photo class state, claim shape camera {
    210ms { }
    80ms  { shutter: 1 }    emit shot
    170ms { shutter: 0 }
    emit capture_done
}
```

### 3.4 And two minor things

**A life of its own, with randomness:**

```
every 2.5s..7s  while ambient { blink;  17%: again at 250ms }
every 5s..14s   while ambient and glances and not open {
    60%: look pointer | 40%: look random(±3.2, ±2);  release after 620ms..2120ms
}
every 12s..24s  while bare_rest { gesture breather(side: random(-1 | 1)) }
```

**Reduced motion, from the language and not from every function.** With `reduced motion` on, the runtime settles the springs instantly and, of each gesture, shows its **still face** —declared with `still { … }` or, failing that, the keyframe furthest from rest, which is what `_quietaDe()` computes today— for as long as it would have lasted. **The 56 branches go to zero.**

## 4. What does not fit, or fits with help

| What | Where it is | What to do |
| --- | --- | --- |
| Lightening a color so it reads (`_tinteLegible`) | l. 1139 | Color functions in expressions: `hsl()`, `lighten()` |
| "At most once every 30 s" (`lastGreeting`, `_ultimoClic`) | l. 1871, 862 | A rule modifier: `at most every 30s` |
| Picking a gesture from data (`_comoLoTraga`) | l. 838 | A ternary with symbols: `pieces > 1 ? batch : …` |
| Plugin gestures validated by a tool | `extraGestures` | They become files in the same language; validating is loading |
| **Deciding when a fact changes** | `MascotModel`, services | **Stays in the logic (Luau). That is its job.** |

None of this breaks the design. The last one is the boundary working as it should.

## 5. What the test asks of the runtime

This is the expensive part, and it is not the parser:

1. **Keyframe tracks with easing** (`OutBack`, `InQuad`…). Today pleamar only has springs.
2. **The layer evaluator**: claims, `while`, `N ms after`, `from … until`.
3. **The fact and event channel** logic → scene, and events scene → logic.
4. **Randomness and timed rules** in the renderer.
5. **Gestures with parameters** (`side`) and **symbols** in expressions.
6. **Rotation.** Shapes do not rotate yet, and `tilt` is in almost every gesture.
7. **More SDF primitives**: ring, arc, segment. The 24 eye shapes (camera, padlock, hourglass, moon…) do not come out of ellipses and boxes. **It is the biggest chunk.**

## 6. Size, by eye

Of the 2103 lines: 679 are comment (they stay, they are worth gold), some 250 are gesture data (they stay almost the same) and some 1170 are code. Of that code, most is guards, return timers, `reducedMotion` branches and `sleeping` checks. **I estimate 250–350 lines of the language** for the same thing. It is an estimate, not a measurement: the measurement will be writing it.

## 7. What this decides

- Sketch B falls short: `state` is not the central concept, **`layer` is**. A B `state` (rest/open) is a layer with two claims that also set properties.
- The language needs **two ways to animate**, not one: springs for what follows a value, keyframes for what tells a story.
- The next step with the most return **is not the parser**: it is points 1–3 of the runtime list, tested with Marea: `layer shape` with `rec`, `magnifier` and `happy`, and three gestures.
