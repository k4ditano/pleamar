-- Never operate a user's application: only the dedicated owned fixture.
local key
assert(sys.watch("tray", function(list)
    fact.count = #list
    key = nil
    for _, item in ipairs(list) do
        if item.id == "tray-fixture" then key = item.key end
    end
    fact.present = key ~= nil
    text.status = key and "Owned tray fixture found" or "Waiting for the owned tray fixture"
end))
every(100, function() fact.ticks += 1 end)
local function act(name)
    if not key then text.status = "Start the tray-fixture example first"; return end
    text.status = "Request pending"
    sys.call_async(name, { key }, function(error, code)
        text.status = code == 0 and "Explorer accepted the request; verify the fixture event" or tostring(error)
        log(name, code, error)
    end)
end
on("activate", function() act("tray.activate") end)
on("context", function() act("tray.context") end)
