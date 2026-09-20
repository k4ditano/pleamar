-- Guardar y recordar. La escena tiene su propia carpeta: `files.folder` dice cuál,
-- y de ahí no se sale. Nada de rutas, nada de `..`, nada del `.ssh` de nadie.
--
-- Lo que se guarda es una tabla, y se lee como una tabla: el JSON lo pone y lo
-- quita el servicio.

local FICHERO = "settings.json"

local guardado = sys.ask("files.read", FICHERO, "json") or {}
fact.tone = tonumber(guardado.tone) or 0
fact.saves = tonumber(guardado.saves) or 0
text.folder = sys.ask("files.folder")

local function guardar()
    sys.call("files.write", FICHERO, { tone = fact.tone, saves = fact.saves, at = os.time() })
end

on("pick", function(cual)
    fact.tone = cual
    fact.saves = fact.saves + 1
    guardar()
end)

-- Si el fichero cambia por debajo —otra copia de la escena, o un editor—, se recarga.
sys.watch("files:" .. FICHERO, function()
    local ahora = sys.ask("files.read", FICHERO, "json")
    if ahora then
        fact.tone = tonumber(ahora.tone) or fact.tone
        fact.saves = tonumber(ahora.saves) or fact.saves
    end
end)
