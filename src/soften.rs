//! A group's `blur`, done for real. A group with effects is painted on a layer
//! of its own; where it is blurred, that layer is blurred in place before it is
//! blended, in a few passes: halved as many times as the radius allows (each
//! halving a 2 × 2 average), a gaussian across and then down at that size, and
//! doubled back up, smoothly, to its own size. A wide blur costs about what a
//! narrow one does, and comes out smooth: the 32 samples on a disc it used to
//! be left copies of a line side by side and grain in a text past a dozen
//! pixels.
//!
//! What is outside where the group was painted this frame counts as
//! transparent: a layer holds whatever earlier frames left there, and a blur
//! that read it would pull it in.

use std::cell::{Cell, RefCell};

/// The most halvings: at 1/32 a radius of hundreds of pixels is a few.
const MOST_DOWN: u32 = 5;

/// How wide the gaussian is let to be, in pixels of the size it runs at,
/// before halving once more is better.
const WIDEST: f32 = 2.0;

/// Uniform slots, one per pass of a frame; their stride is what every card
/// accepts for a dynamic offset.
const SLOTS: u64 = 512;
const STRIDE: u64 = 256;

const SOURCE: &str = "
struct U {
    // 0 halve · 1 gaussian across · 2 gaussian down · 3 double
    mode: u32,
    radius: i32,
    sigma: f32,
    _pad: f32,
    // Where what is read holds something this frame, in its texels: x0, y0, x1, y1.
    valid: vec4<i32>,
};
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<uniform> u: U;

@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

fn at(p: vec2<i32>) -> vec4<f32> {
    if (p.x < u.valid.x || p.y < u.valid.y || p.x >= u.valid.z || p.y >= u.valid.w) {
        return vec4<f32>(0.0);
    }
    return textureLoad(src, p, 0);
}

@fragment fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(floor(pos.xy));
    if (u.mode == 0u) {
        let q = p * 2;
        return (at(q) + at(q + vec2<i32>(1, 0)) + at(q + vec2<i32>(0, 1)) + at(q + vec2<i32>(1, 1))) * 0.25;
    }
    if (u.mode == 3u) {
        // Where this pixel's centre falls on the half-size picture, between four of its texels.
        let c = (pos.xy) * 0.5 - 0.5;
        let b = vec2<i32>(floor(c));
        let f = c - floor(c);
        let top = mix(at(b), at(b + vec2<i32>(1, 0)), f.x);
        let bottom = mix(at(b + vec2<i32>(0, 1)), at(b + vec2<i32>(1, 1)), f.x);
        return mix(top, bottom, f.y);
    }
    let dir = select(vec2<i32>(0, 1), vec2<i32>(1, 0), u.mode == 1u);
    var sum = vec4<f32>(0.0);
    var weights = 0.0;
    for (var t = -u.radius; t <= u.radius; t++) {
        let w = exp(-0.5 * f32(t * t) / (u.sigma * u.sigma));
        sum += at(p + dir * t) * w;
        weights += w;
    }
    return sum / weights;
}
";

pub struct Soften {
    scratch: wgpu::RenderPipeline,
    onto: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    /// The next free slot of `uniforms` this frame.
    next: Cell<u64>,
    warned: Cell<bool>,
}

/// A sheet's scratch pictures: at each size, from the whole down to 1/32, two
/// —one is read while the other is written—. Made as they are first needed,
/// and let go after `IDLE_FRAMES` frames without a blur (a 1080p sheet's are
/// some 13 MB of the card's, and a blur is often a moment: a panel coming in).
#[derive(Default)]
pub struct Levels {
    px: (u32, u32),
    by_size: RefCell<Vec<Option<[(wgpu::Texture, wgpu::TextureView); 2]>>>,
    /// Frames painted since one of them was last used.
    idle: Cell<u32>,
}

/// About two seconds at 60 Hz.
const IDLE_FRAMES: u32 = 120;

