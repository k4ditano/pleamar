-- The notice tray. If this program can be the one that receives the
-- desktop notifications, it shows the real ones; if that place is already
-- taken by another (another bar), it runs a demo with made-up notices.
-- The list is handed over as it comes: `model.notes = list`.

local real = sys.watch("notifications", function(list) model.notes = list end)

if real then
    -- Pressing a notice is its default action, if it has one; and either way, it goes.
    on("opened", function(i)
        local n = model.notes[i + 1]
        local default = false
        for _, a in ipairs(n.actions) do default = default or a.key == "default" end
        if default then sys.call("notifications.invoke", n.id, "default") else sys.call("notifications.dismiss", n.id) end
    end)
else
    log("another program already receives the notifications: here goes the demo")
    local inbox = {
        { app = "Calendar", title = "Meeting in 5 min", body = "Design review" },
        { app = "Messages",   title = "Lucía", body = "coming down for coffee? ☕" },
    }
    local pending = {
        { app = "Updates",         title = "There are 214 packages waiting for someone to decide to install them", body = "", urgency = "low" },
        { app = "Music",           title = "Tycho", body = "Awake" },
        { app = "Battery",         title = "12 % left", body = "", urgency = "critical" },
        { app = "Messages",        title = "Lucía", body = "come on, it is getting cold" },
    }
    every(1600, function()
        local n = table.remove(pending, 1)
        if n then table.insert(inbox, 1, n); model.notes = inbox end
    end)
    on("opened", function(i)
        local n = table.remove(inbox, i + 1)
        if n then log("dismissed:", n.title); model.notes = inbox end
    end)
    model.notes = inbox
end
