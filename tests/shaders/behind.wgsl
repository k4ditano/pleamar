fn shade(s: Shader) -> vec4<f32> {
    let seen = behind(s, s.pos + vec2<f32>(sin(s.pos.y * 0.1) * 3.0, 0.0));
    let frosted = behind_frosted(s, s.pos);
    return vec4<f32>(mix(frosted.rgb, seen.rgb, 0.5), 0.88);
}
