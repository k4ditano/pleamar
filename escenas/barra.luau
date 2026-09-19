-- La lógica de la barra: se entera de lo que pasa y se lo cuenta a la escena.
-- Nada de aquí anima ni coloca nada.

-- ── la hora ──────────────────────────────────────────────────────
local function tick()
    text.clock = os.date("%H:%M")
    text.day = os.date("%a %d %b")
end
tick()
every(1000, tick)

-- ── los escritorios y la ventana: un servicio de la plataforma ───
--  `sys.watch` dice si este sistema lo tiene. Si no, la barra sale sin ellos.
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
    fact["ws.active"] = w.active
end)
if not has then
    log("este sistema no cuenta sus escritorios: la barra sale sin ellos")
end

sys.watch("window", function(w) text.title = w.title end)

on("go", function(i) sys.call("workspaces.focus", i) end)

-- ── el volumen: un servicio, el mismo en todos los sistemas ──────
if not sys.watch("audio", function(a)
    fact.volume = a.volume
    fact.muted = a.muted
end) then
    log("este sistema no cuenta su volumen todavía")
end

-- ── y en la otra dirección: lo que se pide desde la barra ────────
on("volume_step", function(notches) sys.call("audio.step", 0.05 * notches) end)

--  Arrastrando llegan sesenta por segundo: se guarda el último y se manda como
--  mucho uno cada 50 ms.
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
