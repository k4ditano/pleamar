// Rings of water that run away from the pointer while it is over the box.
fn shade(s: Shader) -> vec4<f32> {
    let d = distance(s.pos, s.pointer);
    let wave = sin(d * 0.12 - s.time * 6.0) * exp(-d * 0.012);
    let here = s.hovered * max(wave, 0.0);
    let base = s.color.rgb;
    return vec4<f32>(mix(base, vec3<f32>(1.0), here * 0.6), 0.25 + here * 0.5);
}
