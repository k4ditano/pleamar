-- The tray's logic: which icons there are, and the menu of the one asked for. A menu is
-- a tree, and a submenu does not open beside it: you go into it, with a row to come back.

local owner = nil                -- which icon the open menu belongs to
local root, path = {}, {}        -- that menu, whole; and which submenus have been entered

local function show(list)
    local rows = #path > 0 and { { back = true, label = "‹  Back" } } or {}
    for _, it in ipairs(list) do rows[#rows + 1] = it end
    model.rows = rows
end

if not sys.watch("tray", function(list) model.icons = list end) then
    log("this system has no tray yet")
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
        -- No menu to read: let it show the menu itself, if it knows how.
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
