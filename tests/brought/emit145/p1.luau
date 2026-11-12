after(1500, function() emit("Core.height", { v = 42 }); emit("Core.close") end)
