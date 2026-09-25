// The glass looks at where the edge points with `dpdx`/`dpdy` of the
// distance. All the pixels of a quad belong to the same element, so the
// branches by kind do not split a 2×2 block in two: the derivative holds.
diagnostic(off, derivative_uniformity);

// One quad per element. Each element brings its bounding box, and a pixel only
// runs the shapes of the element that covers it: the scene can grow without
// every pixel paying for all of it.
//
// An element is a body —one or several shapes blended by a smooth minimum,
// with its paint, border, light and shadow— or a piece of the atlas. Everything
// is signed distances and premultiplied alpha.

struct U {
    header: vec4<f32>,   // logical width and height, time, scale (real pixels per logical pixel)
    hud: vec4<f32>,      // origin x, period in ms, logic stalled, origin y: where this surface looks at the scene from
    times: array<vec4<f32>, 30>,
    backdrop: vec4<f32>, // x: there is an unmixed background to read (the lens) · yz: where it was pressed · w: how much light it lets through
};

struct Shape {
    a: vec4<f32>,      // kind, blend, stroke (0 = filled), -
    b: vec4<f32>,      // centre x, y · half width, half height (segment: vector to the other end)
    c: vec4<f32>,      // radius, rotation, scale x, scale y (arc: radius, rotation, half aperture, -; path: first point, rotation, how many, closed)
    t0: vec4<f32>,     // what is inherited, already inverted: from screen to local (2×2 matrix)…
    t1: vec4<f32>,     // …its translation, and how much it stretches distances
};

struct Element {
    header: vec4<f32>,    // kind, first shape (or layer no.), no. of shapes (or "tinted"), alpha
    bounds: vec4<f32>,    // x0, y0, x1, y1
    color0: vec4<f32>,    // r, g, b, edge
    color1: vec4<f32>,    // -, -, -, gradient (0 no, 1 linear, 2 radial)
    line: vec4<f32>,      // gradient: from (x, y) to (x, y); radial: centre (x, y), radius, -
    light: vec4<f32>,     // amount, from y, height, border thickness
    border: vec4<f32>,    // r, g, b, -
    shadow: vec4<f32>,    // dx, dy, blur, alpha
    clips: vec4<f32>,     // up to four shapes it is clipped to (-1 = none)
    dest: vec4<f32>,      // texture: x, y, width, height · gradient: first stop, how many, -, -
    uv: vec4<f32>,
    t0: vec4<f32>,        // from screen to local, as in the shapes
    t1: vec4<f32>,
};

const ELLIPSE: u32 = 0u;
const RECT: u32 = 1u;
const ARC: u32 = 2u;
const SEGMENT: u32 = 3u;
const PATH: u32 = 4u;

const BODY: u32 = 0u;
const TEXTURE: u32 = 1u;
const LAYER: u32 = 2u;
// One of the scene's own shaders: `user_shader`, added after this file (see shaders.rs).
const SHADER: u32 = 3u;
// A particle emitter: see `vs_particle`.
const PARTICLES: u32 = 4u;
const HUD: u32 = 9u;

const FAR: f32 = 1e6;

// 0 · the scene, the same for all surfaces.
@group(0) @binding(0) var<storage, read> shapes: array<Shape>;
@group(0) @binding(1) var<storage, read> elements: array<Element>;
@group(0) @binding(2) var atlas: texture_2d<f32>;
@group(0) @binding(3) var atlas_sampler: sampler;
// The points of the paths, in x, y pairs, relative to the centre of each one.
@group(0) @binding(4) var<storage, read> points: array<f32>;
// The gradient stops: r, g, b and where each one falls, in order.
@group(0) @binding(5) var<storage, read> stops: array<vec4<f32>>;
// 2 · what belongs to each surface: its size and its scale.
@group(2) @binding(0) var<uniform> u: U;
// Groups with opacity are painted separately, here, and blended at once.
@group(1) @binding(0) var layers: texture_2d_array<f32>;
// 3 · what is behind the glass, already unmixed: sharp and frosted.
@group(3) @binding(0) var backdrop_sharp: texture_2d<f32>;
@group(3) @binding(1) var backdrop_blurred: texture_2d<f32>;
@group(3) @binding(2) var backdrop_sampler: sampler;

// The lens: how much of the glass is bent background (the rest, what is behind
// as is, which the compositor puts underneath and lets the next capture unmix)
// and how much tint.
const LENS_ALPHA: f32 = 0.88;
const LENS_TINT: f32 = 0.22;
// The thickness of the glass, in bevel widths: how much it bends.
const LENS_THICKNESS: f32 = 1.4;

