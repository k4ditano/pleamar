//! An SVG that comes in as **geometry**, not as a stamp.
//!
//! An image is rasterised into an atlas: it shows, but it is a seal. It melts
//! into nothing, it is not tinted in parts, it is not animated by layers and when
//! scaled it is pixels. An accessory —a hat resting on her, some glasses stuck to
//! her face— needs exactly the opposite, so from an `.svg` its paths are taken out
//! and become the same signed distances as everything else: they melt with
//! `blend`, carry rim and shadow, and each layer turns on its pivot.
//!
//! The reader is `usvg`, which is already in for the images, so this brings in
//! not a single new dependency. What it does bring is a limit: of an SVG,
//! **filled and stroked paths** are understood, with their colour. Gradients,
//! masks, filters and text are not, and that is said out loud instead of faking them.

use crate::scene::{PathStep, Point, Shape};

/// A path of the SVG, already in scene geometry.
pub struct Stroke {
    pub shape: Shape,
    pub color: [f32; 3],
    pub alpha: f32,
    /// 0 if it is filled; otherwise, the thickness of its stroke.
    pub thickness: f32,
}

/// A layer: what in the editor is a group with a name.
pub struct Layer {
    pub name: String,
    pub strokes: Vec<Stroke>,
}

pub struct Figure {
    pub layers: Vec<Layer>,
    /// What its canvas measures, to be able to say "paint it at 44 × 34".
    pub size: (f32, f32),
}

impl Figure {
    pub fn layer(&self, name: &str) -> Option<&Layer> {
        self.layers.iter().find(|c| c.name == name)
    }
    pub fn names(&self) -> Vec<&str> {
        self.layers.iter().map(|c| c.name.as_str()).collect()
    }
}

/// What it costs to paint a path is its list of points, and the list comes from
/// splitting its curves. It is counted here to be able to say "that layer has 300
/// points" on load, and not find out with half a piece drawn.
fn point_count(origin: &Point, steps: &[PathStep]) -> usize {
    let val = |e: &crate::scene::Expr| match e {
        crate::scene::Expr::K(k) => *k,
        _ => 0.0,
    };
    let mut d = (val(&origin.0), val(&origin.1));
    let mut n = 1;
    for p in steps {
        match p {
            PathStep::Line(a) => {
                d = (val(&a.0), val(&a.1));
                n += 1;
            }
            PathStep::Curve { via, to } => {
                let v = (val(&via.0), val(&via.1));
                let f = (val(&to.0), val(&to.1));
                let length = (v.0 - d.0).hypot(v.1 - d.1) + (f.0 - v.0).hypot(f.1 - v.1);
                n += ((length / 4.0) as usize).clamp(3, 24);
                d = f;
            }
        }
    }
    n
}

fn point(x: f32, y: f32, center: (f32, f32)) -> Point {
    ((x - center.0).into(), (y - center.1).into())
}

/// An SVG cubic as two quadratics, which is what the scene knows how to draw.
/// Splitting it in half and approximating each half leaves the error well below
/// a pixel at the size an accessory lives at.
fn cubic(p0: (f32, f32), c1: (f32, f32), c2: (f32, f32), p3: (f32, f32)) -> [((f32, f32), (f32, f32)); 2] {
    let mid = |a: (f32, f32), b: (f32, f32)| ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
    let (a, b, c) = (mid(p0, c1), mid(c1, c2), mid(c2, p3));
    let (d, e) = (mid(a, b), mid(b, c));
    let m = mid(d, e);
    // The control point of a quadratic that resembles a cubic: (3(b+c) − (a+d))/4.
    let ctrl = |p0: (f32, f32), c1: (f32, f32), c2: (f32, f32), p3: (f32, f32)| {
        ((3.0 * (c1.0 + c2.0) - (p0.0 + p3.0)) * 0.25, (3.0 * (c1.1 + c2.1) - (p0.1 + p3.1)) * 0.25)
    };
    [(ctrl(p0, a, d, m), m), (ctrl(m, e, c, p3), p3)]
}

