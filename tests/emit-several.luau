on("point", function(p) fact["got.x"] = p.x; fact["got.y"] = p.y end)
on("pair", function(t) fact["got.sum"] = t[1] * 10 + t[2] end)
