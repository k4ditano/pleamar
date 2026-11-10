-- Two strokes of different lengths and an empty list: the third copy is not there.
model.strokes = {
    { pts = { { x = 20, y = 20 }, { x = 80, y = 60 }, { x = 140, y = 20 } } },
    { pts = { { x = 160, y = 90 }, { x = 200, y = 30 }, { x = 240, y = 90 }, { x = 280, y = 30 }, { x = 300, y = 90 } } },
}
for k = 0, 5 do
    fact["live.x." .. k] = 40 + k * 40
    fact["live.y." .. k] = (k % 2) * 50
end
