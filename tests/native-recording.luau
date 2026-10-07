local pending = false
text.phase = "idle"
every(16, function() fact.ticks += 1 end)
local function update(value, error)
    pending = false
    if error then text.phase = "error"; text.problem = error; log("recording test:", error); return end
    text.phase = value.state
    if value.path then text.path = value.path end
    if value.error then text.problem = value.error end
    fact.frames = value.frames or 0
end
on("begin", function()
    text.phase = "starting"
    pending = true
    sys.ask_async("recording.start", {}, update)
end)
on("stop", function() sys.ask_async("recording.stop", {}, update) end)
every(200, function()
    if not pending and text.phase ~= "idle" and text.phase ~= "error" and text.phase ~= "saved" and text.phase ~= "cancelled" then
        pending = true
        sys.ask_async("recording.state", {}, update)
    end
end)
