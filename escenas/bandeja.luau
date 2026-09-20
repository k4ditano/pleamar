-- La bandeja de avisos. Si este programa puede ser quien recibe las
-- notificaciones del escritorio, enseña las de verdad; si ese sitio ya lo
-- ocupa otro (otra barra), hace una demostración con avisos de mentira.
-- La lista se entrega tal cual: `model.notes = lista`.

local real = sys.watch("notifications", function(list) model.notes = list end)

if real then
    -- Pulsar un aviso es su acción por defecto, si la tiene; y en todo caso, se va.
    on("opened", function(i)
        local n = model.notes[i + 1]
        local default = false
        for _, a in ipairs(n.actions) do default = default or a.key == "default" end
        if default then sys.call("notifications.invoke", n.id, "default") else sys.call("notifications.dismiss", n.id) end
    end)
else
    log("las notificaciones ya las recibe otro programa: va la demostración")
    local inbox = {
        { app = "Calendario", title = "Reunión en 5 min", body = "Revisión de diseño" },
        { app = "Mensajes",   title = "Lucía", body = "¿bajas a por café? ☕" },
    }
    local pending = {
        { app = "Actualizaciones", title = "Hay 214 paquetes esperando a que alguien se decida a instalarlos", body = "", urgency = "low" },
        { app = "Música",          title = "Tycho", body = "Awake" },
        { app = "Batería",         title = "Queda un 12 %", body = "", urgency = "critical" },
        { app = "Mensajes",        title = "Lucía", body = "venga, que se enfría" },
    }
    every(1600, function()
        local n = table.remove(pending, 1)
        if n then table.insert(inbox, 1, n); model.notes = inbox end
    end)
    on("opened", function(i)
        local n = table.remove(inbox, i + 1)
        if n then log("despachado:", n.title); model.notes = inbox end
    end)
    model.notes = inbox
end
