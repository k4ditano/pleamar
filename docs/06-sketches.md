# Three sketches for the language


> The code in this note was first written with Spanish keywords and names; it is translated here, and where a draft word later became a real keyword, that keyword is used. These are still sketches, not valid syntax today. The language that came out of it has English keywords.

All three describe the same piece of Marea —the body with its water neck, the
eyes, the button that lights up and open/close— and all three compile to the
same thing: the `Scene` of `src/scene.rs`. What changes is where each thing
lives and how much the renderer can do without the logic.

The keywords were in Spanish because the project was; if this ever left home,
that would be a separate decision.

## A · Tree — "like QML, but the springs belong to the language"

Nested elements with properties. `~` turns a property into a spring. A child is
placed relative to its parent and clipped to it. The logic goes apart, in Luau,
and is imperative just like today.

```
Body {
    color: #151616;  rim: 5%
    shadow: 0 10, blur 30, 34%

    Ellipse orb {
        x: 360 ~lively;  y: 90 ~lively
        radius: 28 + breath
        stretch: vel                        // deforms with its own velocity

        Eyes { gaze: pointer, up to 5 3.2;  blink: every 2.4..6s;  closed: sleep }
        breath: wave(1.7) * sleep * 1.3
        sleep: 0 ~slow
    }

    Box panel {
        left: orb.x + 62;  top: orb.y - height * 70/190
        width: 0 ~calm;  height: 0 ~calm;  radius: 24
        blend: 0 ~quick                     // water neck with the previous one
        content: 0 ~calm                    // the children's opacity

        Text { at: 26 78;  "Meeting in 5 min";  20px medium }
        Box view {
            center: 296 151;  size: 168+highlight*4  46+highlight*3;  radius: 9
            color: mix(#9ed6bd, #bdeed6, highlight)
            highlight: 0 ~quick
            hover { highlight: 1 }          // run by the renderer, no logic
            Text { centered; "View event"; 15px medium; #121f1a }
        }
    }
}

logic {
    local function open()
        animate(panel.blend, 96)
        animate(orb.x, 140)
        animate(panel.width, 406, { after = 70 })
        animate(panel.height, 190, { after = 110 })
        animate(panel.content, 1, { after = 300 })
        animate(panel.blend, 0, { after = 560, spring = calm })
    end
    on.hover(orb, 320, open)                -- 320 ms hovering
    on.press(view, function() close(true) end)
}
```

**For:** it reads like what it is, a drawing made of parts; relative coordinates
and clipping by the parent come out on their own, and from there to a layout is
one step. Anyone coming from QML reads it first time.
**Against:** the choreography is still imperative code. Opening depends on the
logic being alive to declare it: if it is stalled, Marea does not open.

## B · States — "the scene is a machine, and the renderer runs it"

The shapes are declared flat. The central things are the **states** —what value
each property has in each one—, the **transitions** —with which spring and which
delay it goes from one to another— and the **rules** —what makes it change
state—. All of that is run by the renderer. Only facts with meaning reach the logic.

```
prop orb.x, orb.y = 360, 90
prop panel.w, panel.h, fusion, content, sleep, highlight = 0

body #151616 rim 5% shadow(0 10, 30, 34%) {
    ellipse orb  at orb.x orb.y  radius 28+breath  stretch vel
    box   panel  from orb.x+62, orb.y - panel.h*70/190  size panel.w panel.h  radius 24
                 blend fusion * smooth(0, 40, panel.w)
}
inside panel, opacity content {
    box  view  center 296 151  size 168+highlight*4 46+highlight*3  radius 9
               color mix(#9ed6bd, #bdeed6, highlight)
    text "View event" centered on view  15px medium  #121f1a
}
eyes on orb  gaze pointer up to 5 3.2  blink every 2.4..6s  closed sleep

state rest   { orb.x: 360;  panel: 0 0;      content: 0 }
state open   { orb.x: 140;  panel: 406 190;  content: 1 }
state asleep : rest { sleep: 1 ~slow }

transition rest -> open {
    fusion:   96 ~quick, and at 560ms 0 ~calm
    orb.x:    ~lively
    panel.w:  ~calm  at 70ms
    panel.h:  ~calm  at 110ms
    content:  ~calm  at 300ms
}
transition open -> rest { … }

hover view                        => highlight: 1 ~quick
hover orb for 320ms               => open
press orb                         => toggle rest open
away from set for 420ms           => rest
press view                        => rest, impulse orb.y -620, emit "view-event"
idle 14s in rest                  => asleep
hover orb in asleep               => rest
```

**For:** Marea opens, closes, falls asleep and answers the mouse with the logic
dead. The logic is left with what is really its own: putting the text in,
deciding a notice has arrived, reacting to `"view-event"`. And it is checkable
before running: a state that names a property which does not exist is an error
with a line number.
**Against:** a flat state machine does not hold up against the real Marea. Its
`ExpressionController` is 2100 lines of "who wins": asleep, recording, urgent
notice, Remanso… It would need layered states with priority, and that has to be
designed well or it turns into another programming language through the back door.

## C · Luau — "no new language"

A library in Luau. `orb.x + 62` does not add: the metatables build the same
expression tree that Rust builds today with its operators. View and logic in a
single language, with its LSP, its formatter and its sandbox already done.

```lua
local e = scene "Marea"
local orb   = e:props("orb",   { x = 360, y = 90 })
local panel = e:props("panel", { w = { 0, calm }, h = { 0, calm } })
local fusion, content, highlight = e:prop("fusion", 0), e:prop("content", 0), e:prop("highlight", 0, quick)

local orb_shape   = ellipse { center = { orb.x, orb.y }, radius = 28 + e.breath, stretch = "vel" }
local panel_shape = box {
    from   = { orb.x + 62, orb.y - panel.h * 70 / 190 },
    size   = { panel.w, panel.h }, radius = 24,
    blend  = fusion * smooth(0, 40, panel.w),
}
e:body { color = "#151616", rim = 0.05, shadow = { 0, 10, 30, 0.34 }, orb_shape, panel_shape }

local view = box { center = { 296, 151 }, size = { 168 + highlight * 4, 46 + highlight * 3 }, radius = 9 }
e:inside(panel_shape, { opacity = content }, {
    solid { view, color = mix("#9ed6bd", "#bdeed6", highlight) },
    text { "View event", centered = view, px = 15 },
})
e:zone("view", view, { hover = { [highlight] = 1 } })

local function open()
    animate(fusion, 96, quick)
    animate(orb.x, 140)
    animate(panel.w, 406, calm, 70)
    -- …
end
e:on("hover", "orb", 320, open)
e:on("press", "view", function() close(true) end)
```

**For:** it is by far the cheapest —there is no parser to write— and the one an
AI writes best, since it knows Luau from Roblox. Loops and functions for free to
generate scenes (twelve icons in orbit are a `for`).
**Against:** the noise (`{ }`, commas, `local`) and the limits of metatables: `>`,
`and` and a ternary cannot be overloaded, so conditions inside expressions are
`cond(gt(a, b), x, y)`. The errors come out when it runs, not when it is read.
And nothing tells apart at a glance what runs in the renderer from what runs in
the logic, which is exactly the boundary this project wants to make visible.
