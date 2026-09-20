# From zero to a bar

This guide writes a real bar, one piece at a time. Everything here compiles:
`./probar.sh` checks it on every change, so if any of this stops being true, the
tests go red.

The program has to be built:

```sh
cargo build --release
alias pleamar=$PWD/target/release/pleamar
```

## 1. A window and a box

A scene is a `.plm` file. The first thing it says is which window it asks for.

```plm
language 0.1
scene Bar {
    surface { size: full, 40; anchor: top }
    box { from: 0, 0; size: 1920, 40; color: #151616 }
}
```

`pleamar --escena bar.plm`. It sticks to the top and takes the whole width
(`full`). Save the file after any change and it reloads on its own, without
losing whatever was in motion.

Get something wrong and it will not launch: it says which line, points at it
with an arrow, and if the word looks like something it knows, adds a "did you
mean…?".

## 2. Letting the system report things

The time needs no logic. It is asked for by name.

```plm
language 0.1
scene Bar {
    surface { size: full, 40; anchor: top }
    permissions { services: "clock" }
    service clock as now { time: text = "--:--"; date: text }

    box { from: 0, 0; size: 1920, 40; color: #151616 }
    text now.time { at: 960, 20; anchor: center; size: 14; weight: 600; color: #f5f7f5 }
    text now.date { at: 1010, 20; anchor: left center; size: 12; color: #f5f7f5; opacity: 50% }
}
```

Two things worth understanding from the start:

- **Without `permissions`, nothing.** Drop that line and the scene still loads,
  and the console says what it is missing. Nothing slips through unwritten.
- What a service reports is in the reference, and **asking it for something it
  does not have is an error on load**, not a blank gap.

Along with the clock there are `audio`, `battery`, `network`, `media` and
`window`.

## 3. Something that moves

A property (`prop`) is a spring. You do not set its value: you say where it is
going, and it travels.

```plm
language 0.1
scene Bar {
    surface { size: full, 40; anchor: top }
    let ink = #f5f7f5

    prop hot = 0 ~quick

    box { from: 0, 0; size: 1920, 40; color: #151616 }
    box { at: 60, 20; size: 90 + hot * 30, 26; corner: 13; color: mix(#2b2d2d, #9ed6bd, hot) }
    text "hello" { at: 60, 20; anchor: center; size: 12.5; color: mix(ink, #10221b, hot) }

    zone box pill { at: 60, 20; size: 120, 30; corner: 15; cursor: pointer }
    on enter pill { hot: 1 }
    on leave pill { hot: 0 }
}
```

The highlight is the renderer's work, not the logic's. With the logic blocked for
five seconds, the pill still answers the mouse.

The house springs are `lively`, `calm`, `quick`, `slow`, `gentle` and `pose`, and
`spring mine = 170, 12` declares another.

## 4. Logic, when it is needed

Put a `bar.luau` next to `bar.plm` and that is its logic. It can only cross the
boundary the scene declares: facts, texts, models and events.

```plm
language 0.1
scene Bar {
    surface { size: full, 40; anchor: top }
    fact howmany = 0
    text title = "nothing"
    event reload ->

    box { from: 0, 0; size: 1920, 40; color: #151616 }
    text title { at: 20, 20; anchor: left center; size: 12.5; color: #f5f7f5 }
    ellipse { at: 300, 20; radius: 4 + howmany; color: #9ed6bd }

    zone box whole { from: 0, 0; size: 1920, 40 }
    on press whole { emit reload }
}
```

```lua
-- bar.luau
text.title = "ready"
fact.howmany = 3

on("reload", function()
    fact.howmany = (fact.howmany + 1) % 8
end)

every(5000, function() text.title = os.date("%H:%M:%S") end)
```

The logic **does not draw**. It reports what happens; what is seen belongs to the
scene. If a handler does not finish in two seconds it gets cut off, and the
screen never notices.

## 5. A list

A model is a list of records that the logic fills. The scene says how many fit
and what they hold.

```plm
language 0.1
scene Bar {
    surface { size: 420, 260; anchor: top }
    let ink = #f5f7f5
    model notices max 8 { title: text; body: text; urgent: bool = false }

    box { from: 0, 0; size: 420, 260; corner: 18; color: #151616 }
    column list {
        at: 16, 16
        gap: 8
        for n in notices {
            group {
                size: 388, 46
                box { from: 0, 0; size: 388, 46; corner: 12; color: #202222 }
                ellipse { at: 20, 23; radius: 4; color: #e86a9a; opacity: n.urgent * 100% }
                text n.title { at: 36, 17; anchor: left center; size: 12.5; color: ink }
                text n.body { at: 36, 33; anchor: left center; size: 11; color: ink; opacity: 50% }
            }
        }
    }
}
```

```lua
-- the logic sends the whole list at once
model.notices = {
    { title = "Meeting", body = "in five minutes", urgent = true },
    { title = "Backup done", body = "412 files" },
}
```

What is not there takes no room: if the list brings two records, the other six do
not exist, and the ones left move into place with the layout's spring.

For lists of thousands, the scene declares a window of records and the logic
sends the slice that fits. That is in [recipes](recipes.md#a-list-of-thousands).

## 6. Repeating without repeating yourself

`component` is something to copy, with typed parameters.

```plm
language 0.1
scene Bar {
    surface { size: full, 40; anchor: top }
    let ink = #f5f7f5
    let mint = #9ed6bd

    component Pill(label: text, picked: event, tone: color = mint) {
        size: 96, 28
        prop glow = 0 ~quick

        zone box touch { from: 0, 0; size: 96, 28; corner: 14; cursor: pointer }
        box { from: 0, 0; size: 96, 28; corner: 14; color: mix(#2b2d2d, tone, glow) }
        text label { at: 48, 14; anchor: center; size: 12; color: mix(ink, #10221b, glow) }

        on enter touch { glow: 1 }
        on leave touch { glow: 0 }
        on press touch { emit picked }
    }

    event choose ->
    box { from: 0, 0; size: 1920, 40; color: #151616 }
    row { at: 20, 6; gap: 10
        Pill("one", picked: choose)
        Pill("two", picked: choose, tone: #e8c07a)
        Pill("three", picked: choose)
    }
}
```

Each copy has **its own** properties and **its own** zones: one pill's `glow` is
not the other's. Handing an event to a component is what gives it permission to
emit that event, and it can touch nothing else of the scene.

## 7. What next

- Patterns that already work, ready to copy: [recipes](recipes.md).
- Everything that exists, with its exact shape:
  [the reference](11-referencia-del-lenguaje.md).
- What the logic can do: [the Luau boundary](10-logica-luau.md).
- In the editor: `pleamar --lsp` gives errors as you type, and
  `pleamar --resaltado vim` the highlighting. See [`editor/`](../editor/).

And a habit that saves trouble: **`pleamar --comprobar yours.plm`** before
launching it. Reads it, says whether it is fine, exits.
