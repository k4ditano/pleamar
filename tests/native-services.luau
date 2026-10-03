-- Read-only hardware probe. Setting ready back to false on every Lua reload
-- verifies that an unchanged native subscription replays its current value.
fact.ready = false
assert(sys.watch("audio", function(value)
    fact.outputs = #(value.outputs or {})
    fact.ready = true
end))
fact.apps = #sys.ask("apps.list")
fact.media = type(sys.ask("media.state")) == "table"
fact.network = type(sys.ask("network.state")) == "table"
fact.wifi = type(sys.ask("network.wifi")) == "table"
fact.bluetooth = type(sys.ask("bluetooth.state")) == "table"
fact.brightness = type(sys.ask("brightness.state")) == "table"
assert(sys.watch("bluetooth", function(_) end))
assert(sys.watch("brightness", function(_) end))
assert(sys.watch("network.wifi", function(_) end))
assert(not pcall(sys.call, "audio.volume", 0/0))
fact.async_error = false
sys.call_async("audio.volume", { 0/0 }, function(error, code)
    fact.async_error = code ~= 0 and #error > 0
end)
fact.async_denied = not pcall(sys.call_async, "session.lock", {}, function() error("permission bypass") end)
fact.async_query = false
sys.ask_async("audio.state", {}, function(value, error)
    fact.async_query = error == nil and type(value.outputs) == "table"
end)
fact.async_query_error = false
sys.ask_async("audio.state", { "invalid argument" }, function(value, error)
    fact.async_query_error = value == nil and type(error) == "string" and #error > 0
end)
fact.async_query_denied = not pcall(sys.ask_async, "env", { "PATH" }, function() error("permission bypass") end)
fact.wallpaper = false
fact.wallpaper_preview = false
sys.ask_async("wallpaper.list", {}, function(value, error)
    assert(error == nil, error)
    fact.wallpaper = type(value.items) == "table" and #value.items > 0
    if value.items[1] then
        sys.ask_async("wallpaper.preview", { value.items[1].path, 1920, 1080 }, function(path, error)
            assert(error == nil, error)
            fact.wallpaper_preview = type(path) == "string" and #path > 0
        end)
    end
end)
