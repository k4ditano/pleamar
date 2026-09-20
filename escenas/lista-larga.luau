-- Five thousand rows that are never all in the scene: only the sixteen in view.
-- All this logic does is cut the slice that belongs when the wheel moves.

local PASO, VENTANA, CUANTAS = 34, 16, 5000

local todas = {}
for i = 1, CUANTAS do
    todas[i] = { label = "row number " .. i, tag = (i % 7 == 0) and "seventh" or "" }
end
fact.total = CUANTAS

local puesta = -1

local function mostrar(desde)
    desde = math.max(0, math.min(desde, CUANTAS - VENTANA))
    if desde == puesta then return end
    puesta = desde
    local trozo = {}
    for k = 1, VENTANA do
        trozo[k] = todas[desde + k]
    end
    fact.first = desde
    model.rows = trozo
end

mostrar(0)
on("slid", function(scroll) mostrar(math.floor(scroll / PASO)) end)
