every(250, function() fact.ticks = fact.ticks + 1 end)
on("clicked", function() fact.clicks = fact.clicks + 1 end)
on("submitted", function() text.result = text.entry end)
on("submit:entry", function(value) text.result = value end)
