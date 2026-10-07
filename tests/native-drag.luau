local path = sys.ask("env", "PLEAMAR_DRAG_FILE")
if path then text.file = path end
every(100, function() fact.ticks += 1 end)
on("drop:target", function(data, mime)
    text.received = data
    text.kind = mime
    fact.transfers += 1
end)
