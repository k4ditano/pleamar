//! A zone's geometry is shared by hit testing and the native input region.
//! Rebuild only when one of its inputs changes, including spring velocity.
use crate::scene::{Ctx, Expr, Zone};
use crate::shapes::{Affine, FlatShape, PathStep, Shape};

pub(super) struct Geometry {
    props: Vec<u16>,
    facts: Vec<u16>,
    seen: Vec<f32>,
    shape: Option<FlatShape>,
    points: Vec<f32>,
    clips: Vec<FlatShape>,
    clip_points: Vec<f32>,
    bounds: Option<[f32; 4]>,
}

impl Geometry {
    // Recreated on every scene load, even if the replacement has the same length.
    pub(super) fn new(zone: &Zone) -> Self {
        let (mut props, mut facts) = (Vec::new(), Vec::new());
        let mut input = |e: &Expr| e.inputs(&mut props, &mut facts);
        shape_inputs(&zone.shape, &mut input);
        for clip in &zone.viewports { shape_inputs(&clip.shape, &mut input); }
        for t in &zone.under {
            for e in [&t.pivot.0, &t.pivot.1, &t.rotate, &t.scale.0, &t.scale.1, &t.translate.0, &t.translate.1] { input(e); }
        }
        Self { props, facts, seen: Vec::new(), shape: None, points: Vec::new(), clips: Vec::new(), clip_points: Vec::new(), bounds: None }
    }

