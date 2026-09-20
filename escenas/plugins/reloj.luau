-- The clock plugin's logic. It runs in its own Luau state: here `text.now`
-- is the scene's `Clock.now`, and nothing exists that is not the plugin's.
--
-- It spawns nothing: `clock.seconds` is a service, and it arrives on its own. With the
-- seconds hand off, the face only changes when the minute changes.

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
