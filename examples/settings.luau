-- Save and remember. The scene has its own folder: `files.folder` says which one,
-- and there is no getting out of it. No paths, no `..`, nobody's `.ssh`.
--
-- What is saved is a table, and it reads back as a table: the JSON is put on and
-- taken off by the service.

local FILE = "settings.json"

local saved = sys.ask("files.read", FILE, "json") or {}
fact.tone = tonumber(saved.tone) or 0
fact.saves = tonumber(saved.saves) or 0
text.folder = sys.ask("files.folder")

local function save()
    sys.call("files.write", FILE, { tone = fact.tone, saves = fact.saves, at = os.time() })
end

on("pick", function(which)
    fact.tone = which
    fact.saves = fact.saves + 1
    save()
end)

-- If the file changes underneath —another copy of the scene, or an editor—, it reloads.
sys.watch("files:" .. FILE, function()
    local now = sys.ask("files.read", FILE, "json")
    if now then
        fact.tone = tonumber(now.tone) or fact.tone
        fact.saves = tonumber(now.saves) or fact.saves
    end
end)
