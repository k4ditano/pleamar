use super::read_with;
use crate::scene::{Animated, Ctx};

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
