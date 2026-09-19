-- La lógica del lanzador: sabe qué aplicaciones hay y filtra. Lo que se escribe
-- ya se ve —eso es del render—; aquí solo se decide qué lista acompaña.

local apps, shown = {}, {}

local function filter()
    local q = (text.query or ""):lower()
    shown = {}
    for _, a in ipairs(apps) do
        if q == "" or a.name:lower():find(q, 1, true) then
            shown[#shown + 1] = a
            if #shown == 7 then break end
        end
    end
    for i = 1, 7 do
        text["hit." .. (i - 1)] = shown[i] and shown[i].name or ""
    end
    fact.count = #shown
    fact.sel = 0
end

--  Un servicio de la plataforma: en Linux, los .desktop. Donde no lo haya, la
--  lista sale vacía y el lanzador lo dice.
if not sys.watch("apps", function(list) apps = list; filter() end) then
    text["hit.0"] = "Este sistema no sabe todavía qué aplicaciones tiene"
    fact.count = 1
end

on("text:query", filter)

on("launch", function(i)
    local a = shown[i + 1]
    if a then
        log("lanzo:", a.name, "→", a.exec)
        sys.call("apps.launch", a.exec)
    end
    fact.open = false
end)

--  Al cerrarse, el campo vuelve a estar vacío para la próxima vez.
on("fact:open", function(v) if v == 0 then text.query = ""; filter() end end)
