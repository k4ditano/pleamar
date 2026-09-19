-- La bandeja, con datos de verdad: una lista en Luau que crece y encoge. La
-- escena tiene cinco huecos; cuántos se ven y qué pone cada uno lo dice esto.

local inbox = {
    { app = "CALENDARIO", title = "Reunión en 5 min · Revisión de diseño" },
    { app = "MENSAJES",   title = "Lucía: ¿bajas a por café? ☕" },
}
local pending = {
    { app = "ACTUALIZACIONES", title = "Hay 214 paquetes esperando a que alguien se decida a instalarlos" },
    { app = "MÚSICA",          title = "Tycho — Awake" },
    { app = "BATERÍA",         title = "Queda un 12 %" },
    { app = "MENSAJES",        title = "Lucía: venga, que se enfría" },
}

local function show()
    for i = 1, 5 do
        local n = inbox[i]
        text["note." .. (i - 1) .. ".app"] = n and n.app or ""
        text["note." .. (i - 1) .. ".title"] = n and n.title or ""
    end
    fact.count = math.min(#inbox, 5)
    fact.more = #inbox > 3
end

-- Llega uno nuevo, arriba del todo.
every(1600, function()
    local n = table.remove(pending, 1)
    if n then table.insert(inbox, 1, n); show() end
end)

-- Pulsar un aviso lo despacha: sale de la lista y los de abajo suben.
on("opened", function(i)
    local n = table.remove(inbox, i + 1)
    if n then log("despachado:", n.title); show() end
end)

show()
