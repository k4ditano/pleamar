// What is behind the glass, from a capture of the screen.
//
// The capture was taken with our surface on top, so it is the blend the
// compositor makes: capture = ours + (1 − our alpha) · background. We know
// ours, because it is the last canvas that was presented: the background gets
// unmixed. Where ours is almost opaque —text, an icon— it cannot be, and the
// background that was there stays; the same if the result is not a possible
// colour, which is what happens when the capture is of a different frame than
// the canvas (something was moving).

struct VertexOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) v: u32) -> VertexOut {
    // A triangle that covers the whole screen.
    let p = vec2<f32>(f32((v << 1u) & 2u), f32(v & 2u));
    var s: VertexOut;
    s.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    return s;
}

// ── unmix ─────────────────────────────────────────────────────────
@group(0) @binding(0) var capture: texture_2d<f32>;
@group(0) @binding(1) var canvas: texture_2d<f32>;
@group(0) @binding(2) var previous: texture_2d<f32>;
@group(0) @binding(3) var linear_sampler: sampler;
// Where the capture falls on the canvas, in pixels: x, y, width, height; and
// with what opacity the compositor blended ours.
struct Bounds {
    rect: vec4<f32>,
    opacity: f32,
};
@group(0) @binding(4) var<uniform> bounds: Bounds;

@fragment
fn unmix(e: VertexOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(e.pos.xy);
    // The compositor takes the capture at its own scale: it is read proportionally.
    let f = textureSampleLevel(capture, linear_sampler, (e.pos.xy - bounds.rect.xy) / bounds.rect.zw, 0.0).rgb;
    // Ours, as the compositor blended it: times its opacity.
    let s = textureLoad(canvas, px, 0) * bounds.opacity;
    let a = textureLoad(previous, px, 0);
    let remaining = 1.0 - s.a;
    // Under our opaque parts —a text, an icon— nothing can be seen: it is
    // marked as unknown (alpha 0), and the frosting fills it with what is
    // around. Keeping the last one seen left it stale forever.
    if (remaining < 0.03) {
        return vec4<f32>(0.0);
    }
    let background = (f - s.rgb) / remaining;
    // A background that cannot be: the capture is not of this canvas. With a
    // margin for rounding, which grows when dividing by what remains: half a
    // level over what remains.
    let margin = 0.03 + 0.5 / 255.0 / remaining;
    if (any(background < vec3<f32>(-margin)) || any(background > vec3<f32>(1.0 + margin))) {
        return a;
    }
    return vec4<f32>(clamp(background, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}

// ── blur ──────────────────────────────────────────────────────────
// Separable gaussian: one pass horizontally and another vertically, at half
// resolution (the frosting needs no more, and it costs a quarter). It is read
// proportionally, so the first pass, from full resolution to half, also
// downscales. What is unknown has alpha 0 and weighs nothing: the result is in
// premultiplied alpha, and whoever reads it divides by the alpha.
struct Blur {
    direction: vec2<f32>, // (1, 0) or (0, 1), in pixels of the destination
    sigma: f32,           // in pixels of the destination
    _r: f32,
    dest_size: vec2<f32>, // the size of the destination, in pixels
    _s: vec2<f32>,
};
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<uniform> b: Blur;
@group(0) @binding(2) var soft_sampler: sampler;

@fragment
fn blur(e: VertexOut) -> @location(0) vec4<f32> {
    let radius = i32(ceil(b.sigma * 2.5));
    var total = vec4<f32>(0.0);
    var weight = 0.0;
    for (var k = -radius; k <= radius; k++) {
        let w = exp(-f32(k * k) / (2.0 * b.sigma * b.sigma));
        let q = (e.pos.xy + b.direction * f32(k)) / b.dest_size;
        total += textureSampleLevel(source, soft_sampler, q, 0.0) * w;
        weight += w;
    }
    return total / weight;
}
