text.status = "Ready from Luau"
fact.ready = true
on("refresh", function()
    text.status = "Updated from Luau"
    fact.changed = true
end)
