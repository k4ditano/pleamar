-- Marea's logic. It knows nothing of springs, shapes or coordinates: it says what
-- the notice reads and that there is one, and finds out that someone wanted to see it.

local notices = {
    { app = "CALENDAR",        title = "Meeting in 5 min",             detail = "Design review · 18:00" },
    { app = "MESSAGES",        title = "Lucía: coming down for coffee? ☕", detail = "A moment ago" },
    { app = "UPDATES",         title = "There are 214 packages waiting for someone to decide to install them", detail = "yay · 1.2 GB" },
}
local turn = 0

local function arrive()
    turn = turn % #notices + 1
    local n = notices[turn]
    text["notice.app"] = n.app
    text["notice.title"] = n.title
    text["notice.detail"] = n.detail
    text["notice.when"] = "Now"
    fact.asleep = false
    fact.open = true
    -- Building a notice's content costs. The renderer does not care.
    busy(600)
end

-- With no mouse, `--demo` plays a notice arriving and leaving.
on("demo", function()
    if fact.open then fact.open = false else arrive() end
end)

on("view_event", function()
    log("someone wants to see:", text["notice.title"])
    -- A system command: it runs on another thread and answers when it finishes.
    run("date", { "+%H:%M:%S" }, function(out) log("…and it was", out) end)
end)

on("fact:asleep", function(asleep) if asleep then log("she fell asleep") end end)
