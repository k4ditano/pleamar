//! The contract between whoever thinks and whoever paints.
//!
//! A scene is *data*: named properties, a list of drawing instructions whose
//! numbers are expressions over those properties, behaviors the render carries
//! on its own (blinking, looking, breathing) and mouse-sensitive zones. The
//! render does not know what a little ball is: it evaluates, paints and reports.
//!
//! The logic never sends values: it sends *intentions* ("this property goes to
//! 406 with this spring, in 60 ms").

#![allow(dead_code)] // the contract runs ahead of the scenes that use it

use std::ops::{Add, Div, Mul, Sub};
use std::time::Duration;

/// A scene's names live as long as the program, but each one only once:
/// reloading the same file a hundred times costs no more than the first.
pub fn intern(s: &str) -> &'static str {
    use std::collections::HashSet;
    use std::sync::Mutex;
    static TABLE: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut t = TABLE.lock().unwrap();
    let table = t.get_or_insert_with(HashSet::new);
    if let Some(existing) = table.get(s) {
        return existing;
    }
    let new: &'static str = Box::leak(s.to_owned().into_boxed_str());
    table.insert(new);
    new
}

// ── where the scene lives ───────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SurfaceAnchor {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
}

impl SurfaceAnchor {
    /// Which edges it is attached to, in the usual order: left, top, right,
    /// bottom. Against an attached edge there is no room to ask for —a top bar
    /// goes off the top because it wants to—, so that is where no warning is
    /// given that something gets cut off.
    pub fn attached_edges(&self) -> [bool; 4] {
        let (l, t, r, b) = match self {
            SurfaceAnchor::Top => (false, true, false, false),
            SurfaceAnchor::Bottom => (false, false, false, true),
            SurfaceAnchor::Left => (true, false, false, false),
            SurfaceAnchor::Right => (false, false, true, false),
            SurfaceAnchor::TopLeft => (true, true, false, false),
            SurfaceAnchor::TopRight => (false, true, true, false),
            SurfaceAnchor::BottomLeft => (true, false, false, true),
            SurfaceAnchor::BottomRight => (false, false, true, true),
            SurfaceAnchor::Center => (false, false, false, false),
        };
        [l, t, r, b]
    }
    pub fn from_word(p: &str) -> Option<SurfaceAnchor> {
        Some(match p {
            "top" => SurfaceAnchor::Top,
            "bottom" => SurfaceAnchor::Bottom,
            "left" => SurfaceAnchor::Left,
            "right" => SurfaceAnchor::Right,
            "top_left" => SurfaceAnchor::TopLeft,
            "top_right" => SurfaceAnchor::TopRight,
            "bottom_left" => SurfaceAnchor::BottomLeft,
            "bottom_right" => SurfaceAnchor::BottomRight,
            "center" => SurfaceAnchor::Center,
            _ => return None,
        })
    }
}

/// Which layer of the desktop: behind the windows or in front.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Level {
    Background,
    Below,
    Above,
    Overlay,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Screens {
    /// One surface on each monitor, and on the ones plugged in later.
    All,
    /// Only on these. A repeated name gives two surfaces on the same one.
    Named(Vec<String>),
    /// Monitor number k of the ones there are, in order. It is what `screens: each` does:
    /// one surface per monitor, each with its own state.
    Number(usize),
}

/// The surface a scene asks for. The size is in logical pixels: on a monitor
/// at scale 2 it is painted with twice as many real pixels.
#[derive(Clone, Debug)]
pub struct Surface {
    /// What it is called. The main one, "".
    pub name: String,
    /// If it comes from a `screens: each`: which of the copies it is. The logic and the drawing
    /// tell it apart by that (`screen.name`, `$screen`).
    pub instance: usize,
    /// Where its own part is drawn, inside the scene's space: each surface looks at
    /// a different piece of the same plane, like popups do. That way they all share
    /// properties, facts and rules without knowing about each other.
    pub origin: (f32, f32),
    /// It exists while this fact is true. Without it, always.
    pub open: Option<Expr>,
    /// 0 is "the whole width of the monitor".
    pub width: u32,
    pub height: u32,
    pub anchor: SurfaceAnchor,
    /// If the edge it attaches to is decided by a fact: which one, and which anchor
    /// each of its values corresponds to. Layer-shell lets it change on the
    /// fly, so something that has to pick a corner —Marea's recording face
    /// picks the one that falls outside what is being recorded— does not need four
    /// surfaces, one per corner.
    pub anchor_from: Option<(FactId, Vec<SurfaceAnchor>)>,
    /// Top, right, bottom, left.
    pub margin: [i32; 4],
    pub level: Level,
    /// If it is a normal window —the kind the compositor decorates and places— instead of
    /// a panel attached to the edge: what its title is. Without this, it is a panel.
    pub window: Option<String>,
    /// If it is the lock screen: a surface that does NOT exist until its
    /// `open:` becomes true, and that, once it exists, has the session really
    /// locked —the compositor guarantees it, not the drawing—, on all
    /// monitors at once. When `open:` stops being true, it unlocks.
    pub lock_screen: bool,
    /// How much room the compositor reserves for it: windows do not step on it.
    pub exclusive_zone: i32,
    /// How many frames per second AT MOST, decided by whoever writes the
    /// scene and not by whatever monitor it lands on. 0 is "the monitor's". A bar
    /// that breathes does not need 165 frames per second, and painting them is what
    /// costs: with this, what it consumes is the same on any screen.
    pub max_fps: u32,
    pub screens: Screens,
    pub keyboard: Keyboard,
    /// The keyboard is only requested while `Scene::keyboard_while` holds.
    pub keyboard_while: bool,
    /// As long as no rule uses the right button, it closes the program.
    pub right_click_quits: bool,
}

/// Whether the surface wants the keyboard. `OnDemand` is the normal thing in a panel with
/// something to type into; `Always` keeps it all to itself, like a launcher.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Keyboard {
    #[default]
    Never,
    OnDemand,
    Always,
}

impl Default for Surface {
    fn default() -> Self {
        // Neutral: full width, at the top, on all monitors. Whatever the scene asks for wins.
        Surface { name: String::new(), instance: 0, origin: (0.0, 0.0), open: None, window: None, lock_screen: false, width: 0, height: 40, anchor: SurfaceAnchor::Top, anchor_from: None, margin: [0; 4], level: Level::Above, exclusive_zone: 0, max_fps: 0, screens: Screens::All, keyboard: Keyboard::Never, keyboard_while: false, right_click_quits: true }
    }
}

// ── properties and expressions ──────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropId(pub u16);
/// Something the logic (or a rule) says is true. A number; yes and no are 1 and 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FactId(pub u16);
/// Something that happens at an instant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignalId(pub u16);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZoneId(pub u16);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GestureId(pub u16);
/// A text the logic can change: a fact, but made of letters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextId(pub u16);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageId(pub u16);

/// What an expression is evaluated with: what moves and what is true.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub props: &'a [Animated],
    pub facts: &'a [f32],
}