// How much what is behind shifts at a distance `inside` from the edge, with a
// bevel of width `bevel`. The edge has the profile of a "squircle",
// ⁴√(1 − (1 − x)⁴): it rises almost vertically and flattens inwards, and in the
// centre it bends nothing. A ray coming straight down enters through that
// slope and bends according to Snell (index 1.5); what shifts is the thickness
// it has left times the tangent of how much it has bent.
fn displacement(inside: f32, bevel: f32) -> f32 {
    let x = clamp(inside / bevel, 0.0, 1.0);
    let v = 1.0 - x;
    let v4 = v * v * v * v;
    let height = pow(max(1.0 - v4, 0.0), 0.25);
    let slope = v * v * v * pow(max(1.0 - v4, 1e-4), -0.75);
    let incoming = atan(slope);
    let outgoing = asin(sin(incoming) / 1.5);
    return height * bevel * LENS_THICKNESS * tan(incoming - outgoing);
}

struct VertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) element: u32,
};

@vertex
fn vs(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> VertexOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = elements[i].bounds;
    // A particle emitter is not drawn here but by `vs_particle`: nothing.
    var p = mix(c.xy, c.zw, corners[v]);
    if (u32(elements[i].header.x) == PARTICLES) { p = c.xy; }
    var s: VertexOut;
    let q = p - u.hud.xw;
    s.pos = vec4<f32>(q.x / u.header.x * 2.0 - 1.0, 1.0 - q.y / u.header.y * 2.0, 0.0, 1.0);
    s.element = i;
    return s;
}

fn to_local(p: vec2<f32>, t0: vec4<f32>, t1: vec4<f32>) -> vec2<f32> {
    return vec2<f32>(dot(t0.xy, p), dot(t0.zw, p)) + t1.xy;
}

fn rotate_point(p: vec2<f32>, pivot: vec2<f32>, angle: f32) -> vec2<f32> {
    if (angle == 0.0) { return p; }
    let q = p - pivot;
    let c = cos(angle);
    let s = sin(angle);
    return vec2<f32>(c * q.x + s * q.y, -s * q.x + c * q.y) + pivot;
}