    fn update(&mut self, zone: &Zone, c: Ctx) {
        let values = self.facts.iter().map(|k| c.facts[*k as usize])
            .chain(self.props.iter().flat_map(|k| [c.props[*k as usize].x, c.props[*k as usize].v]));
        if self.shape.is_some() && values.clone().eq(self.seen.iter().copied()) { return; }
        self.seen.clear();
        self.seen.extend(values);
        self.points.clear();
        self.clip_points.clear();
        self.clips.clear();
        let mut shape = zone.shape.flatten_into(c, &mut self.points);
        shape.affine = zone.under.iter().fold(Affine::IDENTITY, |a, t| a.mul(t.affine(c)));
        let mut bounds = shape.bounds();
        for viewport in &zone.viewports {
            let mut clip = viewport.shape.flatten_into(c, &mut self.clip_points);
            clip.affine = zone.under[..viewport.under].iter().fold(Affine::IDENTITY, |a, t| a.mul(t.affine(c)));
            bounds = bounds.zip(clip.bounds()).and_then(|(a, b)| {
                let b = [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])];
                (b[0] < b[2] && b[1] < b[3]).then_some(b)
            });
            self.clips.push(clip);
        }
        self.shape = Some(shape);
        self.bounds = bounds;
    }

    pub(super) fn bounds(&mut self, zone: &Zone, c: Ctx) -> Option<[f32; 4]> {
        self.update(zone, c);
        self.bounds
    }

    pub(super) fn contains(&mut self, zone: &Zone, c: Ctx, x: f32, y: f32) -> bool {
        self.update(zone, c);
        !self.clips.iter().any(|clip| clip.distance_with(x, y, &self.clip_points) >= 0.0)
            && self.shape.as_ref().is_some_and(|shape| shape.distance_with(x, y, &self.points) < 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Animated, Cursor, FactId, PropId, Spring, Transform, ViewportClip};

    fn zone(shape: Shape) -> Zone {
        Zone { id: "cached", shape, active: 1.0.into(), cursor: Cursor::Normal,
            under: Vec::new(), viewports: Vec::new(), at: 0, zblock: None, carries: None,
            label: None, reach: crate::scene::Reach::Any, scrolls: None, within: None, told: None }
    }

    fn compare(cache: &mut Geometry, zone: &Zone, c: Ctx) {
        assert_eq!(cache.bounds(zone, c), zone.bounds(c));
        for x in (-24..=24).step_by(3) {
            for y in (-24..=24).step_by(3) {
                assert_eq!(cache.contains(zone, c, x as f32, y as f32), zone.contains(c, x as f32, y as f32), "at ({x}, {y})");
            }
        }
    }

    #[test]
    fn cached_geometry_matches_shapes_transforms_clips_and_velocity_changes() {
        let p = Expr::P(PropId(0));
        let h = Expr::H(FactId(0));
        let v = Expr::Vel(PropId(0));
        let point = (p.clone(), h.clone());
        let shapes = [
            Shape::Ellipse { center: point.clone(), radius: h.clone(), scale: (1.0.into(), p.clone()) },
            Shape::Rect { center: point.clone(), half_size: (h.clone(), p.clone()), radius: v.clone() },
            Shape::Arc { center: point.clone(), radius: h.clone(), opening: p.clone(), thickness: v.clone() },
            Shape::Segment { from: point.clone(), to: (h.clone(), v.clone()), thickness: p.clone() },
            Shape::Path { origin: point.clone(), steps: vec![PathStep::Line((10.0.into(), v.clone())), PathStep::Curve { via: (h.clone(), 15.0.into()), to: (0.0.into(), 12.0.into()) }], closed: true }.rotated(v.clone()).stroke(p.clone()),
        ];
        for shape in shapes {
            let mut zone = zone(shape);
            zone.under = vec![Transform::at((1.0.into(), h.clone())).rotate(v.clone()).scale(1.0, p.clone()).translate(h.clone(), 2.0),
                Transform::at((p.clone(), 0.0.into())).translate(0.0, v.clone())];
            let path = Shape::Path { origin: ((-20.0).into(), (-20.0).into()), steps: vec![
                PathStep::Line((20.0.into(), (-20.0).into())), PathStep::Line((20.0.into(), h.clone())), PathStep::Line(((-20.0).into(), h.clone()))], closed: true };
            zone.viewports = vec![ViewportClip { shape: path.clone(), under: 0 }, ViewportClip { shape: path, under: 1 }];
            let mut cache = Geometry::new(&zone);
            let mut props = [Animated::at(1.0, Spring::at(0.2))];
            let mut facts = [12.0];
            for step in 0..8 {
                match step { 1 => props[0].x = 2.0, 2 => props[0].v = 0.4, 3 => facts[0] = 4.0, 4 => props[0].x = 0.0,
                    5 => props[0].x = -1.0, 6 => facts[0] = 0.0, _ => {} }
                compare(&mut cache, &zone, Ctx { props: &props, facts: &facts });
            }
        }
    }

    #[test]
    fn inactive_constant_and_reloaded_zones_do_not_retain_old_geometry() {
        let mut zone = zone(Shape::circle((0.0.into(), 0.0.into()), 10.0));
        let c = Ctx { props: &[], facts: &[] };
        let mut cache = Geometry::new(&zone);
        compare(&mut cache, &zone, c);
        let memory = (cache.seen.capacity(), cache.points.capacity(), cache.clip_points.capacity());
        for _ in 0..100 { compare(&mut cache, &zone, c); }
        assert_eq!(memory, (cache.seen.capacity(), cache.points.capacity(), cache.clip_points.capacity()));
        zone.shape = Shape::circle((30.0.into(), 30.0.into()), 2.0);
        cache = Geometry::new(&zone);
        assert!(!cache.contains(&zone, c, 0.0, 0.0));
        assert!(cache.contains(&zone, c, 30.0, 30.0));
    }

    #[test]
    #[ignore = "set PLEAMAR_ZONE_BENCH_SCENE to profile a local scene without opening windows"]
    fn scene_geometry_benchmark() {
        let path = std::env::var("PLEAMAR_ZONE_BENCH_SCENE").expect("a benchmark scene path");
        let scene = crate::read_scene(&path).unwrap();
        let mut props: Vec<_> = scene.props.iter().map(|(_, x, s)| Animated::at(*x, *s)).collect();
        let facts: Vec<_> = scene.facts.iter().map(|(_, value)| *value).collect();
        let mut caches: Vec<_> = scene.zones.iter().map(Geometry::new).collect();
        let mut baseline = 0.0f32;
        let mut cached = 0.0f32;
        for changing in [false, true] {
            for use_cache in [false, true] {
                let start = std::time::Instant::now();
                for frame in 0..300 {
                    if changing { for p in &mut props { p.x += if frame % 2 == 0 { 0.01 } else { -0.01 }; } }
                    let c = Ctx { props: &props, facts: &facts };
                    for (z, cache) in scene.zones.iter().zip(&mut caches) {
                        let (b, inside) = if use_cache { (cache.bounds(z, c), cache.contains(z, c, 100.0, 100.0)) }
                            else { (z.bounds(c), z.contains(c, 100.0, 100.0)) };
                        let value = b.map_or(0.0, |b| b.iter().sum::<f32>()) + u8::from(inside) as f32;
                        if use_cache { cached += std::hint::black_box(value); } else { baseline += std::hint::black_box(value); }
                    }
                }
                println!("zone benchmark: changing={changing}, cached={use_cache}, zones={}, microseconds/round={:.2}", scene.zones.len(), start.elapsed().as_micros() as f64 / 300.0);
            }
        }
        assert!((baseline - cached).abs() < 1.0, "{baseline} != {cached}");
    }
}

fn shape_inputs(shape: &Shape, input: &mut impl FnMut(&Expr)) {
    match shape {
        Shape::Ellipse { center, radius, scale } => {
            for e in [&center.0, &center.1, radius, &scale.0, &scale.1] { input(e); }
        }
        Shape::Rect { center, half_size, radius } => {
            for e in [&center.0, &center.1, &half_size.0, &half_size.1, radius] { input(e); }
        }
        Shape::Arc { center, radius, opening, thickness } => {
            for e in [&center.0, &center.1, radius, opening, thickness] { input(e); }
        }
        Shape::Segment { from, to, thickness } => {
            for e in [&from.0, &from.1, &to.0, &to.1, thickness] { input(e); }
        }
        Shape::Rotated(shape, value) | Shape::Stroke(shape, value) => {
            shape_inputs(shape, input);
            input(value);
        }
        Shape::Path { origin, steps, .. } => {
            input(&origin.0); input(&origin.1);
            for step in steps {
                match step {
                    PathStep::Line(p) => { input(&p.0); input(&p.1); }
                    PathStep::Curve { via, to } => {
                        for e in [&via.0, &via.1, &to.0, &to.1] { input(e); }
                    }
                }
            }
        }
    }
}