impl Levels {
    fn level(&self, d: &wgpu::Device, k: u32) -> [wgpu::TextureView; 2] {
        self.idle.set(0);
        let mut v = self.by_size.borrow_mut();
        if v.len() <= k as usize {
            v.resize_with(k as usize + 1, || None);
        }
        let made = v[k as usize].get_or_insert_with(|| {
            let size = ((self.px.0 >> k).max(1) + 1, (self.px.1 >> k).max(1) + 1);
            let one = || crate::lens::texture(d, "blurred layer", size, wgpu::TextureFormat::Rgba16Float, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
            [one(), one()]
        });
        [made[0].1.clone(), made[1].1.clone()]
    }
}

/// A box in a picture's texels: x0, y0, x1, y1.
type Box4 = [i32; 4];

impl Soften {
    pub fn new(d: &wgpu::Device, layer_format: wgpu::TextureFormat) -> Soften {
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("soften"), source: wgpu::ShaderSource::Wgsl(SOURCE.into()) });
        let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("soften"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(32) },
                    count: None,
                },
            ],
        });
        let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("soften"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = |format| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("soften"),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let uniforms = d.create_buffer(&wgpu::BufferDescriptor { label: Some("soften"), size: SLOTS * STRIDE, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        Soften { scratch: pipeline(wgpu::TextureFormat::Rgba16Float), onto: pipeline(layer_format), uniforms, next: Cell::new(0), warned: Cell::new(false) }
    }

    /// A new frame: its passes start from the first slot again.
    pub fn new_frame(&self) {
        self.next.set(0);
    }

    /// The scratch pictures a sheet of that size needs, kept if it already has them.
    pub fn fit(levels: &mut Levels, px: (u32, u32)) {
        if levels.px != px {
            *levels = Levels { px, ..Default::default() };
        }
        let idle = levels.idle.get().saturating_add(1);
        levels.idle.set(idle);
        if idle > IDLE_FRAMES && !levels.by_size.borrow().is_empty() {
            levels.by_size.borrow_mut().clear();
        }
    }

    /// How many halvings, and the gaussian left for that size, for a blur of
    /// `sigma` pixels: what halving and doubling back blur on their way is
    /// taken off it.
    fn plan(sigma: f32) -> (u32, f32) {
        let mut k = 0;
        while k < MOST_DOWN && sigma / (1u32 << k) as f32 > WIDEST {
            k += 1;
        }
        let s = (1u32 << k) as f32;
        let left = ((sigma * sigma - Self::on_the_way(k)).max(0.0)).sqrt() / s;
        (k, left.max(0.35))
    }

    /// The variance, in pixels squared, that `k` halvings and as many
    /// doublings back add by themselves: a halving into size `j` averages two
    /// texels 2^(j−1) apart (a box of that width), a doubling from it
    /// interpolates between two 2^j apart (a tent).
    fn on_the_way(k: u32) -> f32 {
        (1..=k).map(|j| {
            let (half, whole) = ((1u32 << (j - 1)) as f32, (1u32 << j) as f32);
            half * half / 4.0 + whole * whole / 6.0
        }).sum()
    }

    /// Blurs `layer` (a sheet's layer, `px` big) in place, by a gaussian of
    /// `sigma` pixels, where the group is: `area`, in its pixels, which holds
    /// all it painted and as far as the blur spills.
    pub fn run(&self, d: &wgpu::Device, q: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, levels: &Levels, layer: &wgpu::TextureView, area: Box4, sigma: f32) {
        let px = (levels.px.0 as i32, levels.px.1 as i32);
        let area = [area[0].max(0), area[1].max(0), area[2].min(px.0), area[3].min(px.1)];
        if area[2] <= area[0] || area[3] <= area[1] || sigma < 0.2 {
            return;
        }
        let (k, left) = Self::plan(sigma);
        let passes = if k == 0 { 2 } else { 2 * k as u64 + 2 };
        let first = self.next.get();
        if first + passes > SLOTS {
            if !self.warned.replace(true) {
                eprintln!("render · too many blurred groups in one frame: the rest are painted sharp");
            }
            return;
        }
        self.next.set(first + passes);
        let mut slot = first;
        let mut go = |enc: &mut wgpu::CommandEncoder, pipeline: &wgpu::RenderPipeline, from: &wgpu::TextureView, valid: Box4, onto: &wgpu::TextureView, scissor: Box4, mode: u32, sigma: f32| {
            let radius = if mode == 1 || mode == 2 { (sigma * 3.0).ceil() as i32 } else { 0 };
            let mut bytes = [0u8; 32];
            bytes[0..4].copy_from_slice(&mode.to_ne_bytes());
            bytes[4..8].copy_from_slice(&radius.to_ne_bytes());
            bytes[8..12].copy_from_slice(&sigma.to_ne_bytes());
            for (i, v) in valid.iter().enumerate() {
                bytes[16 + i * 4..20 + i * 4].copy_from_slice(&v.to_ne_bytes());
            }
            q.write_buffer(&self.uniforms, slot * STRIDE, &bytes);
            let layout = pipeline.get_bind_group_layout(0);
            let group = d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(from) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &self.uniforms, offset: 0, size: wgpu::BufferSize::new(32) }) },
                ],
            });
            let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("soften"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: onto,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            p.set_pipeline(pipeline);
            p.set_bind_group(0, &group, &[(slot * STRIDE) as u32]);
            p.set_scissor_rect(scissor[0] as u32, scissor[1] as u32, (scissor[2] - scissor[0]).max(1) as u32, (scissor[3] - scissor[1]).max(1) as u32);
            p.draw(0..3, 0..1);
            slot += 1;
        };
        // The box at each size: what covers the area, rounded outwards.
        let at = |j: u32| -> Box4 { [area[0] >> j, area[1] >> j, (area[2] + (1 << j) - 1) >> j, (area[3] + (1 << j) - 1) >> j] };
        if k == 0 {
            let [a, _] = levels.level(d, 0);
            go(enc, &self.scratch, layer, area, &a, area, 1, left);
            go(enc, &self.onto, &a, area, layer, area, 2, left);
            return;
        }
        // Down: the layer, then each half the one before.
        let mut from = layer.clone();
        for j in 1..=k {
            let [a, _] = levels.level(d, j);
            go(enc, &self.scratch, &from, at(j - 1), &a, at(j), 0, 0.0);
            from = a;
        }
        // The gaussian at the smallest size, across into the other picture and down back.
        let [a, b] = levels.level(d, k);
        go(enc, &self.scratch, &a, at(k), &b, at(k), 1, left);
        go(enc, &self.scratch, &b, at(k), &a, at(k), 2, left);
        // Up: into the other picture of each size, and last onto the layer.
        let mut from = a;
        for j in (1..k).rev() {
            let [_, b] = levels.level(d, j);
            go(enc, &self.scratch, &from, at(j + 1), &b, at(j), 3, 0.0);
            from = b;
        }
        go(enc, &self.onto, &from, at(1), layer, area, 3, 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::Soften;

    #[test]
    fn the_blur_left_after_halving_adds_up_to_what_was_asked() {
        for sigma in [0.5f32, 1.5, 3.0, 8.0, 24.0, 60.0] {
            let (k, left) = Soften::plan(sigma);
            let s = (1u32 << k) as f32;
            let total = ((left * s).powi(2) + Soften::on_the_way(k)).sqrt();
            assert!((total - sigma).abs() / sigma < 0.1, "sigma {sigma}: {k} halvings and {left} left give {total}");
            assert!(left <= super::WIDEST + 0.01, "sigma {sigma}: {left} left at 1/{s}");
        }
    }
}
