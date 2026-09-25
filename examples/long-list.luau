-- Five thousand rows that are never all in the scene: only the sixteen in view.
-- All this logic does is cut the slice that belongs when the wheel moves.

local STEP, WINDOW, COUNT = 34, 16, 5000

local all = {}
for i = 1, COUNT do
    all[i] = { label = "row number " .. i, tag = (i % 7 == 0) and "seventh" or "" }
end
fact.total = COUNT

local shown = -1

local function show(from)
    from = math.max(0, math.min(from, COUNT - WINDOW))
    if from == shown then return end
    shown = from
    local slice = {}
    for k = 1, WINDOW do
        slice[k] = all[from + k]
    end
    fact.first = from
    model.rows = slice
end

show(0)
on("slid", function(scroll) show(math.floor(scroll / STEP)) end)
