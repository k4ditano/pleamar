//! Marea's face, on its own, to try out what her `ExpressionController` asked for:
//! a `shape` layer with claims by priority, keyframed gestures with a
//! class, and facts and signals as the only boundary with the logic.
//!
//! The script does what the real code is most afraid of: searching and
//! confirming WHILE recording. The red disc has to hold, and when recording
//! stops the magnifying glass has to come out by itself, without anyone asking for it again.

use crate::scene::*;
use crate::logic::{Context, Script};

const CX: f32 = 360.0;
const CY: f32 = 108.0;
/// The real little ball measures 56 px; here one and a half times that, so it can be seen from afar.
const S: f32 = 1.5;
const R: f32 = 28.0 * S;

#[derive(Default)]
pub struct Face {
    step: usize,
}

const SCRIPT: &[(u64, &str)] = &[
    (1000, "recording = yes"),
    (1500, "searching = yes          · the lens claims it, but rec is above"),
    (1500, "signal confirmed + happy · happy is claimed, and it is not seen either"),
    (1800, "recording = no           · nobody claims the lens again: it has to come out by itself"),
    (1500, "signal confirmed         · happy for 620 ms, then back to the lens"),
    (1500, "searching = no           · eyes"),
    (1200, "signal urgent_alert      · 700 ms"),
    (1500, "working = yes            · a posture: it repeats by itself"),
    (1500, "gesture nod (reflex)     · cuts the posture, which is of a lower class"),
    (1200, "gesture happy (asked)"),
    (250, "gesture nod (reflex)     · refused: something asked for is running"),
    (1800, "working = no"),
];

