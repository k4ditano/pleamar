// Heat haze: what is behind the surface, bent by a slow shimmer. It keeps its
// alpha below 1, so the next capture can still see what is behind it.
fn shade(s: Shader) -> vec4<f32> {
    let wobble = vec2<f32>(sin(s.pos.y * 0.09 + s.time * 3.0), cos(s.pos.x * 0.07 + s.time * 2.3)) * 4.0 * s.a.x;
    let seen = behind(s, s.pos + wobble);
    // Where nothing is known yet (the first frames), a faint tint.
    let tone = mix(s.color.rgb, seen.rgb, seen.a);
    return vec4<f32>(tone, 0.88);
}