fn rounded_rect(p: vec2<f32>, half: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// A path: the distance to the nearest segment, signed if it is closed (the
// winding number says whether the point is inside). It is the SDF of a polygon.
fn path_distance(p: vec2<f32>, first: u32, n: u32, closed: bool) -> f32 {
    if (n < 2u) { return FAR; }
    var best = FAR;
    var inside = 1.0;
    let segments = select(n - 1u, n, closed);
    for (var i = 0u; i < segments; i++) {
        let j = (i + 1u) % n;
        let a = vec2<f32>(points[(first + i) * 2u], points[(first + i) * 2u + 1u]);
        let b = vec2<f32>(points[(first + j) * 2u], points[(first + j) * 2u + 1u]);
        let e = b - a;
        let w = p - a;
        let h = clamp(dot(w, e) / max(dot(e, e), 1e-6), 0.0, 1.0);
        best = min(best, length(w - e * h));
        if (closed) {
            // Crossings of the horizontal through the point: even, outside; odd, inside.
            let below_a = p.y >= a.y;
            let above_b = p.y < b.y;
            let side = e.x * w.y > e.y * w.x;
            if ((below_a && above_b && side) || (!below_a && !above_b && !side)) { inside = -inside; }
        }
    }
    return best * inside;
}

// The colour of a gradient at `t`: between the two stops around it.
fn between_stops(first: u32, n: u32, t: f32) -> vec3<f32> {
    var prev = stops[first];
    if (t <= prev.w || n < 2u) { return prev.rgb; }
    for (var i = 1u; i < n; i++) {
        let cur = stops[first + i];
        if (t <= cur.w) {
            let d = max(cur.w - prev.w, 1e-4);
            return mix(prev.rgb, cur.rgb, (t - prev.w) / d);
        }
        prev = cur;
    }
    return prev.rgb;
}

fn smooth_min(a: f32, b: f32, k: f32) -> f32 {
    if (k < 0.5 || a > FAR * 0.5 || b > FAR * 0.5) { return min(a, b); }
    let h = max(k - abs(a - b), 0.0) / k;
    return min(a, b) - h * h * k * 0.25;
}

fn shape_distance(k: u32, point: vec2<f32>) -> f32 {
    let f = shapes[k];
    // First what is inherited from the group, then its own rotation about its centre.
    let p = rotate_point(to_local(point, f.t0, f.t1), f.b.xy, f.c.y) - f.b.xy;
    var d = FAR;
    switch u32(f.a.x) {
        case ELLIPSE: {
            let e = f.c.zw;
            d = (length(p / e) - f.c.x) * min(e.x, e.y);
        }
        case RECT: {
            if (f.b.z >= 0.5 && f.b.w >= 0.5) { d = rounded_rect(p, f.b.zw, f.c.x); }
        }
        case ARC: {
            // An arc like "∩", open as much as the half aperture says.
            let sc = vec2<f32>(sin(f.c.z), cos(f.c.z));
            let q = vec2<f32>(abs(p.x), -p.y);
            if (sc.y * q.x > sc.x * q.y) { d = length(q - sc * f.c.x); } else { d = abs(length(q) - f.c.x); }
        }
        case SEGMENT: {
            let h = clamp(dot(p, f.b.zw) / max(dot(f.b.zw, f.b.zw), 0.0001), 0.0, 1.0);
            d = length(p - f.b.zw * h);
        }
        case PATH: {
            d = path_distance(p, u32(f.c.x), u32(f.c.z), f.c.w > 0.5);
        }
        default: {}
    }
    // The stroke turns any shape into its outline: a circle into a ring.
    if (f.a.z > 0.0 && d < FAR * 0.5) {
        let kind = u32(f.a.x);
        // What is already a line only gets thicker; what encloses something stays as its outline.
        let is_line = kind == ARC || kind == SEGMENT || (kind == PATH && f.c.w <= 0.5);
        if (is_line) { d = d - f.a.z * 0.5; } else { d = abs(d) - f.a.z * 0.5; }
    }
    if (d > FAR * 0.5) { return d; }
    return d * f.t1.z;
}

// The edge is smoothed over three quarters of a REAL pixel: at scale 2, half
// that width in logical pixels, or everything would come out blurry.
fn coverage(d: f32) -> f32 {
    let m = 0.75 / u.header.w;
    return 1.0 - smoothstep(-m, m, d);
}

fn over(below: vec4<f32>, color: vec3<f32>, a: f32) -> vec4<f32> {
    return vec4<f32>(color * a, a) + below * (1.0 - a);
}

fn instruments(p: vec2<f32>) -> vec4<f32> {
    var c = vec4<f32>(0.0);
    let hp = p - vec2<f32>(60.0, u.header.y - 68.0);
    let panel = rounded_rect(hp - vec2<f32>(286.0, 30.0), vec2<f32>(334.0, 38.0), 12.0);
    c = over(c, vec3<f32>(0.05, 0.055, 0.055), 0.78 * coverage(panel));
    if (hp.x >= 0.0 && hp.x < 600.0 && hp.y >= 0.0 && hp.y <= 60.0) {
        let j = u32(hp.x / 5.0);
        let v = u.times[j / 4u][j % 4u];
        let ms = abs(v);
        let height = min(ms * 2.0, 60.0);
        let in_bar = step(60.0 - height, hp.y) * step(fract(hp.x / 5.0), 0.8);
        var tone = vec3<f32>(0.62, 0.84, 0.74);
        if (ms > u.hud.y * 1.6) { tone = vec3<f32>(0.95, 0.35, 0.3); }
        if (v < 0.0) { c = over(c, vec3<f32>(0.95, 0.35, 0.3), 0.16); }
        c = over(c, tone, in_bar * step(0.01, ms));
        let line_y = 60.0 - u.hud.y * 2.0;
        c = over(c, vec3<f32>(1.0), 0.25 * step(abs(hp.y - line_y), 0.5));
    }
    let pilot = length(p - vec2<f32>(38.0, u.header.y - 38.0)) - 5.0;
    let pilot_tone = mix(vec3<f32>(0.62, 0.84, 0.74), vec3<f32>(0.95, 0.3, 0.26), u.hud.z);
    return over(c, pilot_tone, coverage(pilot));
}

@fragment
fn fs(e: VertexOut) -> @location(0) vec4<f32> {
    // The position arrives in real pixels; the scene thinks in logical ones.
    let p = e.pos.xy / u.header.w + u.hud.xw;
    let el = elements[e.element];
    let kind = u32(el.header.x);
    if (kind == HUD) { return instruments(p); }

    var clip = 1.0;
    for (var k = 0; k < 4; k++) {
        let r = el.clips[k];
        if (r >= 0.0) { clip *= coverage(shape_distance(u32(r), p)); }
    }
    let alpha = el.header.w * clip;
    if (alpha <= 0.001) { discard; }

    var c = vec4<f32>(0.0);
    if (kind == LAYER) {
        // A plain opacity group: its layer as is. With effects, `layer_with_effects`.
        if (el.border.y < 0.5) { return textureLoad(layers, vec2<i32>(e.pos.xy), i32(el.header.y), 0) * alpha; }
        return layer_with_effects(el, e.pos.xy, p, alpha);
    }
    if (kind == SHADER) {
        return user_shader(el, p) * alpha;
    }
    let local = to_local(p, el.t0, el.t1);
    if (kind == TEXTURE) {
        let q = local;
        // A letter with effects draws outside itself too: its outline and its shadow.
        if (el.header.z > 1.5 && el.light.w > 0.5) { return text_with_effects(el, q, alpha); }
        let uv01 = (q - el.dest.xy) / max(el.dest.zw, vec2<f32>(1.0));
        if (uv01.x < 0.0 || uv01.x > 1.0 || uv01.y < 0.0 || uv01.y > 1.0) { discard; }
        let t = textureSampleLevel(atlas, atlas_sampler, mix(el.uv.xy, el.uv.zw, uv01), 0.0);
        // Tinted: the piece is a mask —a letter (2), a symbolic icon (1)— and the element provides the colour.
        // A letter (2) gets its edges firmed up. Blended as they are, in sRGB,
        // a stem that falls across two pixels —most of them: the letters are
        // placed at quarters of a pixel and the font is only hinted
        // vertically— comes out as two greys, and light text on a dark
        // background reads thin and washed out. This is the contrast curve
        // DirectWrite and Skia apply to their coverage: stronger the lighter
        // the letter, because dark text on light already gains body from the
        // sRGB blend. Measured on an «I» in #f6f6f5 over #1b1b1c: #bc + #67
        // became #d4 + #8a —the same width, but a letter and not a smudge.
        if (el.header.z > 1.5) {
            let lum = dot(el.color0.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
            let k = mix(0.3, 1.0, lum);
            let a = t.a * (k + 1.0) / (t.a * k + 1.0);
            return vec4<f32>(el.color0.rgb * a, a) * alpha;
        }
        if (el.header.z > 0.5) { return vec4<f32>(el.color0.rgb * t.a, t.a) * alpha; }
        return t * alpha;
    }

    let first = u32(el.header.y);
    let n = u32(el.header.z);
    var d = FAR;
    var d_shadow = FAR;
    let has_shadow = el.shadow.w > 0.0;
    for (var k = 0u; k < n; k++) {
        let blend = shapes[first + k].a.y;
        d = smooth_min(d, shape_distance(first + k, p), blend);
        if (has_shadow) { d_shadow = smooth_min(d_shadow, shape_distance(first + k, p - el.shadow.xy), blend); }
    }
    // Glass: from 0 to 1, in the `uv` slot, which a body does not use.
    let glass = el.uv.x;
    if (has_shadow) {
        // Its colour is the three slots of `color1`; if nothing is said, black.
        // Under glass the shadow does not show through: only around it.
        let covered = 1.0 - coverage(d) * glass;
        c = over(c, el.color1.rgb, el.shadow.w * alpha * covered * (1.0 - smoothstep(-8.0, el.shadow.z, d_shadow)));
    }
    var tone = el.color0.rgb;
    if (el.color1.w > 0.5) {
        var t = 0.0;
        if (el.color1.w > 1.5) {
            // Radial: from the centre outwards, up to the radius.
            t = clamp(length(local - el.line.xy) / max(el.line.z, 0.0001), 0.0, 1.0);
        } else {
            let axis = el.line.zw - el.line.xy;
            t = clamp(dot(local - el.line.xy, axis) / max(dot(axis, axis), 0.0001), 0.0, 1.0);
        }
        tone = between_stops(u32(el.dest.x), u32(el.dest.y), t);
    }
    let light = clamp(1.0 - (local.y - el.light.y) / max(el.light.z, 1.0), 0.0, 1.0) * el.light.x;
    tone += vec3<f32>(light) + vec3<f32>(el.color0.w) * smoothstep(-2.2, -0.4, d);
    // Where the edge points: towards where the distance grows.
    let g = vec2<f32>(dpdx(d), dpdy(d));
    let normal = g / max(length(g), 1e-6);
    let inside = max(-d, 0.0);
    // The light of the glass, the same with a lens or without it. It comes from
    // the top left: the edge that faces it shines thin and strong; the opposite
    // one, weaker, is the light coming out. The closer to the edge, the
    // brighter —the glass seen edge-on—. And when pressed it lights up from the
    // finger.
    let to_light = normalize(vec2<f32>(-0.55, -0.83));
    let rim = 1.0 - smoothstep(0.4, 3.0, inside);
    let facing = pow(max(dot(normal, to_light), 0.0), 1.6);
    let away = pow(max(-dot(normal, to_light), 0.0), 1.6);
    let fresnel = exp(-inside / 10.0);
    let shine = rim * (0.85 * facing + 0.4 * away + 0.12) + 0.14 * fresnel;
    let finger_dist = distance(p, u.backdrop.yz);
    let finger_light = u.backdrop.w * exp(-finger_dist * finger_dist / (2.0 * 70.0 * 70.0)) * 0.3;
    if (glass > 0.0 && u.backdrop.x > 0.5 && el.uv.z > 0.5) {
        // With what is behind at hand, the glass is that background bent by
        // the bevel, frosted, with a little of its tint. Red, green and blue
        // bend a hair differently: the colour fringe of a glass edge.
        let bevel = select(16.0, el.uv.y, el.uv.y > 0.5);
        // A glass that appears does not fade in: it starts bending the light.
        let shift = displacement(inside, bevel) * u.header.w * glass;
        let size = u.header.xy * u.header.w;
        let q = e.pos.xy - normal * shift;
        let tq = normal * shift * 0.06;
        // What is behind comes in premultiplied alpha —what is unknown, under
        // a text, has alpha 0 and weighs nothing—: it is divided by it.
        let br = textureSampleLevel(backdrop_blurred, backdrop_sampler, (q - tq) / size, 0.0);
        let bg = textureSampleLevel(backdrop_blurred, backdrop_sampler, q / size, 0.0);
        let bb = textureSampleLevel(backdrop_blurred, backdrop_sampler, (q + tq) / size, 0.0);
        let frosted = vec3<f32>(br.r / max(br.a, 0.001), bg.g / max(bg.a, 0.001), bb.b / max(bb.a, 0.001));
        let nr = textureSampleLevel(backdrop_sharp, backdrop_sampler, (q - tq) / size, 0.0);
        let ng = textureSampleLevel(backdrop_sharp, backdrop_sampler, q / size, 0.0);
        let nb = textureSampleLevel(backdrop_sharp, backdrop_sampler, (q + tq) / size, 0.0);
        let sharp = vec3<f32>(nr.r / max(nr.a, 0.001), ng.g / max(ng.a, 0.001), nb.b / max(nb.a, 0.001));
        let known = min(nr.a, min(ng.a, nb.a));
        // In the bevel what is bent looks fairly sharp; further in, frosted.
        let in_bevel = 1.0 - clamp(inside / bevel, 0.0, 1.0);
        let background = mix(frosted, sharp, smoothstep(0.0, 0.6, in_bevel) * 0.85 * known);
        // A glass livens up a little what it lets through.
        let gray = dot(background, vec3<f32>(0.299, 0.587, 0.114));
        let vivid = clamp(mix(vec3<f32>(gray), background, 1.18), vec3<f32>(0.0), vec3<f32>(1.0));
        // Over something bright, or over something with a lot of detail —text,
        // lines—, more tint: what is written on top can still be read. Apple
        // puts it this way: the shadow of the glass rises over text. The detail
        // is how much the frosted background varies a few steps from here,
        // which on a flat background is nothing.
        let bright = smoothstep(0.45, 0.9, gray);
        let step_px = 14.0 * u.header.w;
        let around = array<vec2<f32>, 4>(vec2<f32>(step_px, 0.0), vec2<f32>(-step_px, 0.0), vec2<f32>(0.0, step_px), vec2<f32>(0.0, -step_px));
        var variation = 0.0;
        for (var k = 0; k < 4; k++) {
            let m = textureSampleLevel(backdrop_blurred, backdrop_sampler, (q + around[k]) / size, 0.0);
            let l = dot(m.rgb / max(m.a, 0.001), vec3<f32>(0.299, 0.587, 0.114));
            variation += abs(l - gray);
        }
        let detail = smoothstep(0.03, 0.14, variation * 0.25);
        // The glass, with everything: tint, the darkening over text, and its
        // light. It all goes in the colour and nothing in the alpha, which stays
        // at LENS_ALPHA: the next capture is unmixed by dividing by 1 − α, and
        // with an alpha of 0.96 —a small ball is almost all edge— the rounding
        // got multiplied by 25, the background never came out and the ball
        // stayed black inside.
        var glass_color = mix(vivid, tone, LENS_TINT);
        glass_color = mix(glass_color, tone, max(bright * 0.5, detail * 0.55));
        glass_color = mix(glass_color, vec3<f32>(1.0), clamp(shine + finger_light, 0.0, 1.0));
        // What has to be seen is this glass. But the compositor puts the real
        // background, sharp, under our alpha, and it would leak through like a
        // ghost of the text behind: it is subtracted beforehand, since we know
        // it. What is seen: ours + (1 − α)·behind = glass · coverage.
        let seen = coverage(d) * alpha * glass;
        let db = textureSampleLevel(backdrop_sharp, backdrop_sampler, e.pos.xy / size, 0.0);
        let underneath = db.rgb / max(db.a, 0.001) * step(0.5, db.a);
        let lens_alpha = seen * LENS_ALPHA;
        let rgb = clamp(seen * (glass_color - (1.0 - LENS_ALPHA) * underneath), vec3<f32>(0.0), vec3<f32>(lens_alpha));
        c = vec4<f32>(rgb, lens_alpha) + c * (1.0 - lens_alpha);
    } else {
        // Without it, the fill is a tint: it lets what is behind show through
        // (which the compositor blurs) and keeps its colour; and on top, its light.
        c = over(c, tone, coverage(d) * alpha * mix(1.0, 0.36, glass));
        if (glass > 0.0) {
            c = over(c, vec3<f32>(1.0), clamp(shine + finger_light, 0.0, 1.0) * glass * coverage(d) * alpha);
        }
    }
    if (el.light.w > 0.0) {
        c = over(c, el.border.rgb, coverage(abs(d + el.light.w * 0.5) - el.light.w * 0.5) * alpha);
    }
    return c;
}

// ── a group's effects ─────────────────────────────────────────────
// Its layer, blurred over a disc of `radius` real pixels: 32 samples on a
// golden-angle spiral, weighted like a gaussian. Bilinear reads between them
// fill what 32 points leave; what is outside what was painted is transparent.
fn layer_blurred(layer: i32, at: vec2<f32>, radius: f32) -> vec4<f32> {
    let dims = vec2<f32>(textureDimensions(layers));
    var sum = vec4<f32>(0.0);
    var weights = 0.0;
    for (var k = 0; k < 32; k++) {
        let f = (f32(k) + 0.5) / 32.0;
        let r = radius * sqrt(f);
        let angle = f32(k) * 2.39996323;
        let w = exp(-2.0 * f);
        sum += textureSampleLevel(layers, atlas_sampler, (at + vec2<f32>(cos(angle), sin(angle)) * r) / dims, layer, 0.0) * w;
        weights += w;
    }
    return sum / weights;
}

// `hue: 120deg` arrives in radians, like every angle written with `deg`.
fn hue_rotate(c: vec3<f32>, a: f32) -> vec3<f32> {
    let k = vec3<f32>(0.57735027);
    return c * cos(a) + cross(k, c) * sin(a) + k * dot(k, c) * (1.0 - cos(a));
}

// color0: glow colour, and how much · color1: saturation, brightness, contrast, hue
// line: the mask's points · light: blur, glow radius, mask kind (1 linear, 2 radial), add
// border: whether the glow has its own colour, whether there are effects
fn layer_with_effects(el: Element, at: vec2<f32>, p: vec2<f32>, alpha: f32) -> vec4<f32> {
    let layer = i32(el.header.y);
    let scale = u.header.w;
    var c = textureLoad(layers, vec2<i32>(at), layer, 0);
    if (el.light.x > 0.25) { c = layer_blurred(layer, at, el.light.x * scale); }
    // The glow goes under what is inside: light that spills from its edges.
    if (el.light.y > 0.25 && el.color0.w > 0.0) {
        let g = layer_blurred(layer, at, el.light.y * scale);
        var glow = g * el.color0.w;
        if (el.border.x > 0.5) { glow = vec4<f32>(el.color0.rgb, 1.0) * g.a * el.color0.w; }
        c = c + glow * (1.0 - c.a);
    }
    // Colour: on the straight colour, not the premultiplied one.
    let tone = el.color1;
    if (any(tone != vec4<f32>(1.0, 1.0, 1.0, 0.0)) && c.a > 0.0) {
        var rgb = c.rgb / c.a;
        rgb = rgb * tone.y;
        rgb = (rgb - vec3<f32>(0.5)) * tone.z + vec3<f32>(0.5);
        let gray = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        rgb = mix(vec3<f32>(gray), rgb, tone.x);
        if (tone.w != 0.0) { rgb = hue_rotate(rgb, tone.w); }
        c = vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * c.a, c.a);
    }
    // The mask, where the group is (it moves with it): whole at the start, nothing at the end.
    var m = 1.0;
    let local = to_local(p, el.t0, el.t1);
    if (el.light.z > 0.5 && el.light.z < 1.5) {
        let axis = el.line.zw - el.line.xy;
        m = 1.0 - smoothstep(0.0, 1.0, dot(local - el.line.xy, axis) / max(dot(axis, axis), 0.0001));
    } else if (el.light.z > 1.5) {
        m = 1.0 - smoothstep(el.line.z, max(el.line.w, el.line.z + 0.001), distance(local, el.line.xy));
    }
    c = c * m * alpha;
    // Added: light that adds up instead of covering. Premultiplied with no alpha
    // is exactly that, here and in the compositor.
    if (el.light.w > 0.5) { return vec4<f32>(c.rgb, 0.0); }
    return c;
}

// ── particles ─────────────────────────────────────────────────────
// No particle is stored: each one is worked out here from its number and the
// time. Its emitter's element brings everything (see `Instr::Particles` in gpu.rs):
// color0 colour and opacity at birth · color1 at death · line where, and the box
// it is born in · light life a..b, speed a..b · border direction, spread, drag,
// shape · shadow gravity, size at birth and at death · dest now, switched on,
// switched off, last burst · uv the emitter's velocity, whether it bursts.
const PARTICLE_SLOTS: u32 = 4096u;

fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

// A number from 0 to 1, always the same for the same three.
fn chance(a: u32, b: u32, k: u32) -> f32 {
    return f32(pcg(a ^ pcg(b ^ pcg(k)))) / 4294967295.0;
}

struct ParticleOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) element: u32,
    @location(1) quad: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) world: vec2<f32>,
    // Its half size along and across, in pixels, and its shape.
    @location(4) @interpolate(flat) extent: vec3<f32>,
};

