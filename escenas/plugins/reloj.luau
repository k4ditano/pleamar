-- La lógica del plugin del reloj. Corre en su propio estado de Luau: aquí `text.now`
-- es el `Clock.now` de la escena, y no existe nada que no sea del plugin.

local function tick()
    run("date", { fact.seconds and "+%H:%M:%S|%a %d %b" or "+%H:%M|%a %d %b" }, function(out)
        local now, day = out:match("^(.-)|(.-)%s*$")
        if now then text.now = now; text.day = day end
    end)
end

tick()
every(1000, tick)
on("tapped", function() fact.seconds = not fact.seconds; tick() end)
