-- La bandeja de avisos. Si este programa puede ser quien recibe las
-- notificaciones del escritorio, enseña las de verdad; si ese sitio ya lo
-- ocupa otro (otra barra), hace una demostración con avisos de mentira.
-- La escena tiene cinco huecos; cuántos se ven y qué pone cada uno lo dice esto.

local inbox = {}

local function show()
    for i = 1, 5 do
        local n = inbox[i]
        text["note." .. (i - 1) .. ".app"] = n and n.app:upper() or ""
        text["note." .. (i - 1) .. ".title"] = n and (n.body ~= "" and (n.title .. " · " .. n.body) or n.title) or ""
    end
    fact.count = math.min(#inbox, 5)
    fact.more = #inbox > 3
end

local real = sys.watch("notifications", function(list) inbox = list; show() end)

if real then
    -- Pulsar un aviso es su acción por defecto, si la tiene; y en todo caso, se va.
    on("opened", function(i)
        local n = inbox[i + 1]
        if not n then return end
        local default = false
        for _, a in ipairs(n.actions) do default = default or a.key == "default" end
        if default then sys.call("notifications.invoke", n.id, "default") else sys.call("notifications.dismiss", n.id) end
    end)
else
    log("las notificaciones ya las recibe otro programa: va la demostración")
    inbox = {
        { app = "Calendario", title = "Reunión en 5 min", body = "Revisión de diseño" },
        { app = "Mensajes",   title = "Lucía", body = "¿bajas a por café? ☕" },
    }
    local pending = {
        { app = "Actualizaciones", title = "Hay 214 paquetes esperando a que alguien se decida a instalarlos", body = "" },
        { app = "Música",          title = "Tycho", body = "Awake" },
        { app = "Batería",         title = "Queda un 12 %", body = "" },
        { app = "Mensajes",        title = "Lucía", body = "venga, que se enfría" },
    }
    every(1600, function()
        local n = table.remove(pending, 1)
        if n then table.insert(inbox, 1, n); show() end
    end)
    on("opened", function(i)
        local n = table.remove(inbox, i + 1)
        if n then log("despachado:", n.title); show() end
    end)
    show()
end