@vertex
fn vs_particle(@builtin(vertex_index) v: u32, @builtin(instance_index) instance: u32) -> ParticleOut {
    var out: ParticleOut;
    let k = instance / PARTICLE_SLOTS;
    let i = instance % PARTICLE_SLOTS;
    let el = elements[k];
    out.element = k;
    out.pos = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    let seed = u32(el.header.y);
    let count = el.header.z;
    let now = el.dest.x;
    // When it was born, and in which round —each round, another particle—.
    var born = 0.0;
    var round = 0u;
    let lifetime_most = max(el.light.x, el.light.y);
    if (el.uv.z > 0.5) {
        born = el.dest.w + chance(seed, i, 7u) * 0.06;
        round = u32(max(el.dest.w, 0.0) * 1000.0);
        if (el.dest.w < 0.0) { return out; }
    } else {
        // A steady stream: each slot is born once per `lifetime_most`, spread evenly.
        let phase = f32(i) / count * lifetime_most;
        let since = now - el.dest.y - phase;
        if (since < 0.0) { return out; }
        var n = floor(since / lifetime_most);
        born = el.dest.y + phase + n * lifetime_most;
        // Switched off: the ones born after it are not there; the last ones finish.
        if (born > el.dest.z) {
            n -= 1.0;
            born -= lifetime_most;
            if (n < 0.0) { return out; }
        }
        round = u32(n);
    }
    let life = mix(el.light.x, el.light.y, chance(seed, i, round * 11u + 1u));
    let age = now - born;
    if (age < 0.0 || age > life) { return out; }
    let t = age / life;
    // Where it was born: somewhere in the box, where the emitter was then.
    let box = (vec2<f32>(chance(seed, i, round * 11u + 2u), chance(seed, i, round * 11u + 3u)) - 0.5) * el.line.zw;
    let start = el.line.xy + box - el.uv.xy * age;
    let angle = el.border.x + (chance(seed, i, round * 11u + 4u) - 0.5) * el.border.y;
    let speed = mix(el.light.z, el.light.w, chance(seed, i, round * 11u + 5u));
    let v0 = vec2<f32>(cos(angle), sin(angle)) * speed;
    let g = el.shadow.xy;
    let drag = el.border.z;
    var at = vec2<f32>(0.0);
    var velocity = vec2<f32>(0.0);
    if (drag > 0.001) {
        let slow = exp(-drag * age);
        let f = (1.0 - slow) / drag;
        at = start + v0 * f + g * (age - f) / drag;
        velocity = v0 * slow + g * (1.0 - slow) / drag;
    } else {
        at = start + v0 * age + 0.5 * g * age * age;
        velocity = v0 + g * age;
    }
    let size = mix(el.shadow.z, el.shadow.w, t);
    let alpha = mix(el.color0.w, el.color1.w, t) * el.header.w;
    if (size <= 0.01 || alpha <= 0.001) { return out; }
    // Its quad: a square around it, or a streak along where it goes.
    var along = vec2<f32>(1.0, 0.0);
    var extent = vec2<f32>(size * 0.5, size * 0.5);
    let shape = el.border.w;
    if (shape > 1.5) {
        let fast = length(velocity);
        if (fast > 0.001) { along = velocity / fast; }
        extent = vec2<f32>(size * 0.5 + fast * 0.03, max(size * 0.22, 0.6));
    }
    let across = vec2<f32>(-along.y, along.x);
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let q = corners[v];
    // One pixel more on each side, for the edge's softness.
    let pad = 1.0 / u.header.w;
    let local = at + along * q.x * (extent.x + pad) + across * q.y * (extent.y + pad);
    // Back from the group's space to the scene's: `to_local` inverted.
    let det = el.t0.x * el.t0.w - el.t0.y * el.t0.z;
    let d = local - el.t1.xy;
    let world = vec2<f32>(el.t0.w * d.x - el.t0.y * d.y, -el.t0.z * d.x + el.t0.x * d.y) / det;
    let screen = world - u.hud.xw;
    out.pos = vec4<f32>(screen.x / u.header.x * 2.0 - 1.0, 1.0 - screen.y / u.header.y * 2.0, 0.0, 1.0);
    out.quad = q * (extent + vec2<f32>(pad)) ;
    out.color = vec4<f32>(mix(el.color0.rgb, el.color1.rgb, t), alpha);
    out.world = world;
    out.extent = vec3<f32>(extent, shape);
    return out;
}

