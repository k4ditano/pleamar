-- Star catcher's logic: where everything is, every frame.
local LEVEL = 410
local x, v = 480, 0
local stars = {}
local wait, score, lives, over, speed = 0.8, 0, 5, false, 140
local frames, second = 0, 0

local function again()
    stars, wait, score, lives, over, speed = {}, 0.8, 0, 5, false, 140
    fact.score, fact.lives, fact.over = 0, 5, false
    model.stars = stars
    sound.tone({ wave = "triangle", freq = 330, to = 660, ms = 220, volume = 0.3 })
end

on("frame", function(dt)
    frames += 1
    second += dt
    if second >= 1 then
        fact.fps = frames
        frames, second = 0, second - 1
    end
    if over then return end

    -- Where the hand says: the keys, or the stick.
    local dir = ((held("Right") or held("d")) and 1 or 0) - ((held("Left") or held("a")) and 1 or 0)
    local p = pad(1)
    if p then
        if math.abs(p.lx) > 0.18 then dir += p.lx end
        dir += (p.right and 1 or 0) - (p.left and 1 or 0)
    end
    dir = math.clamp(dir, -1, 1)
    v += (dir * 560 - v) * math.min(1, dt * 9)
    x = math.clamp(x + v * dt, 50, 910)
    fact.bx, fact.lean = x, v * 0.03

    -- One more star, sooner the further the game goes.
    wait -= dt
    if wait <= 0 and #stars < 28 then
        table.insert(stars, { x = 60 + math.random() * 840, y = -20, r = 12, fall = speed * (0.8 + math.random() * 0.5), spin = math.random() * 360, turn = (math.random() - 0.5) * 300, rare = math.random() < 0.15 })
        wait = math.max(0.26, 1.1 - score * 0.014)
    end

    for i = #stars, 1, -1 do
        local s = stars[i]
        s.y += s.fall * dt
        s.spin += s.turn * dt
        local side = (s.x - 480) / 480
        if hit.circle_box(s.x, s.y, s.r, x, LEVEL - 24, 86, 44) then
            score += s.rare and 5 or 1
            speed += 3
            fact.score = score
            fact["at.x"], fact["at.y"] = s.x, s.y
            emit("caught")
            sound.tone({ wave = "square", freq = s.rare and 660 or 520, to = s.rare and 1320 or 880, ms = 110, volume = 0.22, pan = side })
            table.remove(stars, i)
        elseif s.y > LEVEL + 8 then
            lives -= 1
            fact.lives = lives
            fact["at.x"] = s.x
            emit("missed")
            sound.tone({ wave = "noise", freq = 700, to = 90, ms = 260, volume = 0.2, pan = side })
            table.remove(stars, i)
            if lives <= 0 then
                over = true
                fact.over = true
                sound.tone({ wave = "saw", freq = 330, to = 82, ms = 700, volume = 0.22 })
            end
        end
    end
    model.stars = stars
end)

on("key_down", function(name)
    if over and (name == "space" or name == "Return") then again() end
end)
sys.watch("gamepad", function(pads)
    local p = pads[1]
    if over and p and (p.a or p.start) then again() end
end)
