-- La lógica de la barra: se entera de lo que pasa y se lo cuenta a la escena.
-- Nada de aquí anima ni coloca nada.

-- La hora no está aquí: la escena la pide con `service clock`, y llega sola.

-- ── los escritorios y la ventana: un servicio de la plataforma ───
--  `sys.watch` dice si este sistema lo tiene. Si no, la barra sale sin ellos.
-- Una ficha por monitor: su escritorio activo y la ventana que tiene delante. Cada
-- copia de la barra (`screens: each`) enseña la suya.
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
        -- El título es de quien tiene el foco: las demás barras no enseñan ventana ajena.
        fichas[k + 1] = { active = suyo, title = con_foco and titulo or "" }
    end
    model.mon = fichas
end

-- Cuál es cada copia lo dice el render en cuanto pone su superficie.
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
