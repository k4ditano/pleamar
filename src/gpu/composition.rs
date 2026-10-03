//! The parts of composition that depend on instruction order, not animation.
use crate::scene::Instr;

pub(crate) struct Composition {
    pub sequence: Vec<usize>,
    /// Zero means the group must be visited; other values point past its close.
    pub jumps: Vec<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::*;

    // The previous per-frame algorithm, retained as an independent oracle.
    fn reference(instrs: &[Instr], order: Option<&[usize]>) -> Composition {
        let sequence: Vec<_> = order.map_or_else(|| (0..instrs.len()).collect(), <[usize]>::to_vec);
        let mut closes = vec![usize::MAX; sequence.len()];
        let mut emitters_before = vec![0u32; sequence.len() + 1];
        let mut open = Vec::new();
        for (p, &idx) in sequence.iter().enumerate() {
            emitters_before[p + 1] = emitters_before[p] + matches!(instrs[idx], Instr::Particles(_)) as u32;
            match &instrs[idx] {
                Instr::Opacity(Some(_)) | Instr::Fade(_) | Instr::Effect(_) => open.push(p),
                Instr::Opacity(None) => { if let Some(o) = open.pop() { closes[o] = p; } }
                _ => {}
            }
        }
        let jumps = closes.iter().enumerate().map(|(p, &close)| {
            if close != usize::MAX && emitters_before[close] == emitters_before[p] { close + 1 } else { 0 }
        }).collect();
        Composition { sequence, jumps }
    }

    #[test]
    fn groups_effects_emitters_and_reordered_siblings_match_previous_traversal() {
        for file in ["particles", "group-effects", "group-modes", "z-order"] {
            let scene = crate::read_scene(&format!("{}/tests/{file}.plm", env!("CARGO_MANIFEST_DIR"))).unwrap();
            let props: Vec<_> = scene.props.iter().map(|(_, x, s)| Animated::at(*x, *s)).collect();
            let mut facts: Vec<_> = scene.facts.iter().map(|(_, value)| *value).collect();
            for top in [0.0, 1.0] {
                if let Some(k) = scene.facts.iter().position(|(name, _)| *name == "top") { facts[k] = top; }
                let arranged = scene.z_arrange(Ctx { props: &props, facts: &facts });
                let order = arranged.as_ref().map(|a| a.order.as_slice());
                let mut instrs = scene.instrs.clone();
                for wrapped in [false, true] {
                    if wrapped {
                        instrs.insert(0, Instr::Fade(0.0.into()));
                        instrs.push(Instr::Opacity(None));
                    }
                    let order = if wrapped { None } else { order };
                    let plan = Composition::new(&instrs, order);
                    let original = reference(&instrs, order);
                    assert_eq!(plan.sequence, original.sequence, "{file}");
                    assert_eq!(plan.jumps, original.jumps, "{file}");
                    if wrapped && instrs.iter().any(|i| matches!(i, Instr::Particles(_))) {
                        assert_eq!(plan.jumps[0], 0, "the emitter clock must run while hidden");
                    }
                }
            }
        }
    }

    #[test]
    fn diagnostic_banner_follows_z_order_and_same_length_reload_rebuilds_groups() {
        let mut instrs = vec![Instr::Fade(0.0.into()), Instr::Transform(None), Instr::Opacity(None), Instr::Transform(None)];
        let plan = Composition::new(&instrs, Some(&[3, 0, 1, 2]));
        assert_eq!(plan.jumps, [0, 4, 0, 0]);
        instrs.extend([Instr::Opacity(Some(1.0.into())), Instr::Opacity(None)]);
        let plan = Composition::new(&instrs, Some(&[3, 0, 1, 2]));
        assert_eq!(plan.sequence, [3, 0, 1, 2, 4, 5]);
        assert_eq!(plan.jumps[4], 6);
        instrs[0] = Instr::Transform(None);
        let plan = Composition::new(&instrs, Some(&[3, 0, 1, 2]));
        assert_eq!(plan.jumps[1], 0);
    }

