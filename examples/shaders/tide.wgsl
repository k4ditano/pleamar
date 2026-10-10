// The sea at night, seen from the side: sky, moon and stars above a surface
// that the scene moves, and lit water under it.
//   s.a = seconds into its 16 (it comes back to where it began), y of the
//         still surface, how far the tide is in (0 to 1), how far the shooting
//         star has gone (0 to 1; at either end there is none)
//   s.b = 1 to paint only the water in front, what laps over the boat · how
//         far the dawn has come (0 night, 1 day) · where the scene's corner is
//         inside this box (x, y): the box can be bigger than the scene, and
//         the sea goes on
//   s.color, s.color2: the two tones of the aurora

// A number from 0 to 1 for each whole point, worked out in whole numbers: the
// usual `fract(sin(…) * 43758.5)` gives the same corner two values from two
// neighbouring cells on some cards, and the noise comes out in tiles.
fn hash(p: vec2<f32>) -> f32 {
    let q = vec2<u32>(vec2<i32>(floor(p)));
    var h = q.x * 1597334673u ^ q.y * 3812015801u;
    h = (h ^ (h >> 15u)) * 2246822519u;
    h = (h ^ (h >> 13u)) * 3266489917u;
    h = h ^ (h >> 16u);
    return f32(h >> 8u) / 16777216.0;
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash(i);
    let b = hash(i + vec2<f32>(1.0, 0.0));
    let c = hash(i + vec2<f32>(0.0, 1.0));
    let d = hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var amplitude = 0.5;
    var q = p;
    for (var k = 0; k < 4; k++) {
        v += amplitude * noise(q);
        q = q * 2.03 + vec2<f32>(1.7, 9.2);
        amplitude *= 0.5;
    }
    return v;
}

// Noise that drifts, and is back where it began when the loop is.
fn drift(p: vec2<f32>, v: vec2<f32>, c: f32, period: f32) -> f32 {
    return mix(fbm(p + v * c), fbm(p + v * (c - period)), c / period);
}

// The two swells: the same ones the scene gives its boat and its clip.
fn swell(x: f32, c: f32) -> f32 {
    return 9.0 * sin(x * 0.0131 + c * 0.785398) + 5.0 * sin(x * 0.0293 - c * 1.570796 + 1.3);
}

