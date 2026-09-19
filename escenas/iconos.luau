-- La lógica de la bandeja: qué iconos hay, y el menú del que se pida. El menú es
-- un árbol; la escena tiene catorce filas, así que un submenú no sale al lado:
-- se entra en él, con una fila arriba para volver.

local items = {}                 -- los iconos, como los cuenta el servicio
local owner = nil                -- de qué icono es el menú abierto
local root = {}                  -- ese menú, entero
local path = {}                  -- por qué submenús se ha entrado
local shown = {}                 -- las filas que se ven ahora

local function show(list)
    shown = {}
    if #path > 0 then shown[1] = { back = true, label = "‹  Volver", enabled = true, children = {} } end
    for _, it in ipairs(list) do
        if #shown < 14 then shown[#shown + 1] = it end
    end
    for i = 1, 14 do
        local r, k = shown[i], i - 1
        text["row." .. k] = r and r.label or ""
        fact["sep." .. k] = r and r.separator and 1 or 0
        fact["off." .. k] = r and not r.enabled and 1 or 0
        fact["sub." .. k] = r and #r.children > 0 and 1 or 0
        fact["chk." .. k] = r and r.checked ~= nil and (r.checked and 2 or 1) or 0
    end
    fact.rows = #shown
end

if not sys.watch("tray", function(list)
    items = list
    fact.count = math.min(#list, 8)
    for i = 1, 8 do text["pic." .. (i - 1)] = list[i] and list[i].icon or "" end
end) then
    log("este sistema no tiene bandeja todavía")
end

on("activate", function(i)
    local it = items[i + 1]
    if it then sys.call("tray.activate", it.key) end
end)

on("menu", function(i)
    local it = items[i + 1]
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
    local r = shown[i + 1]
    if not r or r.separator or not r.enabled then return end
    if r.back then
        table.remove(path)
        show(#path > 0 and path[#path].children or root)
    elseif #r.children > 0 then
        path[#path + 1] = r
        show(r.children)
    else
        sys.call("tray.menu_click", owner.key, r.id)
        fact.menu_open = false
    end
end)