impl Script for Face {
    fn scene(&mut self) -> Scene {
        let mut e = Scene::default();
        // Test bench: it has its own size. `--screen` says which one it opens on.
        e.surfaces = vec![Surface { width: 720, height: 260, margin: [40, 0, 0, 0], ..Default::default() }];

        // ── the boundary ─────────────────────────────────────────
        let recording = e.fact("recording", 0.0);
        let searching = e.fact("searching", 0.0);
        let working = e.fact("working", 0.0);
        let confirmed = e.signal("confirmed");
        let urgent_alert = e.signal("urgent_alert");

        // ── who has the face ─────────────────────────────────────
        //  From more to less. Stopping searching cannot erase the red disc:
        //  nobody erases anything, they just stop claiming.
        let shape = e.layer("shape", Spring::QUICK, vec![
            Claim::during("rec", recording),
            Claim::after("alert", &[urgent_alert], 700),
            Claim::after("happy", &[confirmed], 620),
            Claim::during("lens", searching),
            Claim::fallback("eyes"),
        ]);
        let (p_rec, p_alert, p_happy, p_lens, p_eyes) =
            (shape.presence(0), shape.presence(1), shape.presence(2), shape.presence(3), shape.presence(4));

        // ── the pose, in Marea's units ───────────────────────────
        let eyes = e.pose_prop("eyes", 14.0);
        let width = e.pose_prop("width", 6.0);
        let gap = e.pose_prop("gap", 16.0);
        let turn = e.pose_prop("turn", 0.0);
        let sx = e.pose_prop("sx", 1.0);
        let sy = e.pose_prop("sy", 1.0);
        let rise = e.pose_prop("rise", 0.0);
        let look_x = e.pose_prop("look.x", 0.0);
        let look_y = e.pose_prop("look.y", 0.0);

        let eyelid = e.prop("eyelid", 1.0);
        let pointer_x = e.prop_with("pointer.x", 0.0, Spring::GENTLE);
        let pointer_y = e.prop_with("pointer.y", 0.0, Spring::GENTLE);

        // ── the drawing ──────────────────────────────────────────
        let cy = CY + rise * S;
        let body = Shape::Ellipse { center: (CX.into(), cy.clone()), radius: R.into(), scale: (sx.e(), sy.e()) };
        e.paint(Instr::Group { shadow: Some(Shadow { offset: (0.0.into(), 12.0.into()), blur: 34.0.into(), alpha: 0.34.into(), color: None }) });
        e.paint(Instr::Shape { shape: body.clone(), fusion: 0.0.into() });
        e.paint(Instr::Fill { paint: color(0.082, 0.086, 0.086).into(),
            alpha: 1.0.into(),
            rim: 0.05,
            light: Some(Light { amount: 0.03, from_y: (CY - R).into(), height: R * 2.0 }),
            border: None,
            glass_spec: None,
        });
        e.paint(Instr::Clip(Some((body, 3.0))));

        // The whole face turns with the head, around the center of the little ball.
        e.paint(Instr::Transform(Some(Transform::at((CX.into(), cy.clone())).rotate(turn * (std::f32::consts::PI / 180.0)))));
        let look = (look_x * S + pointer_x, look_y * S + pointer_y);
        let eye_x = |side: f32| CX + look.0.clone() + gap * (0.5 * S * side);
        let eye_y = |_side: f32| cy.clone() + look.1.clone();
        //  A shape is only seen with more than half presence. Since the presences
        //  of a layer add up to one, there are never two at once: one leaves and the
        //  other comes in, instead of blending into a blur.
        let shows = |p: Expr| p.smoothstep(0.5, 1.0);
        let white = color(0.96, 0.97, 0.96);

        // eyes · the usual two pills. While recording, the left one stays.
        let pill = |side: f32| Shape::Rect {
            center: (eye_x(side), eye_y(side)),
            half_size: (width * (0.5 * S), (eyes * (0.5 * S) * eyelid).max(1.6)),
            radius: width * (0.5 * S),
        };
        e.paint(Instr::Solid { shape: pill(-1.0), color: white.clone(), alpha: shows(p_eyes + p_rec), glass_spec: None });
        e.paint(Instr::Solid { shape: pill(1.0), color: white.clone(), alpha: shows(p_eyes.e()), glass_spec: None });

        // rec · the right eye is the red disc, which grows as it comes in.
        e.paint(Instr::Solid {
            shape: Shape::circle((eye_x(1.0), eye_y(1.0)), p_rec * (6.5 * S)),
            color: color(0.93, 0.23, 0.2),
            alpha: shows(p_rec.e()), glass_spec: None,
        });

        // happy · two real arcs.
        for side in [-1.0, 1.0] {
            e.paint(Instr::Solid {
                shape: Shape::Arc { center: (eye_x(side), eye_y(side) + 2.5 * S), radius: (4.6 * S).into(), opening: 1.2.into(), thickness: (2.7 * S).into() },
                color: white.clone(),
                alpha: shows(p_happy.e()), glass_spec: None,
            });
        }

        // alert · an amber exclamation mark.
        let amber = color(0.98, 0.76, 0.32);
        e.paint(Instr::Solid {
            shape: Shape::Rect { center: (CX.into(), cy.clone() - 4.0 * S), half_size: ((2.3 * S).into(), (7.0 * S).into()), radius: (2.3 * S).into() },
            color: amber.clone(),
            alpha: shows(p_alert.e()), glass_spec: None,
        });
        e.paint(Instr::Solid { shape: Shape::circle((CX.into(), cy.clone() + 9.0 * S), 2.7 * S), color: amber, alpha: shows(p_alert.e()), glass_spec: None });

        // lens · a ring and its handle.
        let lx = CX + look.0.clone() - 2.5 * S;
        let ly = cy.clone() + look.1.clone() - 2.0 * S;
        e.paint(Instr::Solid { shape: Shape::ring((lx.clone(), ly.clone()), 6.2 * S, 2.6 * S), color: white.clone(), alpha: shows(p_lens.e()), glass_spec: None });
        e.paint(Instr::Solid {
            shape: Shape::Segment { from: (lx.clone() + 5.4 * S, ly.clone() + 5.4 * S), to: (lx + 9.6 * S, ly + 9.6 * S), thickness: (2.9 * S).into() },
            color: white.clone(),
            alpha: shows(p_lens.e()), glass_spec: None,
        });
        e.paint(Instr::Transform(None));
        e.paint(Instr::Clip(None));

        e.behaviors = vec![
            Behavior::Blink { prop: eyelid, every: (2.5, 7.0), duration: 0.17 },
            Behavior::Gaze {
                x: pointer_x,
                y: pointer_y,
                center: (CX.into(), CY.into()),
                reach: (5.0 * S, 3.2 * S),
                distance: 160.0,
                rest: (0.0.into(), 0.0.into()),
            },
        ];

        // ── the gestures: Marea's keyframes, as they are ─────────
        use Curve::*;
        e.gesture("nod", Class::Reflex, vec![
            keyframe(130, OutQuad).with(look_y, 4.0).with(eyes, 10.0),
            keyframe(170, OutBack).with(eyes, 15.0),
            keyframe(160, InOutSine),
        ]);
        let jump = |f: Keyframe| f.with(eyes, 5.0).with(width, 8.0);
        e.gesture("happy", Class::Asked, vec![
            // It crouches, pushes, floats, falls, and one smaller bounce.
            jump(keyframe(130, InQuad)).with(sx, 1.12).with(sy, 0.78).with(rise, 5.0),
            jump(keyframe(90, OutQuad)).with(sx, 0.90).with(sy, 1.12).with(rise, -3.0),
            jump(keyframe(150, OutQuad)).with(sx, 0.99).with(sy, 1.03).with(rise, -9.0).with(turn, -5.0).hold(35),
            jump(keyframe(140, InQuad)).with(sx, 0.94).with(sy, 1.08).with(rise, -1.0).with(turn, 2.0),
            jump(keyframe(105, OutQuad)).with(sx, 1.12).with(sy, 0.80).with(rise, 5.0),
            jump(keyframe(150, OutQuad)).with(sx, 0.98).with(sy, 1.04).with(rise, -4.0).with(turn, 3.0),
            jump(keyframe(130, InQuad)).with(sx, 1.04).with(sy, 0.94).with(rise, 1.0),
            keyframe(200, InOutSine),
        ]);
        let posture = e.gesture("working", Class::Posture, vec![
            keyframe(320, InOutSine).with(eyes, 9.0).with(width, 7.0).with(turn, -5.0).with(look_x, -2.0).hold(60),
            keyframe(360, InOutSine).with(eyes, 9.0).with(width, 7.0).with(turn, 5.0).with(look_x, 2.0).hold(60),
            keyframe(260, InOutSine).with(eyes, 11.0),
        ]);
        e.posture(posture, working);
        e
    }

    fn on_event(&mut self, e: Event, c: &mut Context) {
        match e {
            Event::Alarm("start") => c.alarm("step", SCRIPT[0].0),
            Event::Alarm("step") => {
                let (_, text) = SCRIPT[self.step];
                println!("script · {text}");
                match self.step {
                    0 => c.fact("recording", true),
                    1 => c.fact("searching", true),
                    2 => {
                        c.signal("confirmed");
                        c.gesture("happy");
                        //  And here the logic gets stuck. The face does not even notice.
                        c.work();
                    }
                    3 => c.fact("recording", false),
                    4 => c.signal("confirmed"),
                    5 => c.fact("searching", false),
                    6 => c.signal("urgent_alert"),
                    7 => c.fact("working", true),
                    8 | 10 => c.gesture("nod"),
                    9 => c.gesture("happy"),
                    _ => c.fact("working", false),
                }
                self.step += 1;
                if let Some((wait, _)) = SCRIPT.get(self.step) {
                    c.alarm("step", *wait);
                }
            }
            Event::GestureRejected(g) => println!("logic  · the scene does not grant '{g}'"),
            _ => {}
        }
    }
}
