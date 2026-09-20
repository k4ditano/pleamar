-- La lógica de Marea. No sabe de muelles, de formas ni de coordenadas: dice qué
-- pone el aviso y que hay uno, y se entera de que alguien quiso verlo.

local notices = {
    { app = "CALENDARIO",      title = "Reunión en 5 min",             detail = "Revisión de diseño · 18:00" },
    { app = "MENSAJES",        title = "Lucía: ¿bajas a por café? ☕", detail = "Hace un momento" },
    { app = "ACTUALIZACIONES", title = "Hay 214 paquetes esperando a que alguien se decida a instalarlos", detail = "yay · 1,2 GB" },
}
local turn = 0

local function arrive()
    turn = turn % #notices + 1
    local n = notices[turn]
    text["notice.app"] = n.app
    text["notice.title"] = n.title
    text["notice.detail"] = n.detail
    text["notice.when"] = "Ahora"
    fact.asleep = false
    fact.open = true
    -- Montar el contenido de un aviso cuesta. Al render le da lo mismo.
    busy(600)
end

-- Sin ratón, `--demo` hace de aviso que llega y se va.
on("demo", function()
    if fact.open then fact.open = false else arrive() end
end)

on("view_event", function()
    log("alguien quiere ver:", text["notice.title"])
    -- Una orden del sistema: corre en otro hilo y contesta cuando acaba.
    run("date", { "+%H:%M:%S" }, function(out) log("…y eran las", out) end)
end)

on("fact:asleep", function(asleep) if asleep then log("se ha dormido") end end)
