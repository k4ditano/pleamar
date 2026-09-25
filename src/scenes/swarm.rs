//! A load bench: many loose shapes, all moving, to measure what
//! painting costs when the scene stops being a little ball.

use crate::scene::*;
use crate::logic::{Context, Script};

#[derive(Default)]
pub struct Swarm;

impl Script for Swarm {
    fn scene(&mut self) -> Scene {
        let n: usize = std::env::var("PLEAMAR_N").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
        let mut e = Scene::default();
        // Test bench: it has its own size. `--screen` says which one it opens on.
        e.surfaces = vec![Surface { width: 720, height: 600, margin: [40, 0, 0, 0], ..Default::default() }];
        let pulse = e.prop("pulse", 0.0);
        let columns = ((n as f32 * 3.2).sqrt().ceil() as usize).max(1);
        let rows = n.div_ceil(columns);
        let (step_x, step_y) = (700.0 / columns as f32, 210.0 / rows as f32);
        let r = (step_x.min(step_y) * 0.42).min(14.0);
        for k in 0..n {
            let (cx, cy) = (10.0 + step_x * ((k % columns) as f32 + 0.5), 8.0 + step_y * ((k / columns) as f32 + 0.5));
            let phase = if k % 2 == 0 { 1.0 } else { -1.0 };
            let shape = if k % 3 == 0 {
                Shape::Rect { center: (cx.into(), cy.into()), half_size: (r + pulse * phase, (r * 0.7).into()), radius: (r * 0.3).into() }
            } else {
                Shape::circle((cx.into(), cy.into()), r + pulse * phase)
            };
            let t = k as f32 / n as f32;
            e.paint(Instr::Solid { shape, color: color(0.35 + 0.5 * t, 0.85 - 0.3 * t, 0.75), alpha: 0.9.into(), glass_spec: None });
        }
        e.behaviors = vec![Behavior::Wave { prop: pulse, frequency: 3.0, amplitude: 2.0.into() }];
        println!("swarm · {n} shapes");
        e
    }
    fn on_event(&mut self, _: Event, _: &mut Context) {}
}
