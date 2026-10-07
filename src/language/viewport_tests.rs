use super::read_with;
use crate::scene::{Animated, Ctx, Scene};

fn compile(body: &str) -> Scene {
    let path = std::env::current_dir().unwrap().join("viewport-test.plm");
    let source = format!("scene Viewport {{ surface {{ kind: window; size: 400, 300 }} {body} }}");
    read_with(path.to_str().unwrap(), vec![(path.clone(), source)]).unwrap().0
}

fn values(scene: &Scene) -> (Vec<Animated>, Vec<f32>) {
    (scene.props.iter().map(|(_, x, spring)| Animated::at(*x, *spring)).collect(), scene.facts.iter().map(|(_, x)| *x).collect())
}

#[test]
fn scrolling_rows_cannot_intercept_the_footer() {
    let scene = compile(r#"
        zone box footer { from: 20, 85; size: 100, 30 }
        column list {
            at: 20, 30; view: 100, 50
            box first { size: 100, 40; active: true }
            box second { size: 100, 40; active: true }
        }
    "#);
    let (mut props, facts) = values(&scene);
    let first = scene.zones.iter().find(|z| z.id == "first").unwrap();
    let second = scene.zones.iter().find(|z| z.id == "second").unwrap();
    let footer = scene.zones.iter().find(|z| z.id == "footer").unwrap();
    let c = Ctx { props: &props, facts: &facts };
    assert!(second.contains(c, 30.0, 75.0));
    assert!(!second.contains(c, 30.0, 100.0), "a clipped row intercepted the footer");
    assert!(footer.contains(c, 30.0, 100.0));
    assert_eq!(second.bounds(c), Some([20.0, 70.0, 120.0, 80.0]));
    let scroll = scene.props.iter().position(|(name, _, _)| name.starts_with("·scroll")).unwrap();
    props[scroll].x = 40.0;
    let c = Ctx { props: &props, facts: &facts };
    assert!(!first.contains(c, 30.0, 20.0), "a row scrolled above the viewport still caught input");
    assert!(first.bounds(c).is_none());
    assert!(second.contains(c, 30.0, 40.0));
    assert_eq!(second.bounds(c), Some([20.0, 30.0, 120.0, 70.0]));
}

#[test]
fn nested_viewports_follow_layout_placement_and_anchors() {
    let scene = compile(r#"
        column outer {
            at: 100, 100; anchor: center middle; view: 100, 80
            box { size: 100, 30 }
            row inner {
                view: 60, 70; corner: 10
                box child { size: 100, 70; active: true }
            }
        }
    "#);
    let (props, facts) = values(&scene);
    let c = Ctx { props: &props, facts: &facts };
    let child = scene.zones.iter().find(|z| z.id == "child").unwrap();
    assert!(child.contains(c, 70.0, 110.0));
    assert!(!child.contains(c, 130.0, 110.0), "inner viewport did not clip horizontally");
    assert!(!child.contains(c, 70.0, 150.0), "outer viewport did not clip vertically");
    assert!(!child.contains(c, 50.5, 90.5), "rounded corner still caught input");
    assert_eq!(child.bounds(c), Some([50.0, 90.0, 110.0, 140.0]));
}

#[test]
fn rotated_viewports_test_in_their_own_coordinate_space() {
    let scene = compile(r#"
        group {
            move: 200, 100; rotate: 90deg
            row list {
                view: 50, 40
                box child { size: 100, 40; active: true }
            }
        }
    "#);
    let (props, facts) = values(&scene);
    let c = Ctx { props: &props, facts: &facts };
    let child = scene.zones.iter().find(|z| z.id == "child").unwrap();
    assert!(child.contains(c, 180.0, 125.0));
    assert!(!child.contains(c, 180.0, 175.0), "rotated hidden content caught input");
}

#[test]
fn grid_model_slots_are_hidden_and_inactive_past_the_actual_count() {
    let scene = compile(r#"
        model cards max 4 { title: text }
        component Card(d: record) {
            size: 80, 40
            box face { from: 0, 0; size: 80, 40; active: true }
        }
        grid { at: 20, 30; columns: 2; gap: 10; width: 170; row: 40
            for d in cards { Card(d) }
        }
    "#);
    let (props, mut facts) = values(&scene);
    let count = scene.facts.iter().position(|(name, _)| *name == "cards.count").unwrap();
    for length in [0, 1, 4, 1, 0] {
        facts[count] = length as f32;
        let c = Ctx { props: &props, facts: &facts };
        assert_eq!(scene.zones.iter().filter(|z| z.active.eval(c) > 0.5).count(), length,
            "empty grid cells must not intercept clicks");
        let mut alpha = vec![1.0];
        let mut visible = 0;
        for instruction in &scene.instrs {
            match instruction {
                crate::scene::Instr::Opacity(Some(value)) => alpha.push(alpha.last().unwrap() * value.eval(c)),
                crate::scene::Instr::Opacity(None) => { alpha.pop(); },
                crate::scene::Instr::Fill { alpha: own, .. }
                | crate::scene::Instr::Solid { alpha: own, .. }
                    if *alpha.last().unwrap() * own.eval(c) > 0.0 => visible += 1,
                _ => (),
            }
        }
        assert_eq!(visible, length, "the grid painted unused model slots");
    }
}
