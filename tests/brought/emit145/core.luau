on("height", function(v) fact.got = (type(v) == "table" and tonumber(v.v)) or -1 end)
on("close", function() fact.got = fact.got + 100 end)
