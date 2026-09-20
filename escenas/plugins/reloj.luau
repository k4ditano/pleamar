-- La lógica del plugin del reloj. Corre en su propio estado de Luau: aquí `text.now`
-- es el `Clock.now` de la escena, y no existe nada que no sea del plugin.
--
-- No lanza nada: `clock.seconds` es un servicio, y llega solo. Con el segundero
-- apagado, la cara solo cambia cuando cambia el minuto.

local ultimo = ""

sys.watch("clock.seconds", function(t)
    local cara = fact.seconds and string.format("%02d:%02d:%02d", t.hour, t.minute, t.second) or t.time
    if cara ~= ultimo then
        ultimo = cara
        text.now = cara
        text.day = t.date
    end
end)

on("tapped", function()
    fact.seconds = not fact.seconds
    ultimo = ""
end)
