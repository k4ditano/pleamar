-- La lógica de la bandeja: qué iconos hay, y el menú del que se pida. El menú es
-- un árbol, y un submenú no sale al lado: se entra en él, con una fila para volver.

local owner = nil                -- de qué icono es el menú abierto
local root, path = {}, {}        -- ese menú, entero; y por qué submenús se ha entrado

local function show(list)
    local rows = #path > 0 and { { back = true, label = "‹  Volver" } } or {}
    for _, it in ipairs(list) do rows[#rows + 1] = it end
    model.rows = rows
end

if not sys.watch("tray", function(list) model.icons = list end) then
    log("este sistema no tiene bandeja todavía")
end

on("activate", function(i)
    local it = model.icons[i + 1]
    if it then sys.call("tray.activate", it.key) end
end)

on("menu", function(i)
    local it = model.icons[i + 1]
    if not it then return end
    local ok, menu = pcall(sys.ask, "tray.menu", it.key)
    if not ok then
        -- Sin menú que leer: que lo enseñe ella, si sabe.
        pcall(sys.call, "tray.context", it.key)
        return
    end
    owner, root, path = it, menu, {}
    show(menu)
    fact.menu_open = true
end)

on("choose", function(i)
    local r = model.rows[i + 1]
    if r.back then
        table.remove(path)
        show(#path > 0 and path[#path].children or root)
    elseif r.children and #r.children > 0 then
        path[#path + 1] = r
        show(r.children)
    else
        sys.call("tray.menu_click", owner.key, r.id)
        fact.menu_open = false
    end
end)
