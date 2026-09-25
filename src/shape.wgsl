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
    let p = mix(c.xy, c.zw, corners[v]);
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
        return textureLoad(layers, vec2<i32>(e.pos.xy), i32(el.header.y), 0) * alpha;
    }
    if (kind == SHADER) {
        return user_shader(el, p) * alpha;
    }
    let local = to_local(p, el.t0, el.t1);
    if (kind == TEXTURE) {
        let q = local;
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
