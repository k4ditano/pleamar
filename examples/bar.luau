-- The bar's logic: it finds out what happens and tells the scene.
-- Nothing here animates or places anything.

-- The clock is not here: the scene asks for it with `service clock`, and it arrives on its own.

-- ── the workspaces and the window: a platform service ────────────
--  `sys.watch` says whether this system has it. If not, the bar comes without them.
-- One record per monitor: its active workspace and the window in front of it. Each
-- copy of the bar (`screens: each`) shows its own.
local monitors, desks, global_active, focused_title = {}, {}, 0, ""

local function distribute()
    local records = {}
    for k = 0, 3 do
        local place = monitors[k]
        if not place then break end
        local own, has_focus = 1, false
        for _, d in ipairs(desks) do
            if d.monitor == place and d.active then
                own = d.id
                has_focus = d.id == global_active
            end
        end
        -- The title belongs to whoever has the focus: the other bars do not show someone else's window.
        records[k + 1] = { active = own, title = has_focus and focused_title or "" }
    end
    model.mon = records
end

-- The renderer says which copy is which as soon as it places its surface.
for k = 0, 3 do
    on("text:screen." .. k .. ".name", function(name) monitors[k] = name; distribute() end)
end

local has = sys.watch("workspaces", function(w)
    local there, busy = {}, {}
    for _, d in ipairs(w.list) do
        there[d.id] = true
        busy[d.id] = d.windows > 0
    end
    for i = 1, 9 do
        fact["ws." .. i .. ".there"] = there[i] or i == w.active
        fact["ws." .. i .. ".busy"] = busy[i] or false
    end
    desks, global_active = w.list, w.active
    distribute()
end)
if not has then
    log("this system does not report its workspaces: the bar comes without them")
end

sys.watch("window", function(w) focused_title = w.title; distribute() end)


on("go", function(i) sys.call("workspaces.focus", i) end)

-- ── the volume: a service, the same on every system ──────────────
if not sys.watch("audio", function(a)
    fact.volume = a.volume
    fact.muted = a.muted
end) then
    log("this system does not report its volume yet")
end

-- ── and the other way round: what the bar asks for ───────────────
on("volume_step", function(notches) sys.call("audio.step", 0.05 * notches) end)

--  Dragging sends sixty a second: the last one is kept and at most one goes out
--  every 50 ms.
local wanted, sending = nil, false
on("volume_set", function(v)
    wanted = v
    if sending then return end
    sending = true
    after(50, function()
        sending = false
        sys.call("audio.volume", wanted)
    end)
end)

on("mute", function() sys.call("audio.mute") end)
