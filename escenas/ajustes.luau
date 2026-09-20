-- Save and remember. The scene has its own folder: `files.folder` says which one,
-- and there is no getting out of it. No paths, no `..`, nobody's `.ssh`.
--
-- What is saved is a table, and it reads back as a table: the JSON is put on and
-- taken off by the service.

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

-- If the file changes underneath —another copy of the scene, or an editor—, it reloads.
sys.watch("files:" .. FICHERO, function()
    local ahora = sys.ask("files.read", FICHERO, "json")
    if ahora then
        fact.tone = tonumber(ahora.tone) or fact.tone
        fact.saves = tonumber(ahora.saves) or fact.saves
    end
end)