/// A pure expression over the properties. Since it has no side effects, the render
/// can evaluate it whenever and wherever it wants: it is what a binding would be.
#[derive(Clone, Debug)]
pub enum Expr {
    K(f32),
    P(PropId),
    H(FactId),
    /// The velocity of a property: the render knows it, the logic does not.
    Vel(PropId),
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    Div(Box<Expr>, Box<Expr>),
    Min(Box<Expr>, Box<Expr>),
    Max(Box<Expr>, Box<Expr>),
    Abs(Box<Expr>),
    /// Rounding down or up: needed to count rows.
    Floor(Box<Expr>),
    /// Sine and cosine, in degrees: what it takes to place something on an arc.
    Sin(Box<Expr>),
    Cos(Box<Expr>),
    Ceil(Box<Expr>),
    /// smoothstep(a, b, x)
    Smoothstep(f32, f32, Box<Expr>),
    /// `mix(a, b, t)` and also `if(c, x, y)`, which is mixing with 0 or 1. Each
    /// side appears only once: written as `a + (b - a) * t`, `a` came out twice, and
    /// a chain of `if`s doubled the tree at every link. And with `t` exactly at
    /// 0 or at 1, which is what a condition gives, only one side is evaluated.
    Mix(Box<Expr>, Box<Expr>, Box<Expr>),
    // Conditions: true is > 0.5, and they return 1 or 0.
    Gt(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
}

impl Expr {
    pub fn eval(&self, c: Ctx) -> f32 {
        use Expr::*;
        match self {
            K(v) => *v,
            P(p) => c.props[p.0 as usize].x,
            H(h) => c.facts[h.0 as usize],
            Vel(p) => c.props[p.0 as usize].v,
            Add(a, b) => a.eval(c) + b.eval(c),
            Sub(a, b) => a.eval(c) - b.eval(c),
            Mul(a, b) => a.eval(c) * b.eval(c),
            Div(a, b) => a.eval(c) / b.eval(c),
            Min(a, b) => a.eval(c).min(b.eval(c)),
            Max(a, b) => a.eval(c).max(b.eval(c)),
            Abs(a) => a.eval(c).abs(),
            Floor(a) => a.eval(c).floor(),
            Sin(a) => a.eval(c).to_radians().sin(),
            Cos(a) => a.eval(c).to_radians().cos(),
            Ceil(a) => a.eval(c).ceil(),
            Smoothstep(a, b, x) => {
                let t = ((x.eval(c) - a) / (b - a)).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            }
            Mix(a, b, t) => match t.eval(c) {
                0.0 => a.eval(c),
                1.0 => b.eval(c),
                t => {
                    let a = a.eval(c);
                    a + (b.eval(c) - a) * t
                }
            },
            Gt(a, b) => (a.eval(c) > b.eval(c)) as u8 as f32,
            And(a, b) => (a.eval(c) > 0.5 && b.eval(c) > 0.5) as u8 as f32,
            Or(a, b) => (a.eval(c) > 0.5 || b.eval(c) > 0.5) as u8 as f32,
            Not(a) => (a.eval(c) <= 0.5) as u8 as f32,
        }
    }
    pub fn is_true(&self, c: Ctx) -> bool {
        self.eval(c) > 0.5
    }
    /// How many nodes it has. A `let` is substituted where it is used, so this is
    /// what it costs each time it is named, and what decides whether it is worth
    /// computing it just once.
    /// Whether any of the facts it reads meets the condition.
    pub fn reads(&self, f: impl Fn(FactId) -> bool + Copy) -> bool {
        use Expr::*;
        match self {
            K(_) | P(_) | Vel(_) => false,
            H(h) => f(*h),
            Abs(a) | Floor(a) | Sin(a) | Cos(a) | Ceil(a) | Not(a) | Smoothstep(_, _, a) => a.reads(f),
            Add(a, b) | Sub(a, b) | Mul(a, b) | Div(a, b) | Min(a, b) | Max(a, b) | Gt(a, b) | And(a, b) | Or(a, b) => a.reads(f) || b.reads(f),
            Mix(a, b, t) => a.reads(f) || b.reads(f) || t.reads(f),
        }
    }
    pub fn node_count(&self) -> usize {
        use Expr::*;
        match self {
            K(_) | P(_) | H(_) | Vel(_) => 1,
            Abs(a) | Floor(a) | Sin(a) | Cos(a) | Ceil(a) | Not(a) | Smoothstep(_, _, a) => 1 + a.node_count(),
            Add(a, b) | Sub(a, b) | Mul(a, b) | Div(a, b) | Min(a, b) | Max(a, b) | Gt(a, b) | And(a, b) | Or(a, b) => 1 + a.node_count() + b.node_count(),
            Mix(a, b, t) => 1 + a.node_count() + b.node_count() + t.node_count(),
        }
    }
    /// What it is worth, if it depends on nothing.
    pub fn constant(&self) -> Option<f32> {
        match self {
            Expr::K(v) => Some(*v),
            _ => None,
        }
    }
    /// Whatever can be known when reading the scene is known once, not every frame:
    /// `2 * 3` is `6`, `-4` is `-4` (and not `0 - 4`), `x * 1` is `x`. The children
    /// come already folded, because it is built bottom up.
    fn folded(self) -> Expr {
        use Expr::*;
        let k = |e: &Expr| e.constant();
        let constant_children = match &self {
            K(_) | P(_) | H(_) | Vel(_) => return self,
            Abs(a) | Floor(a) | Sin(a) | Cos(a) | Ceil(a) | Not(a) | Smoothstep(_, _, a) => k(a).is_some(),
            Add(a, b) | Sub(a, b) | Mul(a, b) | Div(a, b) | Min(a, b) | Max(a, b) | Gt(a, b) | And(a, b) | Or(a, b) => k(a).is_some() && k(b).is_some(),
            Mix(a, b, t) => k(a).is_some() && k(b).is_some() && k(t).is_some(),
        };
        if constant_children {
            return K(self.eval(Ctx { props: &[], facts: &[] }));
        }
        match self {
            Add(a, b) if k(&b) == Some(0.0) => *a,
            Add(a, b) if k(&a) == Some(0.0) => *b,
            Sub(a, b) if k(&b) == Some(0.0) => *a,
            Mul(a, b) if k(&b) == Some(1.0) => *a,
            Mul(a, b) if k(&a) == Some(1.0) => *b,
            Div(a, b) if k(&b) == Some(1.0) => *a,
            Mix(a, b, t) => match k(&t) {
                Some(0.0) => *a,
                Some(1.0) => *b,
                _ => Mix(a, b, t),
            },
            other => other,
        }
    }
    pub fn mix(self, b: impl Into<Expr>, t: impl Into<Expr>) -> Expr {
        Expr::Mix(Box::new(self), Box::new(b.into()), Box::new(t.into())).folded()
    }
    pub fn gt(self, o: impl Into<Expr>) -> Expr {
        Expr::Gt(Box::new(self), Box::new(o.into())).folded()
    }
    pub fn and(self, o: impl Into<Expr>) -> Expr {
        Expr::And(Box::new(self), Box::new(o.into())).folded()
    }
    pub fn or(self, o: impl Into<Expr>) -> Expr {
        Expr::Or(Box::new(self), Box::new(o.into())).folded()
    }
    pub fn not(self) -> Expr {
        Expr::Not(Box::new(self)).folded()
    }
    pub fn min(self, o: impl Into<Expr>) -> Expr {
        Expr::Min(Box::new(self), Box::new(o.into())).folded()
    }
    pub fn max(self, o: impl Into<Expr>) -> Expr {
        Expr::Max(Box::new(self), Box::new(o.into())).folded()
    }
    pub fn clamp(self, a: f32, b: f32) -> Expr {
        self.max(a).min(b)
    }
    pub fn abs(self) -> Expr {
        Expr::Abs(Box::new(self)).folded()
    }
    pub fn sin(self) -> Expr {
        Expr::Sin(Box::new(self)).folded()
    }
    pub fn cos(self) -> Expr {
        Expr::Cos(Box::new(self)).folded()
    }
    pub fn floor(self) -> Expr {
        Expr::Floor(Box::new(self)).folded()
    }
    pub fn ceil(self) -> Expr {
        Expr::Ceil(Box::new(self)).folded()
    }
    pub fn smoothstep(self, a: f32, b: f32) -> Expr {
        Expr::Smoothstep(a, b, Box::new(self)).folded()
    }
}

/// a·(1−t) + b·t
pub fn mix(a: f32, b: f32, t: impl Into<Expr>) -> Expr {
    t.into() * (b - a) + a
}

impl PropId {
    pub fn e(self) -> Expr {
        Expr::P(self)
    }
    pub fn vel(self) -> Expr {
        Expr::Vel(self)
    }
}
impl FactId {
    pub fn e(self) -> Expr {
        Expr::H(self)
    }
}
impl From<FactId> for Expr {
    fn from(h: FactId) -> Expr {
        Expr::H(h)
    }
}
impl From<f32> for Expr {
    fn from(v: f32) -> Expr {
        Expr::K(v)
    }
}
impl From<PropId> for Expr {
    fn from(p: PropId) -> Expr {
        Expr::P(p)
    }
}

macro_rules! operator {
    ($op_trait:ident, $method:ident, $variant:ident) => {
        impl<T: Into<Expr>> $op_trait<T> for Expr {
            type Output = Expr;
            fn $method(self, o: T) -> Expr {
                Expr::$variant(Box::new(self), Box::new(o.into())).folded()
            }
        }
        impl<T: Into<Expr>> $op_trait<T> for PropId {
            type Output = Expr;
            fn $method(self, o: T) -> Expr {
                Expr::$variant(Box::new(self.into()), Box::new(o.into())).folded()
            }
        }
        impl $op_trait<Expr> for f32 {
            type Output = Expr;
            fn $method(self, o: Expr) -> Expr {
                Expr::$variant(Box::new(self.into()), Box::new(o)).folded()
            }
        }
        impl $op_trait<PropId> for f32 {
            type Output = Expr;
            fn $method(self, o: PropId) -> Expr {
                Expr::$variant(Box::new(self.into()), Box::new(o.into())).folded()
            }
        }
    };
}
operator!(Add, add, Add);
operator!(Sub, sub, Sub);
operator!(Mul, mul, Mul);
operator!(Div, div, Div);

// ── what gets painted ───────────────────────────────────────────

pub type Point = (Expr, Expr);

pub use crate::shapes::{Affine, PathStep, Shape};

#[derive(Clone, Debug)]
pub struct Shadow {
    /// Everything in it is an expression, as in the rest of the scene: a shadow
    /// that cannot change forces a choice between the halo a little ball wants
    /// and the shadow a panel wants, when they are the same body.
    pub offset: (Expr, Expr),
    pub blur: Expr,
    pub alpha: Expr,
    /// What color. Black if not said, which is what a shadow is on
    /// paper; on a desktop of dark windows, a black shadow has
    /// nothing to darken and what separates a thing from the background is a light
    /// halo. That is why it can be said, and why it is an expression: it can keep
    /// changing from halo to shadow depending on what is underneath.
    pub color: Option<Color>,
}

/// A glass: how much (`glass`, from 0 to 1) and whether it bends what is behind like a
/// lens (`lens`, true or not) or only lets it be seen blurred.
#[derive(Clone, Debug)]
pub struct Glass {
    pub amount: Expr,
    pub lens: Expr,
}

/// A vertical gradient of lightness, so the body is not flat.
#[derive(Clone, Debug)]
pub struct Light {
    pub amount: f32,
    pub from_y: Expr,
    pub height: f32,
}

pub type Color = [Expr; 3];

#[derive(Clone, Debug)]
pub enum Paint {
    Color(Color),
    /// A gradient from one point to another, or from a center outwards. The stops
    /// go in order, each with where it falls (from 0 to 1) and what color it is.
    Gradient { radial: bool, from: Point, to: Point, stops: Vec<(Expr, Color)> },
}
impl From<Color> for Paint {
    fn from(c: Color) -> Paint {
        Paint::Color(c)
    }
}

/// Rotate, scale and move everything that comes after, around a point. It
/// composes with any that were already there: the head rotates, and inside it an eye
/// can rotate on its own.
#[derive(Clone, Debug)]
pub struct Transform {
    pub pivot: Point,
    pub rotate: Expr,
    pub scale: Point,
    pub translate: Point,
}

impl Transform {
    pub fn at(pivot: Point) -> Self {
        Transform { pivot, rotate: 0.0.into(), scale: (1.0.into(), 1.0.into()), translate: (0.0.into(), 0.0.into()) }
    }
    pub fn rotate(mut self, radians: impl Into<Expr>) -> Self {
        self.rotate = radians.into();
        self
    }
    pub fn scale(mut self, x: impl Into<Expr>, y: impl Into<Expr>) -> Self {
        self.scale = (x.into(), y.into());
        self
    }
    pub fn translate(mut self, x: impl Into<Expr>, y: impl Into<Expr>) -> Self {
        self.translate = (x.into(), y.into());
        self
    }
    pub fn affine(&self, c: Ctx) -> Affine {
        Affine::new(
            (self.pivot.0.eval(c), self.pivot.1.eval(c)),
            self.rotate.eval(c),
            (self.scale.0.eval(c), self.scale.1.eval(c)),
            (self.translate.0.eval(c), self.translate.1.eval(c)),
        )
    }
}

// ── text and images ─────────────────────────────────────────────

#[derive(Clone, Debug)]
pub enum Content {
    Literal(String),
    /// A number that comes from an expression, with so many decimals and whatever goes after it: `54 %`.
    Number(Expr, u8, String),
    /// Whatever a live text is worth right now: the logic changes it.
    Live(TextId),
    /// A text with placeholders: `"{r.title} · {volume * 100} %"`. It is assembled in the render,
    /// every time any of its parts changes.
    Template(Vec<Piece>),
}

#[derive(Clone, Debug)]
pub enum Piece {
    Literal(String),
    /// A live text, as is or turned to upper or lower case.
    Live(TextId, LetterCase),
    /// An expression, with so many decimals.
    Number(Expr, u8),
    /// Some seconds, written the way a clock writes them: `1:07`, and `1:02:07`
    /// when they go past the hour.
    TimeSpan(Expr),
    /// The name of whatever an enumerated fact is worth: `{mode}` → `critical`.
    Name(Expr, Vec<String>),
    /// `{? · {r.body}}`: a span that is only there if none of its texts is empty.
    Optional(Vec<Piece>),
    /// A span that comes from outside already assembled: the text that was passed to a component.
    Span(Vec<Piece>),
    /// A text that was passed empty: it writes nothing, but counts as empty for a `{? …}`.
    Empty,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LetterCase {
    AsIs,
    Upper,
    Lower,
}

impl Piece {
    /// Writes the span. Returns whether it was whole: whether no live text came out empty.
    pub fn write_into(pieces: &[Piece], c: Ctx, texts: &[String], out: &mut String) -> bool {
        use std::fmt::Write;
        let mut whole = true;
        for p in pieces {
            match p {
                Piece::Literal(s) => out.push_str(s),
                Piece::Live(id, case) => {
                    let s = texts.get(id.0 as usize).map_or("", String::as_str);
                    whole &= !s.is_empty();
                    match case {
                        LetterCase::AsIs => out.push_str(s),
                        LetterCase::Upper => out.push_str(&s.to_uppercase()),
                        LetterCase::Lower => out.push_str(&s.to_lowercase()),
                    }
                }
                Piece::Number(e, decimals) => {
                    let _ = write!(out, "{:.*}", *decimals as usize, e.eval(c));
                }
                Piece::TimeSpan(e) => {
                    let t = e.eval(c).max(0.0) as u64;
                    let _ = if t >= 3600 {
                        write!(out, "{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60)
                    } else {
                        write!(out, "{}:{:02}", t / 60, t % 60)
                    };
                }
                Piece::Name(e, names) => {
                    if let Some(n) = names.get(e.eval(c).round().max(0.0) as usize) {
                        out.push_str(n);
                    }
                }
                Piece::Span(inner) => whole &= Piece::write_into(inner, c, texts, out),
                Piece::Empty => whole = false,
                Piece::Optional(inner) => {
                    let start = out.len();
                    if !Piece::write_into(inner, c, texts, out) {
                        out.truncate(start);
                    }
                }
            }
        }
        whole
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug)]
pub struct Style {
    /// `None` is the system sans, whatever the system is.
    pub family: Option<&'static str>,
    pub px: f32,
    /// 400 normal, 500 medium, 700 bold.
    pub weight: u16,
    pub color: Color,
    /// Line height, in multiples of the size.
    pub line_height: f32,
    pub align: TextAlign,
    /// Beyond this many lines, an ellipsis.
    pub max_lines: Option<usize>,
}

impl Style {
    pub fn new(px: f32, color: Color) -> Self {
        Style { family: None, px, weight: 400, color, line_height: 1.3, align: TextAlign::Left, max_lines: None }
    }
    pub fn weight(mut self, p: u16) -> Self {
        self.weight = p;
        self
    }
    pub fn align(mut self, a: TextAlign) -> Self {
        self.align = a;
        self
    }
    pub fn lines(mut self, n: usize) -> Self {
        self.max_lines = Some(n);
        self
    }
    pub fn family(mut self, f: &'static str) -> Self {
        self.family = Some(f);
        self
    }
}

/// Where an image comes from.
#[derive(Clone, Debug)]
pub enum ImageSource {
    File(std::path::PathBuf),
    /// An icon by its name ("firefox"). Where to find it is something the platform knows.
    Icon(String),
    /// Whatever a live text says: the name of an icon, or a path if it starts
    /// with `/`. It is how the logic chooses an image: by changing that text.
    Live(TextId),
}

pub fn color(r: f32, g: f32, b: f32) -> Color {
    [r.into(), g.into(), b.into()]
}

/// The draw list. The render turns it into elements —one quad each—
/// and paints them in order.
#[derive(Clone, Debug)]
pub enum Instr {
    /// Starts a body: from here on, shapes accumulate.
    Group { shadow: Option<Shadow> },
    /// Adds a shape to the body, merged with what it already has (`fusion` is the
    /// radius of the smooth minimum; 0 is a hard union).
    Shape { shape: Shape, fusion: Expr },
    /// Paints the accumulated body: shadow, fill, light and rim. With `glass_spec`, from
    /// 0 to 1, the fill turns into glass: translucent, and with the light on the edges.
    Fill { paint: Paint, alpha: Expr, rim: f32, light: Option<Light>, border: Option<(Expr, Color)>, glass_spec: Option<Glass> },
    /// Everything that comes after is clipped to this shape, as well as to the ones
    /// that were already there (up to four). `None` removes the last one.
    Clip(Option<(Shape, f32)>),
    /// Everything that comes after is transformed with this, as well as with whatever
    /// was already there. `None` removes the last one.
    Transform(Option<Transform>),
    /// Everything that comes after is painted apart and blended as ONE thing with
    /// this opacity: what is in front does not let what is behind show through half blended.
    /// `None` closes the group.
    Opacity(Option<Expr>),
    /// A loose shape, of a solid color.
    Solid { shape: Shape, color: Color, alpha: Expr, glass_spec: Option<Glass> },
    /// Text. `at` is the reference point and `anchor` which part of the text falls
    /// on it: (0, 0) the top left corner, (0.5, 0.5) the
    /// center. With `width` it is broken into lines; without it, it is a single one.
    /// `measure`, if given, are two properties where the render leaves how much space
    /// the text takes: with them a box can grow with its label.
    Text { content: Content, at: Point, anchor: (f32, f32), width: Option<Expr>, style: Style, alpha: Expr, measure: Option<(PropId, PropId)> },
    /// A field to type into. It edits a live text; the cursor, the selection and the
    /// echo of each key are handled by the render, without waiting for the logic. `zone` is
    /// the name of the zone that focuses it when pressed.
    /// `secret`: what is typed is shown as dots, one per letter. The real
    /// text is still the live text; what changes is what gets painted.
    Field { text: TextId, zone: &'static str, at: Point, width: Expr, style: Style, alpha: Expr, placeholder: String, selection: Color, secret: bool },
    /// An image or an icon. With `tint`, its shape is painted in that color: what
    /// a symbolic icon wants.
    Image { image: ImageId, target: (Expr, Expr, Expr, Expr), alpha: Expr, tint: Option<Color> },
}

// ── what the render does on its own ─────────────────────────────

#[derive(Clone, Debug)]
pub enum Behavior {
    /// Takes the property from 1 to 0 and back, every now and then.
    Blink { prop: PropId, every: (f32, f32), duration: f32 },
    /// prop = amplitude · sin(frequency · t). With amplitude 0 it costs nothing.
    Wave { prop: PropId, frequency: f32, amplitude: Expr },
    /// The property chases an expression with its spring. It is what in the
    /// language will be `width: label.width + 24 ~lively`.
    Follow { prop: PropId, to: Expr },
    /// prop = expression, without a spring. So that something computed at the end of the
    /// drawing —the space a stack takes— can be read from the beginning.
    Bind { prop: PropId, to: Expr },
    /// prop += per_second · dt, endlessly: a hand that goes round and round.
    Advance { prop: PropId, per_second: Expr },
    /// Two properties that pull towards the pointer, with their own spring.
    Gaze { x: PropId, y: PropId, center: Point, reach: (f32, f32), distance: f32, rest: Point },
}

#[derive(Clone, Debug)]
pub struct Transition {
    pub prop: PropId,
    /// Where to. An expression, evaluated when its time comes: `center - 220`.
    pub to: Expr,
    pub spring: Spring,
    pub delay: Duration,
}

pub fn go_to(prop: PropId, to: impl Into<Expr>, spring: Spring, delay_ms: u64) -> Transition {
    Transition { prop, to: to.into(), spring, delay: Duration::from_millis(delay_ms) }
}

/// The keys that go along with another one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Cursor {
    #[default]
    Normal,
    Hand,
    Text,
    Grab,
    Grabbing,
}

/// A sensitive region: a shape with a name. The render does the hit-test with
/// the same formula it paints with.
#[derive(Clone, Debug)]
pub struct Zone {
    pub id: &'static str,
    pub shape: Shape,
    pub active: Expr,
    /// Which cursor is set when passing over it.
    pub cursor: Cursor,
    /// The transforms it lives under, from outside in: what is seen
    /// rotated is pressed rotated.
    pub under: Vec<Transform>,
}

impl Zone {
    /// The box that contains it, to tell the compositor where the mouse comes in.
    pub fn bounds(&self, c: Ctx) -> Option<[f32; 4]> {
        let mut p = self.shape.flatten_into(c, &mut Vec::new());
        p.affine = self.under.iter().fold(Affine::IDENTITY, |a, t| a.mul(t.affine(c)));
        p.bounds()
    }