@fragment
fn fs_particle(e: ParticleOut) -> @location(0) vec4<f32> {
    let el = elements[e.element];
    var clip = 1.0;
    for (var k = 0; k < 4; k++) {
        let r = el.clips[k];
        if (r >= 0.0) { clip *= coverage(shape_distance(u32(r), e.world)); }
    }
    // Its distance to its own edge, in logical pixels, like any shape's.
    var d = 0.0;
    let x = e.extent.xy;
    if (e.extent.z > 1.5) {
        // A streak: a capsule along x.
        let h = max(x.x - x.y, 0.0);
        let q = vec2<f32>(max(abs(e.quad.x) - h, 0.0), e.quad.y);
        d = length(q) - x.y;
    } else if (e.extent.z > 0.5) {
        let q = abs(e.quad) - x;
        d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0);
    } else {
        d = length(e.quad) - x.x;
    }
    let a = e.color.a * coverage(d) * clip;
    if (a <= 0.001) { discard; }
    return vec4<f32>(e.color.rgb * a, a);
}

// ── a text's effects ──────────────────────────────────────────────
// How much of the letter covers this point; nothing outside its piece of the atlas.
fn glyph_at(el: Element, q: vec2<f32>) -> f32 {
    let uv01 = (q - el.dest.xy) / max(el.dest.zw, vec2<f32>(1.0));
    if (uv01.x < 0.0 || uv01.x > 1.0 || uv01.y < 0.0 || uv01.y > 1.0) { return 0.0; }
    return textureSampleLevel(atlas, atlas_sampler, mix(el.uv.xy, el.uv.zw, uv01), 0.0).a;
}

