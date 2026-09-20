-- La lógica del lanzador: sabe qué aplicaciones hay y filtra. Lo que se escribe
-- ya se ve —eso es del render—; aquí solo se decide qué lista acompaña.

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

--  Un servicio de la plataforma: en Linux, los .desktop. Donde no lo haya, la
--  lista sale vacía y el lanzador lo dice.
if not sys.watch("apps", function(list) apps = list; filter() end) then
    model.hits = { { name = "Este sistema no sabe todavía qué aplicaciones tiene" } }
end

on("text:query", filter)

on("launch", function(i)
    local a = model.hits[i + 1]
    if a then
        log("lanzo:", a.name, "→", a.exec)
        sys.call("apps.launch", a.exec)
    end
    fact.open = false
end)

--  Al cerrarse, el campo vuelve a estar vacío para la próxima vez.
on("fact:open", function(open) if not open then text.query = ""; filter() end end)
