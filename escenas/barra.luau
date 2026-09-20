-- The bar's logic: it finds out what happens and tells the scene.
-- Nothing here animates or places anything.

-- The clock is not here: the scene asks for it with `service clock`, and it arrives on its own.

-- ── the workspaces and the window: a platform service ────────────
--  `sys.watch` says whether this system has it. If not, the bar comes without them.
-- One record per monitor: its active workspace and the window in front of it. Each
-- copy of the bar (`screens: each`) shows its own.
local monitores, escritorios, activo_global, titulo = {}, {}, 0, ""

local function repartir()
    local fichas = {}
    for k = 0, 3 do
        local donde = monitores[k]
        if not donde then break end
        local suyo, con_foco = 1, false
        for _, d in ipairs(escritorios) do
            if d.monitor == donde and d.active then
                suyo = d.id
                con_foco = d.id == activo_global
            end
        end
        -- The title belongs to whoever has the focus: the other bars do not show someone else's window.
        fichas[k + 1] = { active = suyo, title = con_foco and titulo or "" }
    end
    model.mon = fichas
end

-- The renderer says which copy is which as soon as it places its surface.
for k = 0, 3 do
    on("text:screen." .. k .. ".name", function(nombre) monitores[k] = nombre; repartir() end)
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
    escritorios, activo_global = w.list, w.active
    repartir()
end)
if not has then
    log("this system does not report its workspaces: the bar comes without them")
end

sys.watch("window", function(w) titulo = w.title; repartir() end)


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
