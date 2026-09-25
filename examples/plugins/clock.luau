-- The clock plugin's logic. It runs in its own Luau state: here `text.now`
-- is the scene's `Clock.now`, and nothing exists that is not the plugin's.
--
-- It spawns nothing: `clock.seconds` is a service, and it arrives on its own. With the
-- seconds hand off, the face only changes when the minute changes.

local last = ""

sys.watch("clock.seconds", function(t)
    local face = fact.seconds and string.format("%02d:%02d:%02d", t.hour, t.minute, t.second) or t.time
    if face ~= last then
        last = face
        text.now = face
        text.day = t.date
    end
end)

on("tapped", function()
    fact.seconds = not fact.seconds
    last = ""
end)