/// From a `tiny_skia` path to scene paths: one per loose stroke, because here
/// a `path` starts in one place and only in one.
fn paths(data: &resvg::tiny_skia::Path, center: (f32, f32)) -> Vec<Shape> {
    use resvg::tiny_skia::PathSegment as S;
    let mut out = Vec::new();
    let (mut origin, mut steps, mut closed) = (None, Vec::new(), false);
    let mut here = (0.0f32, 0.0f32);
    let close = |origin: &mut Option<Point>, steps: &mut Vec<PathStep>, closed: &mut bool, out: &mut Vec<Shape>| {
        if let (Some(o), false) = (origin.take(), steps.is_empty()) {
            out.push(Shape::Path { origin: o, steps: std::mem::take(steps), closed: *closed });
        }
        steps.clear();
        *closed = false;
    };
    for s in data.segments() {
        match s {
            S::MoveTo(p) => {
                close(&mut origin, &mut steps, &mut closed, &mut out);
                here = (p.x, p.y);
                origin = Some(point(p.x, p.y, center));
            }
            S::LineTo(p) => {
                here = (p.x, p.y);
                steps.push(PathStep::Line(point(p.x, p.y, center)));
            }
            S::QuadTo(v, p) => {
                here = (p.x, p.y);
                steps.push(PathStep::Curve { via: point(v.x, v.y, center), to: point(p.x, p.y, center) });
            }
            S::CubicTo(c1, c2, p) => {
                for (v, a) in cubic(here, (c1.x, c1.y), (c2.x, c2.y), (p.x, p.y)) {
                    steps.push(PathStep::Curve { via: point(v.0, v.1, center), to: point(a.0, a.1, center) });
                }
                here = (p.x, p.y);
            }
            S::Close => closed = true,
        }
    }
    close(&mut origin, &mut steps, &mut closed, &mut out);
    out
}

fn color_of(paint: &resvg::usvg::Paint) -> Option<[f32; 3]> {
    match paint {
        resvg::usvg::Paint::Color(c) => Some([c.red as f32 / 255.0, c.green as f32 / 255.0, c.blue as f32 / 255.0]),
        // A gradient or a pattern has no single colour: let whoever uses it say so.
        _ => None,
    }
}

fn walk(group: &resvg::usvg::Group, layer: Option<&str>, center: (f32, f32), layers: &mut Vec<Layer>, warnings: &mut Vec<String>) {
    for child in group.children() {
        match child {
            resvg::usvg::Node::Group(g) => {
                // The name of a layer is the group's `id`. Inside an already
                // named layer, the groups below belong to it: what is drawn in
                // the editor inside a layer is that layer's.
                let mine = layer.map(str::to_owned).or_else(|| (!g.id().is_empty()).then(|| g.id().to_owned()));
                walk(g, mine.as_deref(), center, layers, warnings);
            }
            resvg::usvg::Node::Path(p) => {
                let name = layer
                    .map(str::to_owned)
                    .or_else(|| (!p.id().is_empty()).then(|| p.id().to_owned()))
                    .unwrap_or_else(|| format!("layer{}", layers.len() + 1));
                let shapes = paths(p.data(), center);
                let mut strokes = Vec::new();
                if let Some(f) = p.fill() {
                    match color_of(f.paint()) {
                        Some(c) => strokes.extend(shapes.iter().map(|shape| Stroke { shape: shape.clone(), color: c, alpha: f.opacity().get(), thickness: 0.0 })),
                        None => warnings.push(format!("'{name}' is filled with a gradient or a pattern, which does not come across; give it a `color:` where you draw it")),
                    }
                }
                if let Some(t) = p.stroke() {
                    if let Some(c) = color_of(t.paint()) {
                        strokes.extend(shapes.iter().map(|shape| Stroke { shape: shape.clone(), color: c, alpha: t.opacity().get(), thickness: t.width().get() }));
                    }
                }
                if strokes.is_empty() {
                    continue;
                }
                match layers.iter_mut().find(|c| c.name == name) {
                    Some(c) => c.strokes.extend(strokes),
                    None => layers.push(Layer { name, strokes }),
                }
            }
            // An image or a text inside the SVG are not geometry: they are reported.
            resvg::usvg::Node::Image(_) => warnings.push("there is an image inside that svg, and an image is not geometry: it is left out".into()),
            resvg::usvg::Node::Text(_) => warnings.push("there is text inside that svg: convert it to paths in the editor, or it is left out".into()),
        }
    }
}

/// Reads an SVG and returns its layers, plus whatever has to be said out loud.
pub fn read(data: &[u8]) -> Result<(Figure, Vec<String>), String> {
    let tree = resvg::usvg::Tree::from_data(data, &resvg::usvg::Options::default()).map_err(|e| e.to_string())?;
    let size = (tree.size().width(), tree.size().height());
    let mut layers = Vec::new();
    let mut warnings = Vec::new();
    walk(tree.root(), None, (size.0 * 0.5, size.1 * 0.5), &mut layers, &mut warnings);
    if layers.is_empty() {
        return Err("that svg has no paths in it".into());
    }
    // What it costs to paint it is known now, not halfway through drawing.
    for c in &layers {
        for t in &c.strokes {
            if let Shape::Path { origin, steps, .. } = &t.shape {
                let n = point_count(origin, steps);
                if n > crate::shapes::MAX_POINTS {
                    return Err(format!(
                        "the layer '{}' needs {} points and the most a path takes is {}: simplify it in the editor (Path › Simplify), or split it into layers",
                        c.name,
                        n,
                        crate::shapes::MAX_POINTS
                    ));
                }
            }
        }
    }
    Ok((Figure { layers, size }, warnings))
}
