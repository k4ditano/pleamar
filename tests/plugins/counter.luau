-- The test plugin's logic: it only exists so the library is a plugin.
on("bumped", function() text.label = "more" end)