fn shade(s: Shader) -> vec4<f32> {
    let c = s.a.x;
    let level = s.a.y;
    let tide = s.a.z;
    let dawn = clamp(s.b.y, 0.0, 1.0);
    let period = 16.0;
    let pos = s.pos - s.b.zw;
    let uv = pos / vec2<f32>(960.0, 540.0);
    let x = pos.x;
    let y = pos.y;
    let turn = 6.2831853 / period;

    if (s.b.x > 0.5) {
        // A swell a little ahead of the main one, and thin: what is under it
        // still shows.
        let d = y - (level + 6.0 + swell(x + 44.0, c + 0.4));
        let water = smoothstep(-1.0, 1.0, d) * (1.0 - smoothstep(4.0, 40.0, d)) * 0.78;
        let foam = exp(-d * d / 2.4) * 0.85;
        let tone = mix(mix(vec3<f32>(0.05, 0.36, 0.40), vec3<f32>(0.10, 0.50, 0.56), dawn), mix(vec3<f32>(0.86, 1.0, 0.95), vec3<f32>(1.0, 0.93, 0.86), dawn), foam);
        return vec4<f32>(tone, clamp(water + foam, 0.0, 1.0));
    }

    // ── the sky ──
    let high = pow(clamp(uv.y, 0.0, 1.0), 1.3);
    let night = mix(vec3<f32>(0.014, 0.020, 0.048), vec3<f32>(0.035, 0.105, 0.17), high);
    // At dawn: violet above, rose and then gold down towards the water.
    let low = clamp(uv.y / 0.34, 0.0, 1.0);
    let day = mix(mix(vec3<f32>(0.20, 0.17, 0.42), vec3<f32>(0.86, 0.42, 0.50), smoothstep(0.0, 0.75, low)), vec3<f32>(1.0, 0.74, 0.46), smoothstep(0.55, 1.0, low));
    var col = mix(night, day, dawn);
    let fold = drift(vec2<f32>(uv.x * 2.2, 0.0), vec2<f32>(0.11, 0.07), c, period);
    let band = exp(-pow((uv.y - 0.05 - fold * 0.26) * 6.0, 2.0));
    let rays = 0.45 + 0.55 * drift(vec2<f32>(uv.x * 15.0, uv.y * 0.7), vec2<f32>(-0.32, 0.0), c, period);
    let tone = mix(s.color.rgb, s.color2.rgb, smoothstep(0.15, 0.85, uv.x + fold * 0.3));
    col += tone * band * rays * 0.6 * (1.0 - 0.8 * dawn);

    let cell = floor(pos / 11.0);
    let seed = hash(cell);
    let place = fract(pos / 11.0) - 0.5 - (vec2<f32>(hash(cell + 3.1), hash(cell + 7.7)) - 0.5) * 0.7;
    let twinkle = 0.55 + 0.45 * sin(c * turn * (3.0 + floor(seed * 173.0) % 7.0) + seed * 90.0);
    let size = 0.07 + 0.09 * hash(cell + 11.3);
    col += vec3<f32>(0.9, 0.97, 1.0) * step(0.955, seed) * smoothstep(size, 0.0, length(place)) * twinkle * (1.0 - dawn);

    // A shooting star, while the scene says one is on its way.
    let k = clamp(s.a.w, 0.0, 1.0);
    let head = mix(vec2<f32>(455.0, 48.0), vec2<f32>(170.0, 205.0), k * k * (3.0 - 2.0 * k));
    let back = normalize(vec2<f32>(285.0, -157.0));
    let along = clamp(dot(pos - head, back), 0.0, 120.0);
    let aside = distance(pos, head + back * along);
    col += vec3<f32>(0.9, 1.0, 0.95) * exp(-aside * aside / 1.6) * (1.0 - along / 120.0) * sin(k * 3.14159) * 1.4;

    // The moon; and at dawn the same disc is the sun, bigger in its glow.
    let moon = vec2<f32>(772.0, 80.0);
    let away = distance(pos, moon);
    let craters = mix(0.92 + 0.08 * fbm(pos * 0.11), 1.0, dawn);
    col += mix(vec3<f32>(0.55, 0.9, 0.82) * exp(-away / 60.0) * 0.32, vec3<f32>(1.0, 0.82, 0.55) * exp(-away / 150.0) * 0.6, dawn);
    col = mix(col, mix(vec3<f32>(0.96, 0.99, 0.93), vec3<f32>(1.0, 0.97, 0.86), dawn) * craters, smoothstep(27.0, 25.6, away));

    // ── two swells further off, as silhouettes ──
    let far = level - 17.0 + 0.65 * swell(x * 0.62 + 520.0, c + 4.0);
    let mid = level - 8.0 + 0.8 * swell(x * 0.8 + 210.0, c + 2.0);
    col = mix(col, mix(vec3<f32>(0.035, 0.20, 0.26), vec3<f32>(0.20, 0.33, 0.46), dawn), smoothstep(-0.8, 0.8, y - far) * 0.72);
    col = mix(col, mix(vec3<f32>(0.04, 0.28, 0.33), vec3<f32>(0.14, 0.40, 0.50), dawn), smoothstep(-0.8, 0.8, y - mid) * 0.8);

    // ── the water ──
    let depth = y - (level + swell(x, c));
    var water = mix(mix(vec3<f32>(0.04, 0.40, 0.43), vec3<f32>(0.08, 0.56, 0.62), dawn), mix(vec3<f32>(0.010, 0.045, 0.095), vec3<f32>(0.02, 0.12, 0.24), dawn), pow(smoothstep(0.0, 430.0, depth), 0.62));
    // Shafts of light slanting down from the surface…
    let shaft = drift(vec2<f32>((x + depth * 0.42) * 0.017, 3.0), vec2<f32>(0.06, 0.0), c, period);
    water += vec3<f32>(0.22, 0.78, 0.62) * pow(smoothstep(0.38, 0.78, shaft), 2.0) * exp(-max(depth, 0.0) / 250.0) * (0.25 + 0.2 * tide);
    // …the net of light close under it…
    let net = drift(vec2<f32>(x * 0.05, depth * 0.11), vec2<f32>(0.28, 0.13), c, period);
    water += vec3<f32>(0.62, 1.0, 0.86) * pow(1.0 - abs(net * 2.0 - 1.0), 7.0) * exp(-max(depth, 0.0) / 80.0) * 0.5;
    // …the moon broken on it…
    let glitter = drift(vec2<f32>(x * 0.09, depth * 0.55), vec2<f32>(0.0, 0.7), c, period);
    let column = smoothstep(1.0, 0.0, abs(x - moon.x) / (16.0 + max(depth, 0.0) * 0.55));
    water += vec3<f32>(0.92, 1.0, 0.92) * column * smoothstep(0.52, 0.7, glitter) * exp(-max(depth, 0.0) / 55.0) * 0.7;
    // …and the surface itself: a bright line, and the light it lets in.
    water += vec3<f32>(0.82, 1.0, 0.94) * exp(-depth * depth / 3.0) * 0.9;
    water += vec3<f32>(0.16, 0.78, 0.66) * exp(-max(depth, 0.0) / 16.0) * 0.36;
    col = mix(col, water, smoothstep(-0.8, 0.8, depth));

    // A little darker towards the corners.
    let off = (uv - 0.5) * vec2<f32>(1.0, 0.8);
    col *= 1.0 - 0.55 * min(dot(off, off), 0.7);
    return vec4<f32>(col, 1.0);
}
