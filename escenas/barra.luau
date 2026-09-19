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

-- ── el volumen: una orden que no acaba, y otra que contesta ──────
local waiting = false
local function volume()
    waiting = false
    run("wpctl", { "get-volume", "@DEFAULT_AUDIO_SINK@" }, function(out)
        local v = tonumber(out:match("([%d%.]+)"))
        if v then fact.volume = v end
        fact.muted = out:find("MUTED") ~= nil
    end)
end
volume()
--  `pactl subscribe` escribe una línea por cada cosa que cambia en el audio, y
--  al mover el volumen son muchas: se junta lo de 40 ms en una sola pregunta.
spawn("pactl", { "subscribe" }, function(line)
    if line:find("sink") and not waiting then
        waiting = true
        after(40, volume)
    end
end)

-- ── y en la otra dirección: lo que se pide desde la barra ────────
local sink = "@DEFAULT_AUDIO_SINK@"

on("volume_step", function(notches)
    local step = string.format("%d%%%s", 5 * math.abs(notches), notches > 0 and "+" or "-")
    run("wpctl", { "set-volume", "-l", "1.0", sink, step })
end)

--  Arrastrando llegan sesenta por segundo, y cada uno sería un proceso: se
--  guarda el último y se manda como mucho uno cada 50 ms.
local wanted, sending = nil, false
on("volume_set", function(v)
    wanted = v
    if sending then return end
    sending = true
    after(50, function()
        sending = false
        run("wpctl", { "set-volume", sink, string.format("%.2f", wanted) })
    end)
end)

on("mute", function() run("wpctl", { "set-mute", sink, "toggle" }) end)
