-- The notice tray. If this program can be the one that receives the
-- desktop notifications, it shows the real ones; if that place is already
-- taken by another (another bar), it runs a demo with made-up notices.
-- The list is handed over as it comes: `model.notes = lista`.

local real = sys.watch("notifications", function(list) model.notes = list end)

if real then
    -- Pressing a notice is its default action, if it has one; and either way, it goes.
    on("opened", function(i)
        local n = model.notes[i + 1]
        local default = false
        for _, a in ipairs(n.actions) do default = default or a.key == "default" end
        if default then sys.call("notifications.invoke", n.id, "default") else sys.call("notifications.dismiss", n.id) end
    end)
else
    log("another program already receives the notifications: here goes the demo")
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
        if n then log("dismissed:", n.title); model.notes = inbox end
    end)
    model.notes = inbox
end
