//! Marea: the little ball, its eyes and the notification card that grows out of its side.
//! Everything here is data and decisions; not a single pixel.

use crate::scene::*;
use crate::logic::{Context, Script};

const REST_X: f32 = 360.0;
const OPEN_X: f32 = 140.0;
const ORB_Y: f32 = 90.0;
const R: f32 = 28.0;
const GAP: f32 = 34.0;
const PANEL_W: f32 = 406.0;
const PANEL_H: f32 = 190.0;
const ANCHOR_Y: f32 = 70.0;

#[derive(Default)]
pub struct Marea {
    open: bool,
    count: usize,
}

impl Script for Marea {
    fn scene(&mut self) -> Scene {
        let mut e = Scene::default();
        // Test bench: it has its own size. `--screen` says which one it opens on.
        e.surfaces = vec![Surface { width: 720, height: 224, margin: [40, 0, 0, 0], ..Default::default() }];
        let orb_x = e.prop("orb.x", REST_X);
        let orb_y = e.prop("orb.y", ORB_Y);
        let panel_w = e.prop("panel.width", 0.0);
        let panel_h = e.prop("panel.height", 0.0);
        let fusion = e.prop("fusion", 0.0);
        let content = e.prop("content", 0.0);
        let sleep = e.prop("sleep", 0.0);
        let button = e.prop("button", 0.0);
        let eyelid = e.prop("eyelid", 1.0);
        let breath = e.prop("breath", 0.0);
        let look_x = e.prop_with("gaze.x", 0.0, Spring::GENTLE);
        let look_y = e.prop_with("gaze.y", 0.0, Spring::GENTLE);

        // Stretching with velocity: an expression, not code.
        let stretch = ((orb_x.vel().abs() - orb_y.vel().abs()) / 2400.0).clamp(-0.22, 0.22);
        let orb = Shape::Ellipse {
            center: (orb_x.e(), orb_y.e()),
            radius: R + breath,
            scale: (1.0 + stretch.clone(), 1.0 / (1.0 + stretch)),
        };

        let panel_x = orb_x + (R + GAP);
        let panel_y = orb_y - panel_h * (ANCHOR_Y / PANEL_H);
        let panel = Shape::Rect {
            center: (panel_x.clone() + panel_w * 0.5, panel_y + panel_h * 0.5),
            half_size: (panel_w * 0.5, panel_h * 0.5),
            radius: 24.0.into(),
        };

        // The body: ball and card merged by a neck of water.
        e.paint(Instr::Group { shadow: Some(Shadow { offset: (0.0.into(), 10.0.into()), blur: 30.0.into(), alpha: 0.34.into(), color: None }) });
        e.paint(Instr::Shape { shape: orb.clone(), fusion: 0.0.into() });
        e.paint(Instr::Shape { shape: panel.clone(), fusion: fusion * panel_w.e().smoothstep(0.0, 40.0) });
        e.paint(Instr::Fill { paint: color(0.082, 0.086, 0.086).into(),
            alpha: 1.0.into(),
            rim: 0.05,
            light: Some(Light { amount: 0.03, from_y: orb_y - ANCHOR_Y, height: PANEL_H }),
            border: None,
            glass_spec: None,
        });

        // What is inside the card, anchored to where it will end up and clipped to
        // however much it has grown.
        let origin_y = orb_y - ANCHOR_Y;
        let visible = content.e().clamp(0.0, 1.0);
        let box_at = |cx: f32, cy: f32, mx: Expr, my: Expr| Shape::Rect {
            center: (panel_x.clone() + cx, origin_y.clone() + cy),
            half_size: (mx, my),
            radius: 9.0.into(),
        };
        e.paint(Instr::Clip(Some((panel, 1.0))));
        //  The content blends as a single thing: half appeared, the button
        //  does not show through its own label.
        e.paint(Instr::Opacity(Some(visible)));
        let visible: Expr = 1.0.into();
        e.paint(Instr::Solid {
            shape: box_at(110.0, 151.0, 84.0.into(), 23.0.into()),
            color: color(0.18, 0.184, 0.184),
            alpha: visible.clone(), glass_spec: None,
        });
        let view = box_at(296.0, 151.0, 84.0 + button * 2.0, 23.0 + button * 1.5);
        e.paint(Instr::Solid {
            shape: view.clone(),
            color: [mix(0.62, 0.74, button), mix(0.84, 0.93, button), mix(0.74, 0.84, button)],
            alpha: visible.clone(), glass_spec: None,
        });
        // The text is text: the logic changes it when another notification arrives, and it
        // is painted sharp at each monitor's scale.
        let white = color(0.96, 0.97, 0.96);
        let app = e.live_text("notice.app", "CALENDAR");
        let when = e.live_text("notice.when", "Now");
        let title = e.live_text("notice.title", "Meeting in 5 min");
        let detail = e.live_text("notice.detail", "Design review · 18:00");
        let write = |e: &mut Scene, c: Content, x: f32, y: f32, anchor: (f32, f32), width: Option<f32>, style: Style, alpha: f32| {
            e.paint(Instr::Text { content: c, at: (panel_x.clone() + x, origin_y.clone() + y), anchor, width: width.map(Into::into), style, alpha: alpha.into(), measure: None });
        };
        write(&mut e, Content::Live(app), 26.0, 33.0, (0.0, 0.5), None, Style::new(12.5, white.clone()), 0.62);
        write(&mut e, Content::Live(when), 380.0, 33.0, (1.0, 0.5), None, Style::new(13.5, white.clone()), 0.55);
        // A single line: if the title does not fit, an ellipsis.
        write(&mut e, Content::Live(title), 26.0, 71.0, (0.0, 0.5), Some(354.0), Style::new(20.0, white.clone()).weight(500).lines(1), 1.0);
        write(&mut e, Content::Live(detail), 26.0, 99.0, (0.0, 0.5), Some(354.0), Style::new(14.5, white.clone()).lines(1), 0.6);
        write(&mut e, Content::Literal("Dismiss".into()), 110.0, 151.0, (0.5, 0.5), None, Style::new(15.0, white.clone()).weight(500), 0.95);
        write(&mut e, Content::Literal("View event".into()), 296.0, 151.0, (0.5, 0.5), None, Style::new(15.0, color(0.07, 0.12, 0.10)).weight(500), 1.0);
        e.paint(Instr::Opacity(None));

        // The eyes: two pills clipped to the face.
        let opened = (eyelid * (1.0 - sleep)).max(0.14);
        let eye = |side: f32| Shape::Rect {
            center: (orb_x + look_x + side, orb_y + look_y + sleep * 3.0 - 1.0),
            half_size: (3.5.into(), 8.4 * opened.clone()),
            radius: 3.5.into(),
        };
        e.paint(Instr::Clip(None));
        e.paint(Instr::Clip(Some((orb.clone(), 3.0))));
        for side in [-8.6, 8.6] {
            e.paint(Instr::Solid { shape: eye(side), color: color(0.96, 0.97, 0.96), alpha: 1.0.into(), glass_spec: None });
        }
        e.paint(Instr::Clip(None));

        e.behaviors = vec![
            Behavior::Blink { prop: eyelid, every: (2.4, 6.0), duration: 0.17 },
            Behavior::Wave { prop: breath, frequency: 1.7, amplitude: sleep * 1.3 },
            Behavior::Gaze {
                x: look_x,
                y: look_y,
                center: (orb_x.e(), orb_y.e()),
                reach: (5.0, 3.2),
                distance: 140.0,
                // Without a mouse, it looks at the card if there is one.
                rest: (panel_w * (3.2 / PANEL_W), panel_w * (0.6 / PANEL_W)),
            },
        ];

        // ── the boundary, the layers and the rules ───────────────
        let open = e.fact("open", 0.0);
        let asleep = e.fact("asleep", 0.0);
        let view_event = e.outgoing_signal("view-event");

        //  Opening and closing are two claims of the same layer, each with
        //  its own choreography. Whoever sets `open` to yes —a mouse rule or the
        //  logic because a notification has arrived— knows nothing about springs.
        e.layer("card", Spring::CALM, vec![
            Claim::during("open", open).sets(vec![
                go_to(fusion, 96.0, Spring::QUICK, 0),
                go_to(orb_x, OPEN_X, Spring::LIVELY, 0),
                go_to(panel_w, PANEL_W, Spring::CALM, 70),
                go_to(panel_h, PANEL_H, Spring::CALM, 110),
                go_to(content, 1.0, Spring::CALM, 300),
                go_to(fusion, 0.0, Spring::CALM, 560),
            ]),
            Claim::fallback("rest").sets(vec![
                go_to(button, 0.0, Spring::QUICK, 0),
                go_to(content, 0.0, Spring::QUICK, 0),
                go_to(fusion, 96.0, Spring::QUICK, 0),
                go_to(panel_h, 0.0, Spring::CALM, 90),
                go_to(panel_w, 0.0, Spring::CALM, 90),
                go_to(orb_x, REST_X, Spring::LIVELY, 150),
                go_to(fusion, 0.0, Spring::CALM, 700),
            ]),
        ]);
        e.layer("sleep", Spring::SLOW, vec![
            Claim::during("asleep", asleep.e().and(open.e().not())).sets(vec![go_to(sleep, 1.0, Spring::SLOW, 0)]),
            Claim::fallback("awake").sets(vec![go_to(sleep, 0.0, Spring::LIVELY, 0)]),
        ]);

        let whole = e.zone(
            "whole",
            Shape::Rect {
                center: ((OPEN_X + (GAP + PANEL_W) * 0.5).into(), (ORB_Y - ANCHOR_Y + PANEL_H * 0.5).into()),
                half_size: (((R * 2.0 + GAP + PANEL_W) * 0.5 + 18.0).into(), (PANEL_H * 0.5 + 18.0).into()),
                radius: 24.0.into(),
            },
            //  Only while there is a card. Closed, that zone is empty, and the
            //  click has to go through to whatever is underneath.
            open.e().or(panel_w.e().gt(1.0)),
        );
        let z_orb = e.zone("orb", Shape::circle((orb_x.e(), orb_y.e()), R + 8.0), 1.0);
        let z_dismiss = e.zone("dismiss", box_at(110.0, 151.0, 84.0.into(), 23.0.into()), content);
        let z_view = e.zone("view", view, content);

        //  All this is executed by the render. With the logic dead, Marea opens,
        //  closes, falls asleep and highlights its button all the same.
        use Trigger::*;
        e.rule(Enter(z_orb), vec![Effect::Fact(asleep, 0.0.into())]);
        e.rule(Above { zone: z_orb, duration: ms(320) }, vec![Effect::Fact(open, 1.0.into())]);
        e.rule(Press(z_orb), vec![Effect::Toggle(open)]);
        e.rule(Away { zone: whole, duration: ms(420) }, vec![Effect::Fact(open, 0.0.into())]);
        e.rule(Enter(z_view), vec![Effect::Animate(go_to(button, 1.0, Spring::QUICK, 0))]);
        e.rule(Leave(z_view), vec![Effect::Animate(go_to(button, 0.0, Spring::QUICK, 0))]);
        e.rule(Press(z_dismiss), vec![Effect::Fact(open, 0.0.into())]);
        e.rule(Press(z_view), vec![Effect::Fact(open, 0.0.into()), Effect::Impulse(orb_y, (-620.0).into()), Effect::Signal(view_event, None)]);
        e.rule(Idle { duration: ms(14_000), during: open.e().not() }, vec![Effect::Fact(asleep, 1.0.into())]);

        e
    }

    fn on_event(&mut self, e: Event, c: &mut Context) {
        match e {
            //  The only thing left for the logic: to find out and do ITS work
            //  —put together the content, open the calendar—, which may take a while.
            Event::Layer("card", _) => c.work(),
            Event::Signal("view-event", _) => println!("logic  · someone wants to see the event"),
            //  Without a mouse, the demo plays a notification that arrives and leaves.
            Event::Demo => {
                self.open = !self.open;
                if self.open {
                    //  Each time a different notification arrives: the logic changes the text
                    //  and nothing more. The third one does not fit, on purpose.
                    const NOTICES: [[&str; 3]; 3] = [
                        ["CALENDAR", "Meeting in 5 min", "Design review · 18:00"],
                        ["MESSAGES", "Lucía: coming down for a coffee? ☕", "A moment ago"],
                        ["UPDATES", "There are 214 packages waiting for someone to make up their mind to install them", "yay · 1.2 GB"],
                    ];
                    let a = NOTICES[self.count % NOTICES.len()];
                    self.count += 1;
                    c.text("notice.app", a[0]);
                    c.text("notice.title", a[1]);
                    c.text("notice.detail", a[2]);
                }
                c.fact("open", self.open);
            }
            _ => {}
        }
    }
}
