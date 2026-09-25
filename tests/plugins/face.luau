-- The face plugin's logic: it counts the pokes, and hears when the scene cheers it on.
local n = 0
on("poked", function() n += 1; log("I got poked", n) end)
on("cheer", function() log("the scene cheers me on") end)
