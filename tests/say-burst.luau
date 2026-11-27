-- At each `x` it writes down what `x.v` says; with `text:x.v`, what it was told, one by one.
local seen, heard, counted = {}, {}, {}
on("x", function()
    seen[#seen + 1] = text["x.v"]
    text.out = table.concat(seen, "|")
    counted[#counted + 1] = tostring(fact["x.n"])
    text.facts = table.concat(counted, "|")
end)
on("text:x.v", function(v)
    heard[#heard + 1] = v
    text.heard = table.concat(heard, "|")
end)
