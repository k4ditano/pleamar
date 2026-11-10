-- What a paste brings, as the logic gets it: no text here (the film has no clipboard).
on("paste:draft", function(p)
    fact.heard = (p.text == nil and p.image == false and type(p.uris) == "table" and #p.uris == 0) and 1 or 2
end)
