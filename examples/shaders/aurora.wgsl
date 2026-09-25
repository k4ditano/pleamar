// An aurora: curtains of light that drift and fold, drawn by noise.
// `s.a.x` is how bright it is (0 to 1), `s.color` and `s.color2` its two tones.

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
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

// Several octaves of noise: large folds with small ripples on them.
fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var amplitude = 0.5;
    var q = p;
    for (var k = 0; k < 5; k++) {
        v += amplitude * noise(q);
        q = q * 2.03 + vec2<f32>(1.7, 9.2);
        amplitude *= 0.5;
    }
    return v;
}

fn shade(s: Shader) -> vec4<f32> {
    let t = s.time * 0.12;
    let p = vec2<f32>(s.uv.x * 3.0, s.uv.y * 1.2);
    // The curtain: a band whose height wanders with the noise.
    let fold = fbm(vec2<f32>(p.x + t, t * 0.7)) * 0.9;
    let band = exp(-pow((s.uv.y - 0.35 - fold * 0.35) * 5.0, 2.0));
    // Vertical rays inside the band, as the real ones have.
    let rays = 0.55 + 0.45 * fbm(vec2<f32>(p.x * 6.0 - t * 2.0, s.uv.y * 0.5));
    let glow = band * rays * s.a.x;
    let tone = mix(s.color.rgb, s.color2.rgb, smoothstep(0.2, 0.8, s.uv.x + fold * 0.3));
    return vec4<f32>(tone, clamp(glow, 0.0, 1.0));
}