    /// A point on the screen, seen from inside: in a stack, (0, 0) is the
    /// corner of the slot. It is what a rule reads in `local.x`.
    pub fn to_local(&self, c: Ctx, x: f32, y: f32) -> (f32, f32) {
        self.under.iter().fold(Affine::IDENTITY, |a, t| a.mul(t.affine(c))).inverse().apply(x, y)
    }

    /// A path is measured against its points, as on the GPU: a zone shaped
    /// like an arrow is pressed where the arrow is seen, not in its box.
    pub fn contains(&self, c: Ctx, x: f32, y: f32) -> bool {
        let mut pts = Vec::new();
        let mut p = self.shape.flatten_into(c, &mut pts);
        p.affine = self.under.iter().fold(Affine::IDENTITY, |a, t| a.mul(t.affine(c)));
        p.distance_with(x, y, &pts) < 0.0
    }
}

// ── layers: who wins ────────────────────────────────────────────

/// When a claim holds.
#[derive(Clone, Debug)]
pub enum When {
    Always,
    While(Expr),
    /// For a while after any of these signals.
    After { signals: Vec<SignalId>, duration: Duration },
    /// From one of these signals until one of those.
    FromUntil { from: Vec<SignalId>, until: Vec<SignalId> },
}

#[derive(Clone, Debug)]
pub struct Claim {
    pub name: &'static str,
    pub when: When,
    /// What it sets when it wins: it is what sketch B called "state".
    pub sets: Vec<Transition>,
}

impl Claim {
    pub fn during(name: &'static str, c: impl Into<Expr>) -> Self {
        Claim { name, when: When::While(c.into()), sets: vec![] }
    }
    pub fn after(name: &'static str, signals: &[SignalId], ms: u64) -> Self {
        Claim { name, when: When::After { signals: signals.to_vec(), duration: Duration::from_millis(ms) }, sets: vec![] }
    }
    pub fn from_until(name: &'static str, from: &[SignalId], until: &[SignalId]) -> Self {
        Claim { name, when: When::FromUntil { from: from.to_vec(), until: until.to_vec() }, sets: vec![] }
    }
    pub fn fallback(name: &'static str) -> Self {
        Claim { name, when: When::Always, sets: vec![] }
    }
    pub fn sets(mut self, t: Vec<Transition>) -> Self {
        self.sets = t;
        self
    }
}

/// A slot that many claim. The first claim that holds wins;
/// when it stops holding, the next one shows, by itself. Nobody "turns off" anything.
#[derive(Clone, Debug)]
pub struct Layer {
    pub name: &'static str,
    pub spring: Spring,
    pub claims: Vec<Claim>,
    /// One property per claim, which goes to 1 when it wins and to 0 when not:
    /// with it one drawing is blended into another.
    pub presences: Vec<PropId>,
}

/// What `Scene::layer` returns, to paint according to who wins.
pub struct LayerRef {
    pub presences: Vec<PropId>,
}
impl LayerRef {
    pub fn presence(&self, i: usize) -> PropId {
        self.presences[i]
    }
}

// ── gestures: timelines ─────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub enum Curve {
    Linear,
    InQuad,
    OutQuad,
    InCubic,
    OutCubic,
    InOutSine,
    OutBack,
}

impl Curve {
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Curve::Linear => t,
            Curve::InQuad => t * t,
            Curve::OutQuad => 1.0 - (1.0 - t) * (1.0 - t),
            Curve::InCubic => t * t * t,
            Curve::OutCubic => 1.0 - (1.0 - t).powi(3),
            Curve::InOutSine => 0.5 - 0.5 * (std::f32::consts::PI * t).cos(),
            Curve::OutBack => {
                let (c1, u) = (1.70158, t - 1.0);
                1.0 + (c1 + 1.0) * u * u * u + c1 * u * u
            }
        }
    }
}

