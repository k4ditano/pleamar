# Why it exists

## The problem

Quickshell is QML on Qt, and QtQuick animates on the same thread the logic runs on. A `Behavior` or a `SpringAnimation` stops if a JavaScript handler takes its time. And it takes its time exactly when it shows most: **opening a panel**, which is when things get instantiated, read and parsed.

This is not a Quickshell bug, and it is not fixed by swapping C++ for Rust: it is a QtQuick design decision, made for applications and not for desktops.

## The idea

Taken from Core Animation (iOS), which is why the Dynamic Island never stalls: **the view declares transitions and someone else runs them.**

- The **logic** says "the width goes to 406 with this spring, in 70 ms" and forgets about it.
- The **renderer**, on its own thread, walks that intent at the screen's cadence no matter what.

Everything else follows from there: if the renderer has to be able to work alone, what it is given must be **data and pure expressions**, not code. And that is a language.

## What was measured

RTX 2060, 60 Hz screen. Opening Marea's card with the logic blocked for 600 ms right at the start:

| | frames during the block | longest frame |
| --- | --- | --- |
| pleamar | 38 | 17–19 ms |
| pleamar `--ingenuo` (logic on the thread that paints) | 0 | 617 ms |
| QtQuick on Quickshell (`comparar/shell.qml`) | 0 | 600 ms |

Memory at rest: 119 MB against the 214 MB of the Quickshell bench. **Half, not a quarter**: almost everything pleamar holds is NVIDIA's Vulkan driver. Painting the static parts on the CPU would be the way to go below that.

## The five ideas of the complete design

Only 2 and half of 3 are done.

1. **State lives outside the view.** Reloading means swapping the function, not losing the state.
2. **Animations are not run by your code.** ← what the prototype proves
3. **A renderer for this domain**: SDF shapes. Turning one shape into another is interpolating two formulas.
4. **The system as a lazy data graph**: `audio.output.volume`. If nobody looks at it, it does not even connect.
5. **Plugins are actors with permissions**: isolated VM, CPU budget, never on the thread that paints.

## What this is not

It does not replace k4 or Marea. This is 2000 lines of prototype against fifteen years of QtQuick: text, IME, accessibility, multi-monitor, tray… A prototype that impresses is months; something to use daily, years. **The realistic path for k4 and Marea today is still a Rust + Lua daemon beside Quickshell.** pleamar is the question of what would have to be built if nothing existed.
