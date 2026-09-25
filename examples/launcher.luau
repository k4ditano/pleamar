-- The launcher's logic: it knows which applications there are and filters. What is typed
-- already shows —that is the renderer's job—; here it only decides which list goes with it.

local apps = {}

local function filter()
    local q = (text.query or ""):lower()
    local hits = {}
    for _, a in ipairs(apps) do
        if #hits < 7 and (q == "" or a.name:lower():find(q, 1, true)) then hits[#hits + 1] = a end
    end
    model.hits = hits
    fact.sel = 0
end

--  A platform service: on Linux, the .desktop files. Where there is none, the
--  list comes out empty and the launcher says so.
if not sys.watch("apps", function(list) apps = list; filter() end) then
    model.hits = { { name = "This system does not know yet which applications it has" } }
end

on("text:query", filter)

on("launch", function(i)
    local a = model.hits[i + 1]
    if a then
        log("launching:", a.name, "→", a.exec)
        sys.call("apps.launch", a.exec)
    end
    fact.open = false
end)

--  On closing, the field is empty again for next time.
on("fact:open", function(open) if not open then text.query = ""; filter() end end)
