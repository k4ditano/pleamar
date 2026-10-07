use super::read_with;
use crate::scene::{Animated, Ctx};

#[test]
fn single_line_labels_reserve_their_line_before_the_font_worker_answers() {
    use crate::scene::{Behavior, Instr};
    let path = std::env::current_dir().unwrap().join("cold-labels.plm");
    let source = r#"scene Labels {
        surface { size: 300, 200 }
        fact detail = true
        column labels {
            at: 20, 20
            gap: 2
            text "Bluetooth" { size: 13; lines: 1 }
            text "Reading devices..." { size: 11.5; lines: 1; show: detail }
            box after_labels { size: 10, 10; active: true }
        }
    }"#;
    let scene = read_with(path.to_str().unwrap(), vec![(path.clone(), source.into())]).unwrap().0;
    let mut props: Vec<_> = scene.props.iter().map(|(_, x, spring)| Animated::at(*x, *spring)).collect();
    let mut facts: Vec<_> = scene.facts.iter().map(|(_, x)| *x).collect();
    let detail = scene.facts.iter().position(|(name, _)| *name == "detail").unwrap();
    let heights: Vec<_> = scene.instrs.iter().filter_map(|i| match i {
        Instr::Text { measure: Some((_, h)), .. } => Some(*h), _ => None,
    }).collect();
    assert_eq!(heights.len(), 2);
    let marker = scene.zones.iter().find(|z| z.id == "after_labels").unwrap();
    for (shown, measured, expected) in [
        (true, [0.0, 0.0], 20.0 + 16.9 + 2.0 + 14.95 + 2.0),
        (false, [0.0, 0.0], 20.0 + 16.9 + 2.0),
        (true, [24.0, 18.0], 20.0 + 24.0 + 2.0 + 18.0 + 2.0),
    ] {
        facts[detail] = if shown { 1.0 } else { 0.0 };
        for (h, value) in heights.iter().zip(measured) { props[h.0 as usize].set(value); }
        for _ in 0..scene.behaviors.len() {
            for behavior in &scene.behaviors {
                if let Behavior::Bind { prop, to } = behavior {
                    let value = to.eval(Ctx { props: &props, facts: &facts });
                    props[prop.0 as usize].set(value);
                }
            }
        }
        let bounds = marker.bounds(Ctx { props: &props, facts: &facts }).unwrap();
        assert!((bounds[1] - expected).abs() < 0.01, "labels overlap while measurements are pending: {bounds:?}, expected y={expected}");
    }
}

#[test]
fn capture_visibility_belongs_to_each_surface_and_its_monitor_copies() {
    let path = std::env::current_dir().unwrap().join("capture-visibility.plm");
    let source = r#"scene Captures {
        surface { size: 100, 100 }
        surface private { size: 80, 80; captures: hidden; screens: each max 2 }
        surface shared { size: 80, 80; captures: shown }
    }"#;
    let scene = read_with(path.to_str().unwrap(), vec![(path.clone(), source.into())]).unwrap().0;
    assert_eq!(scene.surfaces.iter().filter(|s| s.name == "private").count(), 2);
    for surface in &scene.surfaces {
        assert_eq!(surface.hidden_from_captures, surface.name == "private");
    }
}

#[test]
fn component_size_and_list_content_share_each_copys_actual_measure() {
    use crate::scene::{Behavior, Instr};
    let path = std::env::current_dir().unwrap().join("tests/component-own-measure.plm");
    let scene = read_with(path.to_str().unwrap(), vec![(path.clone(), include_str!("../../tests/component-own-measure.plm").into())]).unwrap().0;
    let mut props: Vec<_> = scene.props.iter().map(|(_, x, spring)| Animated::at(*x, *spring)).collect();
    let facts: Vec<_> = scene.facts.iter().map(|(_, x)| *x).collect();
    let measures: Vec<_> = scene.instrs.iter().filter_map(|i| match i {
        Instr::Text { measure: Some((_, h)), .. } => Some(*h), _ => None,
    }).collect();
    assert_eq!(measures.len(), 2);
    assert_ne!(measures[0], measures[1]);
    for (h0, h1) in [(20.0, 40.0), (70.0, 10.0)] {
        props[measures[0].0 as usize].x = h0;
        props[measures[1].0 as usize].x = h1;
        for _ in 0..scene.behaviors.len() {
            for behavior in &scene.behaviors {
                if let Behavior::Bind { prop, to } = behavior {
                    let value = to.eval(Ctx { props: &props, facts: &facts });
                    props[prop.0 as usize].x = value;
                }
            }
        }
        let c = Ctx { props: &props, facts: &facts };
        let content = scene.props.iter().position(|p| p.0 == "list.content").unwrap();
        assert_eq!(props[content].x, h0 + h1 + 24.0);
        assert_eq!(scene.zones[0].bounds(c), Some([0.0, 0.0, 100.0, h0 + 12.0]));
        assert_eq!(scene.zones[1].bounds(c), Some([0.0, h0 + 12.0, 100.0, h0 + h1 + 24.0]));
    }
}

#[test]
fn repeated_surfaces_keep_independent_measured_geometry() {
    let path = std::env::current_dir().unwrap().join("surface-sizes.plm");
    let source = r#"scene Sizes {
        surface { size: 820, 680; screens: each max 2 }
        surface tide {
            size: full, full; screens: each max 2
            let w = tide.width
            let h = tide.height
            box extent { from: 0, 0; size: w, h; active: true }
        }
        surface panel {
            size: 260, 120; screens: each max 2
            box panel_extent { from: 0, 0; size: panel.width, panel.height; active: true }
        }
    }"#;
    let scene = read_with(path.to_str().unwrap(), vec![(path.clone(), source.into())]).unwrap().0;
    let tides: Vec<_> = scene.surfaces.iter().filter(|s| s.name == "tide").collect();
    assert_ne!(tides[0].size_props, tides[1].size_props, "the second monitor overwrites the first one's size");
    let mut props: Vec<_> = scene.props.iter().map(|(_, x, spring)| Animated::at(*x, *spring)).collect();
    let facts: Vec<_> = scene.facts.iter().map(|(_, x)| *x).collect();
    // 1440p and 1080p at 125%, mixed DPI, then a portrait monitor after resize.
    for sizes in [[(2048.0, 1152.0), (1536.0, 864.0)], [(2560.0, 1440.0), (1536.0, 864.0)], [(2048.0, 1152.0), (864.0, 1536.0)]] {
        for surface in &scene.surfaces {
            let Some((w, h)) = surface.size_props else { continue };
            let size = if surface.name == "tide" { sizes[surface.instance] } else { (260.0, 120.0) };
            props[w.0 as usize].x = size.0;
            props[h.0 as usize].x = size.1;
        }
        let c = Ctx { props: &props, facts: &facts };
        for (k, tide) in tides.iter().enumerate() {
            let zone = scene.zones.iter().find(|z| z.id == format!("extent#screen{k}")).unwrap();
            assert_eq!(zone.bounds(c), Some([tide.origin.0, tide.origin.1, tide.origin.0 + sizes[k].0, tide.origin.1 + sizes[k].1]));
        }
        let first = scene.props.iter().position(|p| p.0 == "tide.width").unwrap();
        assert_eq!(props[first].x, sizes[0].0, "the unsuffixed size remains the first copy's");
    }
}