    #[test]
    fn reused_composition_keeps_drawing_and_hidden_regions_across_visibility_changes() {
        use crate::gpu::DrawList;
        let solid = |x: f32| Instr::Solid { shape: Shape::circle((x.into(), 40.0.into()), 12.0),
            color: [1.0.into(), 0.5.into(), 0.2.into()], alpha: 1.0.into(), glass_spec: None };
        let instrs = vec![Instr::Opacity(Some(FactId(0).into())), solid(30.0), Instr::Opacity(None),
            Instr::Fade(FactId(1).into()), solid(60.0), Instr::Opacity(None)];
        let plan = Composition::new(&instrs, None);
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut tip = crate::text::Texts::open(tx);
        let mut cached = DrawList::default();
        for (a, b, skip) in [(1.0, 0.0, false), (0.0, 1.0, false), (0.5, 1.0, true), (1.0, 1.0, false)] {
            let mut original = DrawList::default();
            cached.skip = if skip { vec![3..6] } else { vec![] };
            original.skip.clone_from(&cached.skip);
            let c = Ctx { props: &[], facts: &[a, b] };
            original.compose_prepared(&instrs, &reference(&instrs, None), c, &[], &mut tip, None, (100.0, 100.0), false);
            cached.compose_prepared(&instrs, &plan, c, &[], &mut tip, None, (100.0, 100.0), false);
            assert_eq!(cached.elements, original.elements);
            assert_eq!(cached.shapes, original.shapes);
            assert_eq!(cached.points, original.points);
            assert_eq!(cached.stops, original.stops);
            assert_eq!(cached.hidden, original.hidden);
            assert_eq!(cached.offscreen_groups, original.offscreen_groups);
        }
    }

    #[test]
    #[ignore = "set PLEAMAR_COMPOSE_BENCH_SCENE for an isolated structural traversal measurement"]
    fn scene_preparation_benchmark() {
        let scene = crate::read_scene(&std::env::var("PLEAMAR_COMPOSE_BENCH_SCENE").expect("a benchmark scene")).unwrap();
        let plan = Composition::new(&scene.instrs, None);
        let original = reference(&scene.instrs, None);
        assert_eq!(plan.jumps, original.jumps);
        for cached in [false, true] {
            let start = std::time::Instant::now();
            for _ in 0..3000 {
                if cached { std::hint::black_box(&plan); }
                else { std::hint::black_box(reference(&scene.instrs, None)); }
            }
            println!("composition preparation: cached={cached}, instructions={}, microseconds/round={:.3}, retained_bytes={}",
                scene.instrs.len(), start.elapsed().as_secs_f64() * 1e6 / 3000.0,
                (plan.sequence.capacity() + plan.jumps.capacity()) * std::mem::size_of::<usize>());
        }
    }
}

impl Composition {
    /// Rebuild when the scene, its diagnostic banner or its z order changes.
    pub fn new(instrs: &[Instr], order: Option<&[usize]>) -> Self {
        let mut sequence: Vec<_> = order.map_or_else(|| (0..instrs.len()).collect(), <[usize]>::to_vec);
        // The diagnostic banner follows the scene and is outside its z blocks.
        if order.is_some() { sequence.extend(sequence.len()..instrs.len()); }
        let mut jumps = vec![0; sequence.len()];
        let mut open = Vec::new();
        let mut emitters = 0;
        for (p, &idx) in sequence.iter().enumerate() {
            match &instrs[idx] {
                Instr::Opacity(Some(_)) | Instr::Fade(_) | Instr::Effect(_) => open.push((p, emitters)),
                Instr::Opacity(None) => {
                    if let Some((start, before)) = open.pop() {
                        // Hidden emitters still keep their clock, so they must be visited.
                        if before == emitters { jumps[start] = p + 1; }
                    }
                }
                Instr::Particles(_) => emitters += 1,
                _ => {}
            }
        }
        Self { sequence, jumps }
    }
}