/// Who can interrupt whom: a gesture only cuts off another of its class or
/// lower. A reflex does not take the face away from something that was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    Ambient,
    Posture,
    Reflex,
    Asked,
    State,
}

#[derive(Clone, Debug)]
pub struct Keyframe {
    pub ms: u32,
    pub hold: u32,
    pub curve: Curve,
    /// Whatever is not named goes back to its base pose.
    /// Expressions, which are evaluated when the gesture starts: that way the same gesture
    /// can point to one side or the other depending on a fact.
    pub values: Vec<(PropId, Expr)>,
    pub emit: Option<SignalId>,
}

pub fn keyframe(ms: u32, curve: Curve) -> Keyframe {
    Keyframe { ms, hold: 0, curve, values: vec![], emit: None }
}
impl Keyframe {
    pub fn with(mut self, p: PropId, v: impl Into<Expr>) -> Self {
        self.values.push((p, v.into()));
        self
    }
    pub fn hold(mut self, ms: u32) -> Self {
        self.hold = ms;
        self
    }
    pub fn emit(mut self, s: SignalId) -> Self {
        self.emit = Some(s);
        self
    }
}

#[derive(Clone, Debug)]
pub struct Gesture {
    pub name: &'static str,
    pub class: Class,
    pub keyframes: Vec<Keyframe>,
}

