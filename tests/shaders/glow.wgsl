// A glow that breathes, with a helper of its own and a constant.
const SOFT: f32 = 0.35;

fn breathe(t: f32) -> f32 {
    return 0.5 + 0.5 * sin(t * 2.0);
}

fn shade(s: Shader) -> vec4<f32> {
    let d = distance(s.uv, vec2<f32>(0.5));
    let a = smoothstep(0.5, SOFT, d) * breathe(s.time) * s.a.x;
    return vec4<f32>(mix(s.color.rgb, s.color2.rgb, s.hovered), a);
}
