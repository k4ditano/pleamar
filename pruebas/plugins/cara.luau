-- La lógica del plugin de la cara: cuenta los toques, y se entera de cuando la escena la anima.
local n = 0
on("poked", function() n += 1; log("me han tocado", n) end)
on("cheer", function() log("la escena me anima") end)