// ── rules: what makes things change ─────────────────────────────

#[derive(Clone, Debug)]
pub enum Trigger {
    Enter(ZoneId),
    Leave(ZoneId),
    /// The left button.
    Press(ZoneId),
    /// Another button: 1 is the right one and 2 the middle one.
    PressWith(ZoneId, u8),
    /// What was pressed here is released, wherever the mouse is by now.
    Release(ZoneId),
    /// The wheel, over it. How much, in `wheel`: +1 per notch upwards.
    Wheel(ZoneId),
    /// The mouse moves with the button held since it was pressed here. Where, in
    /// `local.x` and `local.y`; how much since it was pressed, in `drag.dx` and `drag.dy`.
    Drag(ZoneId),
    /// It has been held pressed for this long.
    Hold { zone: ZoneId, duration: Duration },
    /// A key, by its name: `Escape`, `Return`, `a`, `Ctrl+k`.
    Key(String),
    /// Enter has been pressed in a field.
    Submit(TextId),
    /// The surface gains or loses the keyboard: losing it means someone clicked outside.
    FocusGained,
    FocusLost,
    /// Something dragged from another application has been dropped on it.
    Receive(ZoneId),
    /// The mouse has been over it for this long.
    Above { zone: ZoneId, duration: Duration },
    /// It has been over it and has been outside for this long.
    Away { zone: ZoneId, duration: Duration },
    /// Nobody has touched the mouse for this long, while the condition holds.
    Idle { duration: Duration, during: Expr },
    /// Every now and then, at random, while the condition holds.
    Every { between: (f32, f32), during: Expr },
    /// When that calculation stops being worth what it was worth. The first time does not count: it
    /// fires on changing, not on being born.
    Change(Expr),
    /// When that calculation has been worth the same for this long. It is the reverse of
    /// `Change`, and like it, being born does not count: a change is needed first, or a
    /// scene that starts still would fire by itself on opening. Each change
    /// resets the clock to zero, so a burst —the volume key
    /// pressed six times— is a single wait and not six.
    Still { value: Expr, duration: Duration },
    On(SignalId),
}

