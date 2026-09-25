@group(0) @binding(0) var<uniform> sneaky: vec4<f32>;
fn shade(s: Shader) -> vec4<f32> {
    return sneaky;
}