// color0 the letter's colour · color1 the shadow's colour, and the gradient's kind ·
// line the gradient's points · light first stop, how many, -, «has effects» ·
// border the outline's colour and width · shadow its offset, blur and alpha
fn text_with_effects(el: Element, q: vec2<f32>, alpha: f32) -> vec4<f32> {
    // The letter, firmed up like any other (see the contrast curve above).
    let lum = dot(el.color0.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    let k = mix(0.3, 1.0, lum);
    let raw = glyph_at(el, q);
    let fill = raw * (k + 1.0) / (raw * k + 1.0);
    var c = vec4<f32>(0.0);
    // The shadow: the letter moved and blurred, under everything.
    if (el.shadow.w > 0.0) {
        // Blurred over a disc of `blur`: 24 points on a golden-angle spiral,
        // weighted like a gaussian —with a 3 × 3 grid, a wide blur came out as
        // stacked copies of the letter—.
        let at = q - el.shadow.xy;
        let radius = el.shadow.z;
        var s = 0.0;
        var weights = 0.0;
        for (var i = 0; i < 24; i++) {
            let f = (f32(i) + 0.5) / 24.0;
            let r = radius * sqrt(f);
            let a = f32(i) * 2.39996323;
            let w = exp(-2.0 * f);
            s += glyph_at(el, at + vec2<f32>(cos(a), sin(a)) * r) * w;
            weights += w;
        }
        c = vec4<f32>(el.color1.rgb, 1.0) * (s / weights) * el.shadow.w;
    }
    // The outline: the letter grown by its width, in its colour, under the fill.
    if (el.border.w > 0.0) {
        let w = el.border.w;
        var o = raw;
        for (var i = 0; i < 12; i++) {
            let a = f32(i) * 0.5235988;
            let dir = vec2<f32>(cos(a), sin(a));
            o = max(o, glyph_at(el, q + dir * w));
            o = max(o, glyph_at(el, q + dir * w * 0.5));
        }
        c = over(c, el.border.rgb, clamp(o * 1.4, 0.0, 1.0));
    }
    // The fill: its colour, or its gradient at this point.
    var tone = el.color0.rgb;
    if (el.color1.w > 0.5) {
        var t = 0.0;
        if (el.color1.w > 1.5) {
            t = clamp(length(q - el.line.xy) / max(el.line.z, 0.0001), 0.0, 1.0);
        } else {
            let axis = el.line.zw - el.line.xy;
            t = clamp(dot(q - el.line.xy, axis) / max(dot(axis, axis), 0.0001), 0.0, 1.0);
        }
        tone = between_stops(u32(el.light.x), u32(el.light.y), t);
    }
    c = over(c, tone, fill);
    if (c.a <= 0.001) { discard; }
    return c * alpha;
}