#[derive(Clone, Debug)]
pub enum Effect {
    Animate(Transition),
    /// A fact becomes worth whatever the expression is worth at that moment. It can
    /// read the mouse's things: `level = clamp(local.x / 64, 0, 1)`.
    Fact(FactId, Expr),
    /// From yes to no and from no to yes.
    Toggle(FactId),
    /// A signal, with a payload if wanted: `emit opened(i)`. It is evaluated when it fires.
    Signal(SignalId, Option<Expr>),
    /// A push to the spring's velocity. The amount is evaluated when it
    /// fires, so it can depend on what is going on: the bounce of
    /// a countdown decreases with the number that is left.
    Impulse(PropId, Expr),
    Gesture(GestureId),
    /// Put the text cursor in a field, or take it away.
    FocusField(Option<TextId>),
}

/// Everything here is executed by the render, whatever state the logic is in.
#[derive(Clone, Debug)]
pub struct Rule {
    pub when: Trigger,
    /// `on press x while open`: only if this is true at the moment it fires.
    pub guard: Option<Expr>,
    pub effects: Vec<Effect>,
}

pub fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[derive(Default)]
pub struct Scene {
    pub props: Vec<(&'static str, f32, Spring)>,
    pub instrs: Vec<Instr>,
    pub behaviors: Vec<Behavior>,
    pub zones: Vec<Zone>,
    /// Name and initial value of each live text.
    pub texts: Vec<(&'static str, String)>,
    /// Each image, and the largest logical size it is painted at.
    pub images: Vec<(ImageSource, (u32, u32))>,
    pub facts: Vec<(&'static str, f32)>,
    /// Name, and whether besides the scene it also reaches the logic.
    pub signals: Vec<(&'static str, bool)>,
    pub layers: Vec<Layer>,
    pub gestures: Vec<Gesture>,
    /// The properties that make up the pose: the ones a gesture leads by the hand.
    pub pose: Vec<PropId>,
    /// Gestures that repeat by themselves while something is true.
    pub postures: Vec<(GestureId, Expr)>,
    pub rules: Vec<Rule>,
    /// The windows it asks for. The first is the main one: the one that rules if something belongs to just one.
    pub surfaces: Vec<Surface>,
    /// Files that are not `.plm` and that it is also made of —the SVGs of
    /// its figures—: touching them also reloads it.
    pub attachments: Vec<std::path::PathBuf>,
    /// `keyboard: exclusive while open`: when it wants the keyboard.
    pub keyboard_while: Option<Expr>,
    pub permissions: Permissions,
    pub popups: Vec<Popup>,
    pub models: Vec<Model>,
    /// Of the facts that are not plain numbers, what they are. By name.
    pub types: Vec<(String, FactType)>,
    pub plugins: Vec<Plugin>,
    /// The services the scene asks for by name, and which fields it wants from each one.
    pub services: Vec<Service>,
    /// With `screens: each`, which span of instructions, behaviors and rules
    /// belongs to each copy (by the index of its surface). A copy whose
    /// surface is closed has nothing to show and nothing to move:
    /// the render skips its part entirely. Without this, two copies were twice
    /// the scene per frame even if one was not visible.
    pub spans: Vec<Span>,
}

#[derive(Clone, Debug, Default)]
pub struct Span {
    pub surface: usize,
    pub instrs: std::ops::Range<usize>,
    pub behaviors: std::ops::Range<usize>,
    pub rules: std::ops::Range<usize>,
}

/// A library with logic of its own. Its boundary —facts, texts, models, signals—
/// lives under its name (`Clock.now`), its logic runs in its own Luau state, it can only
/// name its own things, and what it touches of the system is decided by **its** permissions.
#[derive(Clone, Debug, PartialEq)]
pub struct Plugin {
    pub name: String,
    pub logic: std::path::PathBuf,
    pub permissions: Permissions,
}

/// Shaped data that crosses the boundary: a list of rows, all with the
/// same fields. The logic hands it over whole (`model.rows = list`) and the scene
/// walks through it (`for r in rows`). Inside, each field of each row is a live
/// text or a named fact —`rows.3.label`—: the render does not know there are lists.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub name: String,
    /// How many rows fit. Whatever goes beyond that is not seen, but is counted: `rows.total`.
    pub capacity: usize,
    pub fields: Vec<Field>,
}

/// A system service requested from the scene: whatever arrives fills
/// `alias.field` without anyone writing logic. `service clock as now { time: text }`.
#[derive(Clone, Debug, PartialEq)]
pub struct Service {
    /// As the platform knows it: `clock`, `audio`, `battery`, `network`, `media`.
    pub name: String,
    /// In front of each field: `now.time`.
    pub alias: String,
    pub fields: Vec<Field>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: String,
    pub kind: FieldType,
    /// What it is worth if the row does not bring it.
    pub fallback: FieldValue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FieldType {
    Text,
    Number,
    /// Like `Number`, but only 0 or 1, and the logic writes it with `true` and `false`.
    Bool,
    /// One of these names: `low | normal | critical`. Inside, its position.
    Enum(Vec<String>),
    /// The name of an icon or a path, and the image that says, at this size.
    Image(u32, u32),
    /// A list of rows inside the row: a menu with its submenus.
    List(Box<Model>),
}

/// What a fact that is not a plain number is. The scene only sees numbers; this
/// is for whoever sets them and reads them from outside —the logic, `--say`, a placeholder in a
/// text—, who talks about `true` and `critical`, not about 1 and 2.
#[derive(Clone, Debug, PartialEq)]
pub enum FactType {
    Bool,
    Enum(Vec<String>),
}

impl FactType {
    /// As it is shown to someone: `true`, `critical`.
    pub fn to_text(&self, v: f32) -> String {
        match self {
            FactType::Bool => (v > 0.5).to_string(),
            FactType::Enum(names) => names.get(v.round().max(0.0) as usize).cloned().unwrap_or_else(|| v.to_string()),
        }
    }
    /// And the other way round: what someone wrote, as a number.
    pub fn from_text(&self, t: &str) -> Option<f32> {
        match self {
            FactType::Bool => match t { "true" => Some(1.0), "false" => Some(0.0), _ => None },
            FactType::Enum(names) => names.iter().position(|n| n == t).map(|k| k as f32),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FieldValue {
    Text(String),
    Number(f32),
}

/// A surface that comes out of the main one: a menu, a card. What it paints
/// is one more piece of the scene —same springs, same zones, same rules—,
/// drawn far away, at `origin`; the popup surface is a window onto that piece.
#[derive(Clone, Debug)]
pub struct Popup {
    pub name: &'static str,
    /// It is open while this fact is true. If the system closes it
    /// (someone clicked outside), the render sets it to false.
    pub open: FactId,
    /// Where it comes out, inside the main surface; and how big it is.
    pub at: (Expr, Expr),
    pub size: (Expr, Expr),
    pub origin: (f32, f32),
}

/// What this scene's logic can touch of the system. **Undeclared,
/// nothing**: not a command, not a service. It is in the scene and not in the script
/// so it can be read at a glance, before running anything, what it will be able to do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Permissions {
    /// The commands that `run` and `spawn` can launch, by their exact name.
    pub commands: Vec<String>,
    /// The services that `sys.watch` and `sys.call` can use: `audio`, `apps`…
    pub services: Vec<String>,
}

impl Scene {
    /// The main one: the one declared without a name, or the first.
    pub fn surface(&self) -> &Surface {
        self.surfaces.first().expect("every scene has at least one surface")
    }
    pub fn surface_mut(&mut self) -> &mut Surface {
        if self.surfaces.is_empty() {
            self.surfaces.push(Surface::default());
        }
        &mut self.surfaces[0]
    }

    /// Properties go by name: if the scene is replaced by another, the
    /// ones with the same name keep their value and velocity.
    pub fn prop(&mut self, name: &'static str, initial: f32) -> PropId {
        self.prop_with(name, initial, Spring::LIVELY)
    }
    pub fn prop_with(&mut self, name: &'static str, initial: f32, spring: Spring) -> PropId {
        self.props.push((name, initial, spring));
        PropId(self.props.len() as u16 - 1)
    }
    /// The spring a property was declared with. It is its own: a rule that
    /// sends it somewhere without saying which spring to use goes with this one, which is what
    /// `prop x = 0 ~620ms` says when written.
    pub fn spring_of(&self, p: PropId) -> Spring {
        self.props[p.0 as usize].2
    }
    pub fn paint(&mut self, i: Instr) {
        self.instrs.push(i);
    }
    /// A property of the pose, with its rest value.
    pub fn pose_prop(&mut self, name: &'static str, rest: f32) -> PropId {
        let p = self.prop_with(name, rest, Spring::POSE);
        self.pose.push(p);
        p
    }
    /// Two read-only properties —width and height— that the render fills with
    /// whatever a text measures.
    pub fn measured(&mut self, name: &'static str) -> (PropId, PropId) {
        let n = |suffix: &str| intern(&format!("{name}.{suffix}"));
        (self.prop(n("width"), 0.0), self.prop(n("height"), 0.0))
    }
    pub fn live_text(&mut self, name: &'static str, initial: &str) -> TextId {
        self.texts.push((name, initial.to_owned()));
        TextId(self.texts.len() as u16 - 1)
    }
    pub fn image(&mut self, source: ImageSource, width: u32, height: u32) -> ImageId {
        self.images.push((source, (width, height)));
        ImageId(self.images.len() as u16 - 1)
    }
    pub fn fact(&mut self, name: &'static str, initial: f32) -> FactId {
        self.facts.push((name, initial));
        FactId(self.facts.len() as u16 - 1)
    }
    /// An internal signal: the layers and the rules hear it.
    pub fn signal(&mut self, name: &'static str) -> SignalId {
        self.signals.push((name, false));
        SignalId(self.signals.len() as u16 - 1)
    }
    /// A signal that also goes out to the logic.
    pub fn outgoing_signal(&mut self, name: &'static str) -> SignalId {
        self.signals.push((name, true));
        SignalId(self.signals.len() as u16 - 1)
    }
    pub fn zone(&mut self, id: &'static str, shape: Shape, active: impl Into<Expr>) -> ZoneId {
        self.zone_under(id, shape, active, vec![])
    }
    pub fn zone_under(&mut self, id: &'static str, shape: Shape, active: impl Into<Expr>, under: Vec<Transform>) -> ZoneId {
        self.zones.push(Zone { id, shape, active: active.into(), cursor: Cursor::Normal, under });
        ZoneId(self.zones.len() as u16 - 1)
    }
    /// Claims go from more to less priority; the last one should be
    /// `fallback`.
    pub fn layer(&mut self, name: &'static str, spring: Spring, claims: Vec<Claim>) -> LayerRef {
        let presences: Vec<PropId> = claims
            .iter()
            .map(|r| {
                let n = intern(&format!("layer.{name}.{}", r.name));
                self.props.push((n, 0.0, spring));
                PropId(self.props.len() as u16 - 1)
            })
            .collect();
        self.layers.push(Layer { name, spring, claims, presences: presences.clone() });
        LayerRef { presences }
    }
    pub fn gesture(&mut self, name: &'static str, class: Class, keyframes: Vec<Keyframe>) -> GestureId {
        self.gestures.push(Gesture { name, class, keyframes });
        GestureId(self.gestures.len() as u16 - 1)
    }
    pub fn posture(&mut self, gesture: GestureId, during: impl Into<Expr>) {
        self.postures.push((gesture, during.into()));
    }
    pub fn rule(&mut self, when: Trigger, effects: Vec<Effect>) {
        self.rules.push(Rule { when, guard: None, effects });
    }
}

// ── messages ────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub stiffness: f32,
    pub damping: f32,
}

impl Spring {
    pub const LIVELY: Spring = Spring { stiffness: 170.0, damping: 19.0 };
    pub const CALM: Spring = Spring { stiffness: 150.0, damping: 23.0 };
    pub const QUICK: Spring = Spring { stiffness: 420.0, damping: 40.0 };
    pub const SLOW: Spring = Spring { stiffness: 28.0, damping: 11.0 };
    pub const GENTLE: Spring = Spring { stiffness: 190.0, damping: 24.0 };
    pub const POSE: Spring = Spring { stiffness: 260.0, damping: 28.0 };

    /// The spring that gets there in that time without overshooting. Critically damped
    /// (`damping = 2 · √stiffness`), which is the one that does not bounce; with that, reaching 99 %
    /// takes about 6.64 time constants, and the stiffness comes from there.
    pub fn at(seconds: f32) -> Spring {
        let w = 6.64 / seconds.max(0.016);
        Spring { stiffness: w * w, damping: 2.0 * w }
    }
}

pub enum Command {
    Animate(Transition),
    Impulse { prop: PropId, velocity: f32 },
    /// Only in naive mode: the logic lives on the thread that paints, as in
    /// QtQuick, and its work stops the clock for everything.
    Block(Duration),
}

pub enum ToRender {
    Scene(Scene),
    /// A reload that does not go through. A runtime that reloads on save cannot
    /// report it only through a console that maybe nobody looks at: it is shown on the
    /// surface itself, which is where whoever writes it is already looking. `None`
    /// removes it: the scene is fine again.
    ReloadError(Option<String>),
    /// One more surface to paint on: a monitor that was already there or that was just
    /// plugged in.
    Sheet(Box<crate::gpu::NewSheet>),
    SheetGone(u32),
    /// The compositor has already shown that sheet's last frame and wants another.
    Frame(u32),
    /// What was seen on screen behind a glass, with it on top.
    Backdrop(Box<crate::platform::Backdrop>),
    /// The workshop has finished something: a layout, some images.
    Workshop(Box<crate::text::Parcel>),
    Scale(u32, f32),
    /// A surface has changed size: a window someone is stretching.
    SheetSize(u32, (f32, f32)),
    Command(Command),
    /// The boundary: the logic tells what happens, and nothing more.
    Fact(&'static str, f32),
    Text(&'static str, String),
    Signal(&'static str),
    /// A signal that comes from outside the program: the scene and the logic hear it.
    ExternalSignal(&'static str, Option<f32>),
    Gesture(&'static str),
    Pointer(Option<(f32, f32)>),
    /// 0 is the left one, 1 the right one, 2 the middle one.
    Button(u8, bool),
    /// Wheel notches: positive, upwards.
    Wheel(f32),
    /// The name of the key, what it types if it types anything, and what it was pressed with.
    Key(String, Option<String>, Mods),
    KeyReleased(String),
    /// The surface has gained or lost the keyboard.
    KeyboardFocus(bool),
    /// Put the text cursor in a field, or take it away from wherever it is.
    FocusField(Option<&'static str>),
    /// How this user's keys repeat: after how many ms it starts and every how many
    /// it goes on. `None`: they have no repetition. The system says so; if it says nothing, 400 and 33.
    KeyRepeat(Option<(u32, u32)>),
    /// A fact set from outside, as it was written: `true`, `critical`, `3`. The render knows what type it is.
    ExternalFact(&'static str, String),
    /// The system has closed a popup: someone clicked outside it.
    PopupClosed(usize),
    /// What the compositor says about the lock: `true`, the session IS locked
    /// (and not before); `false`, it has not granted it or has considered it over.
    LockScreen(bool),
    /// Someone from outside asks how much a fact, a text or a property is worth.
    Query(&'static str, std::sync::mpsc::Sender<String>),
    /// Something has been dropped on it, dragged from another application: (type, content).
    Dropped(String, String),
    Quit,
}

/// The texts a `secret: true` field edits. What they hold is not written
/// in any log, nor in the echo of the rehearsals: it is a password. It belongs to the whole
/// process because whoever does the echo —the logic— does not have the scene in front of it.
pub static SECRETS: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());

#[derive(Clone, Debug)]
pub enum Event {
    Enter(&'static str),
    Leave(&'static str),
    Press(&'static str),
    Release(&'static str),
    Wheel(&'static str, f32),
    Key(String, Option<String>),
    /// What a field says now, key by key.
    Text(&'static str, String),
    /// Enter in a field.
    Submit(&'static str, String),
    Focus(bool),
    /// Something dropped on a zone: (zone, type, content).
    Received(&'static str, String, String),
    Alarm(&'static str),
    /// A scene signal that goes out to the logic, with its payload if it brings one.
    Signal(&'static str, Option<f32>),
    /// A rule has changed a fact: the logic keeps track of what is true.
    Fact(&'static str, f32),
    /// A command that was launched has finished: (which one, what it wrote, with what code).
    Process(u32, String, i32),
    /// A command that is still running has written a line.
    Line(u32, String),
    /// A system service has something new to tell.
    Data(String, crate::platform::SysValue),
    /// The logic file has changed.
    ReloadLogic,
    /// The scene has been reloaded: these are now its facts and its texts.
    NewScene(Vec<(&'static str, f32)>, Vec<(&'static str, String)>, Permissions, Vec<Model>, Vec<(String, FactType)>, Vec<Plugin>, Vec<&'static str>, Vec<Service>),
    /// A layer has changed hands: (layer, who wins now).
    Layer(&'static str, &'static str),
    /// A gesture was asked for and there was one of a higher class in place.
    GestureRejected(&'static str),
    /// The mouseless mode: "do the next thing you would do".
    Demo,
}

/// An animated property: position, velocity and where it wants to go.
#[derive(Clone, Copy)]
pub struct Animated {
    pub x: f32,
    pub v: f32,
    pub target: f32,
    pub spring: Spring,
}

impl Animated {
    pub fn at(x: f32, spring: Spring) -> Self {
        Animated { x, v: 0.0, target: x, spring }
    }

    /// Semi-implicit Euler in steps of 2 ms at most: stable even if a
    /// frame arrives late.
    pub fn step(&mut self, dt: f32) {
        let steps = (dt / 0.002).ceil().max(1.0);
        let h = dt / steps;
        for _ in 0..steps as u32 {
            let a = -self.spring.stiffness * (self.x - self.target) - self.spring.damping * self.v;
            self.v += a * h;
            self.x += self.v * h;
        }
    }

    pub fn at_rest(&self) -> bool {
        (self.x - self.target).abs() < 0.02 && self.v.abs() < 0.08
    }

    pub fn settle(&mut self) {
        self.x = self.target;
        self.v = 0.0;
    }

    /// For the ones a behavior writes directly.
    pub fn set(&mut self, x: f32) {
        self.x = x;
        self.target = x;
        self.v = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn animated(props: &[f32]) -> Vec<Animated> {
        props.iter().map(|&x| Animated { x, v: 0.0, target: x, spring: Spring::LIVELY }).collect()
    }

    #[test]
    fn constants_fold_on_read() {
        assert!(matches!((Expr::K(2.0) * 3.0 + 1.0), Expr::K(7.0)));
        assert!(matches!(Expr::K(0.0) - Expr::K(4.0), Expr::K(-4.0)));
        assert!(matches!(PropId(0) * 1.0, Expr::P(_)));
        assert!(matches!(PropId(0) + 0.0, Expr::P(_)));
        assert!(matches!(Expr::K(3.0).mix(PropId(0), 0.0), Expr::K(3.0)));
        assert!(matches!(Expr::K(3.0).mix(PropId(0), 1.0), Expr::P(_)));
    }

    #[test]
    fn mix_matches_lerp() {
        let props = animated(&[0.25, 1.0, 0.0]);
        let c = Ctx { props: &props, facts: &[] };
        let (a, b) = (Expr::K(10.0), Expr::K(20.0));
        for p in 0..3 {
            let t = props[p].x;
            assert_eq!(a.clone().mix(b.clone() + PropId(2), PropId(p as u16)).eval(c), 10.0 + 10.0 * t);
        }
    }

    #[test]
    fn if_chain_grows_linearly() {
        let mut e = Expr::K(1.0);
        for k in 0..30 {
            e = e.mix(Expr::K(k as f32), PropId(0).e().gt(k as f32));
        }
        assert!(e.node_count() < 200, "{} nodes", e.node_count());
    }
}
