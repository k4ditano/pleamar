-- Guardar y recordar. La escena tiene su propia carpeta: `files.folder` dice cuál,
-- y de ahí no se sale. Nada de rutas, nada de `..`, nada del `.ssh` de nadie.

local FICHERO = "settings.txt"

-- Un formato sin pretensiones: una línea por cosa, `clave=valor`.
local function leer()
    local guardado = sys.ask("files.read", FICHERO)
    local ajustes = {}
    if guardado then
        for clave, valor in string.gmatch(guardado, "([%w_]+)=([^\n]*)") do
            ajustes[clave] = valor
        end
    end
    return ajustes
end

local ajustes = leer()
fact.tone = tonumber(ajustes.tone) or 0
fact.saves = tonumber(ajustes.saves) or 0
text.folder = sys.ask("files.folder")

local function guardar()
    sys.call("files.write", FICHERO, string.format("tone=%d\nsaves=%d\n", fact.tone, fact.saves))
end

on("pick", function(cual)
    fact.tone = cual
    fact.saves = fact.saves + 1
    guardar()
end)

-- Si el fichero cambia por debajo —otra copia de la escena, o un editor—, se recarga.
sys.watch("files:" .. FICHERO, function()
    local ahora = leer()
    fact.tone = tonumber(ahora.tone) or fact.tone
    fact.saves = tonumber(ahora.saves) or fact.saves
end)
