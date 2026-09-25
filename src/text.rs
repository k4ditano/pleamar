//! Text and images: everything that ends up being a piece of the atlas.
//!
//! Text is shaped with `cosmic-text` —ligatures, right-to-left
//! languages, emoji, fallback fonts— and each glyph is painted once, at the
//! scale of the finest sheet, into an atlas it shares with the images. The
//! three pieces (`cosmic-text`, `image`, `resvg`) are pure Rust and exist on
//! Linux, Windows and macOS; the only thing that depends on the system is finding an
//! icon by its name, and the platform answers that.

use crate::scene::{ToRender, TextAlign, Style, ImageSource};
use std::collections::HashSet;
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use cosmic_text::{Align, Attrs, Buffer, CacheKey, Ellipsize, EllipsizeHeightLimit, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Weight, Wrap};
use std::collections::HashMap;

pub const ATLAS_SIZE: u32 = 2048;

/// A piece of the atlas, in pixels.
#[derive(Clone, Copy, Debug)]
pub struct AtlasSlot {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl AtlasSlot {
    pub fn uv(&self) -> [f32; 4] {
        let l = ATLAS_SIZE as f32;
        [self.x as f32 / l, self.y as f32 / l, (self.x + self.width) as f32 / l, (self.y + self.height) as f32 / l]
    }
}

/// A glyph already placed, relative to the corner of its text and in logical pixels.
#[derive(Clone, Copy)]
pub struct PlacedGlyph {
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    /// An emoji brings its own color; a letter is a mask that gets tinted.
    pub colored: bool,
}

pub struct Layout {
    pub glyphs: Vec<PlacedGlyph>,
    pub size: (f32, f32),
    /// Where the text cursor falls before each letter: (byte, x). Only for the
    /// first line: it is what a field to type into needs.
    pub cursors: Vec<(usize, f32)>,
}

impl Layout {
    /// The x of the cursor in front of byte `b`.
    pub fn x_at_byte(&self, b: usize) -> f32 {
        self.cursors.iter().rev().find(|(k, _)| *k <= b).or(self.cursors.first()).map_or(0.0, |c| c.1)
    }

    /// The byte closest to an x: where a click falls.
    pub fn byte_at_x(&self, x: f32) -> usize {
        self.cursors.iter().min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs())).map_or(0, |c| c.0)
    }
}

/// Everything that decides how a text turns out. Two texts with the same key are
/// the same layout, and it is enough to make it: it is what travels to the workshop.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct LayoutKey {
    text: String,
    family: Option<&'static str>,
    px: u32,
    weight: u16,
    line_height: u32,
    width: Option<u32>,
    align: TextAlign,
    max_lines: Option<usize>,
}

impl LayoutKey {
    pub fn new(text: &str, e: &Style, width: Option<f32>) -> LayoutKey {
        LayoutKey {
            text: text.to_owned(), family: e.family, px: e.px.to_bits(), weight: e.weight, line_height: e.line_height.to_bits(),
            width: width.map(|a| a.round().max(1.0) as u32), align: e.align, max_lines: e.max_lines,
        }
    }
}

/// Divides the atlas into shelves: rows as tall as the first thing that fell into them.
struct Shelves {
    rows: Vec<(u32, u32, u32)>, // y, height, how far it is filled
    next_y: u32,
}

impl Shelves {
    fn new() -> Self {
        Shelves { rows: Vec::new(), next_y: 1 }
    }

    fn request(&mut self, width: u32, height: u32) -> Option<AtlasSlot> {
        let (w, h) = (width + 1, height + 1); // one pixel of air so filtering does not bleed
        let row = self.rows.iter_mut().find(|(_, fh, fx)| *fh >= h && *fh <= h + h / 3 + 2 && fx + w <= ATLAS_SIZE);
        let (y, x) = match row {
            Some((y, _, fx)) => {
                let x = *fx;
                *fx += w;
                (*y, x)
            }
            None => {
                if self.next_y + h > ATLAS_SIZE || w > ATLAS_SIZE {
                    return None;
                }
                let y = self.next_y;
                self.next_y += h;
                self.rows.push((y, h, 1 + w));
                (y, 1)
            }
        };
        Some(AtlasSlot { x, y, width, height })
    }
}

/// Whoever shapes and paints. It lives on its own thread —the workshop—: reading the system
/// fonts, shaping a paragraph or decoding a PNG can take a while, and
/// none of that may cost the render a frame.
struct Typesetter {
    fonts: FontSystem,
    swash: SwashCache,
    shelves: Shelves,
    glyphs: HashMap<CacheKey, Option<(AtlasSlot, i32, i32, bool)>>,
    /// What has been painted since the last delivery: where and what (premultiplied RGBA).
    pending_upload: Vec<(AtlasSlot, Vec<u8>)>,
    /// How many real pixels per logical pixel it is painted at: that of the finest sheet.
    scale: f32,
}

impl Typesetter {
    fn new() -> Self {
        let t0 = std::time::Instant::now();
        let fonts = FontSystem::new();
        println!("text   · {} system fonts in {} ms", fonts.db().len(), t0.elapsed().as_millis());
        Typesetter { fonts, swash: SwashCache::new(), shelves: Shelves::new(), glyphs: HashMap::new(), pending_upload: Vec::new(), scale: 1.0 }
    }

    /// When the scale changes, everything painted stops being valid.
    fn clear(&mut self, scale: f32) {
        self.scale = scale;
        self.shelves = Shelves::new();
        self.glyphs.clear();
        self.pending_upload.clear();
    }

    fn lay_out(&mut self, c: &LayoutKey) -> Layout {
        let (px, line_height, width) = (f32::from_bits(c.px), f32::from_bits(c.line_height), c.width.map(|a| a as f32));
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(px, px * line_height));
        buffer.set_wrap(if width.is_some() { Wrap::WordOrGlyph } else { Wrap::None });
        if let Some(n) = c.max_lines {
            buffer.set_ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(n)));
        }
        buffer.set_size(width, None);
        let attrs = Attrs::new().family(c.family.map_or(Family::SansSerif, Family::Name)).weight(Weight(c.weight));
        let align = match c.align {
            TextAlign::Left => Align::Left,
            TextAlign::Center => Align::Center,
            TextAlign::Right => Align::Right,
        };
        buffer.set_text(&c.text, &attrs, Shaping::Advanced, Some(align));
        buffer.shape_until_scroll(&mut self.fonts, false);

        let s = self.scale;
        let mut glyphs = Vec::new();
        let (mut w, mut h) = (0f32, 0f32);
        let mut full = false;
        let mut cursors: Vec<(usize, f32)> = Vec::new();
        for run in buffer.layout_runs() {
            if run.line_i == 0 && cursors.is_empty() {
                cursors = run.glyphs.iter().map(|g| (g.start, g.x)).collect();
                cursors.push((c.text.len(), run.line_w));
                cursors.sort_by_key(|k| k.0);
            }
            w = w.max(run.line_w);
            h = h.max(run.line_top + run.line_height);
            for g in run.glyphs {
                let f = g.physical((0.0, 0.0), s);
                let Some((slot, left, top, colored)) = self.glyph(f.cache_key, &mut full) else { continue };
                let x = (f.x + left) as f32;
                let y = ((run.line_y * s).round() as i32 + f.y - top) as f32;
                glyphs.push(PlacedGlyph { rect: [x / s, y / s, slot.width as f32 / s, slot.height as f32 / s], uv: slot.uv(), colored });
            }
        }
        if full {
            eprintln!("text   · the {ATLAS_SIZE}² atlas is full: some glyphs are not painted");
        }
        // Without a fixed width, the alignment is relative to whatever the text itself measures.
        if cursors.is_empty() {
            cursors.push((0, 0.0));
        }
        Layout { glyphs, size: (width.unwrap_or(w), h), cursors }
    }

    fn glyph(&mut self, key: CacheKey, full: &mut bool) -> Option<(AtlasSlot, i32, i32, bool)> {
        if let Some(g) = self.glyphs.get(&key) {
            return *g;
        }
        let painted = self.swash.get_image_uncached(&mut self.fonts, key).and_then(|img| {
            let (w, h) = (img.placement.width, img.placement.height);
            if w == 0 || h == 0 {
                return None;
            }
            let rgba: Vec<u8> = match img.content {
                // A mask: premultiplied white, which the shader will tint.
                SwashContent::Mask => img.data.iter().flat_map(|a| [*a, *a, *a, *a]).collect(),
                SwashContent::Color => img.data.chunks_exact(4).flat_map(|p| {
                    let a = p[3] as u16;
                    [(p[0] as u16 * a / 255) as u8, (p[1] as u16 * a / 255) as u8, (p[2] as u16 * a / 255) as u8, p[3]]
                }).collect(),
                SwashContent::SubpixelMask => return None,
            };
            let Some(slot) = self.shelves.request(w, h) else {
                *full = true;
                return None;
            };
            self.pending_upload.push((slot, rgba));
            Some((slot, img.placement.left, img.placement.top, matches!(img.content, SwashContent::Color)))
        });
        self.glyphs.insert(key, painted);
        painted
    }

    /// A scene's images, at the logical size they ask for times the scale. An
    /// SVG is painted at that exact size; a PNG is scaled down if it is too big.
    fn load_images(&mut self, images: &[(ImageSource, (u32, u32))]) -> Vec<Option<AtlasSlot>> {
        images
            .iter()
            .map(|(source, (w, h))| {
                let px = (((*w as f32) * self.scale).round().max(1.0) as u32, ((*h as f32) * self.scale).round().max(1.0) as u32);
                let path = match source {
                    ImageSource::File(r) => Some(r.clone()),
                    ImageSource::Icon(name) => crate::platform::icon(name),
                    // It is requested once it is known what the text says: `live_image`.
                    ImageSource::Live(_) => return None,
                };
                let slot = path.as_ref().and_then(|r| rasterize_image(r, px)).and_then(|rgba| {
                    let slot = self.shelves.request(px.0, px.1)?;
                    self.pending_upload.push((slot, rgba));
                    Some(slot)
                });
                if slot.is_none() {
                    eprintln!("image  · cannot load {source:?}");
                }
                slot
            })
            .collect()
    }

    /// An image some data has asked for: an icon by its name, or a path.
    fn load_live(&mut self, name: &str, (w, h): (u32, u32)) -> Option<AtlasSlot> {
        let px = (((w as f32) * self.scale).round().max(1.0) as u32, ((h as f32) * self.scale).round().max(1.0) as u32);
        let path = if name.starts_with('/') { Some(std::path::PathBuf::from(name)) } else { crate::platform::icon(name) };
        let rgba = rasterize_image(&path?, px)?;
        let slot = self.shelves.request(px.0, px.1)?;
        self.pending_upload.push((slot, rgba));
        Some(slot)
    }
}

// ── the workshop and its counter ────────────────────────────────

enum Job {
    Layout(LayoutKey, u32),
    Images(Vec<(ImageSource, (u32, u32))>, u32),
    Live(String, (u32, u32), u32),
    /// New atlas, at this scale. Everything from previous generations is thrown away.
    Clear(f32),
}

/// What the workshop hands back, with whatever it painted along the way.
pub enum Delivery {
    Layout { key: LayoutKey, layout: Arc<Layout>, generation: u32 },
    Images { slots: Vec<Option<AtlasSlot>>, generation: u32 },
    Live { name: String, size: (u32, u32), slot: Option<AtlasSlot>, generation: u32 },
}

pub struct Parcel {
    pub delivery: Delivery,
    pub pending_upload: Vec<(AtlasSlot, Vec<u8>)>,
}

/// The render's side: it asks, does not wait, and meanwhile shows what it had.
pub struct Texts {
    to_workshop: Sender<Job>,
    layouts: HashMap<LayoutKey, Arc<Layout>>,
    requested: HashSet<LayoutKey>,
    /// The last layout shown at each place in the scene: it is what is
    /// seen while the new one arrives, instead of a gap.
    last: HashMap<usize, Arc<Layout>>,
    images: Vec<Option<AtlasSlot>>,
    /// Where each image of the scene comes from, to know which ones depend on a text.
    sources: Vec<(ImageSource, (u32, u32))>,
    /// The ones the data have asked for, by name and size. `None`: it was looked for and does not exist.
    live: HashMap<(String, (u32, u32)), Option<AtlasSlot>>,
    live_requested: HashSet<(String, (u32, u32))>,
    /// The last thing each live image showed, while the new one arrives.
    last_live: HashMap<usize, AtlasSlot>,
    pub pending_upload: Vec<(AtlasSlot, Vec<u8>)>,
    generation: u32,
    scale: f32,
}

impl Texts {
    /// Starts the workshop. The first thing it does is read the system fonts,
    /// so it is best to call it as early as possible.
    pub fn open(to_render: Sender<ToRender>) -> Texts {
        let (to_workshop, jobs) = channel::<Job>();
        std::thread::Builder::new()
            .name("workshop".into())
            .spawn(move || {
                let mut t = Typesetter::new();
                for job in jobs {
                    let delivery = match job {
                        Job::Clear(scale) => {
                            t.clear(scale);
                            continue;
                        }
                        Job::Layout(key, generation) => {
                            let layout = Arc::new(t.lay_out(&key));
                            Delivery::Layout { key, layout, generation }
                        }
                        Job::Images(list, generation) => Delivery::Images { slots: t.load_images(&list), generation },
                        Job::Live(name, size, generation) => {
                            let slot = t.load_live(&name, size);
                            Delivery::Live { name, size, slot, generation }
                        }
                    };
                    let pending_upload = std::mem::take(&mut t.pending_upload);
                    // Through the usual channel, which also wakes the render up if it was asleep.
                    if to_render.send(ToRender::Workshop(Box::new(Parcel { delivery, pending_upload }))).is_err() {
                        return;
                    }
                }
            })
            .unwrap();
        Texts { to_workshop, layouts: HashMap::new(), requested: HashSet::new(), last: HashMap::new(), images: Vec::new(), sources: Vec::new(), live: HashMap::new(), live_requested: HashSet::new(), last_live: HashMap::new(), pending_upload: Vec::new(), generation: 0, scale: 1.0 }
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// New scene or new scale: new atlas. What came before stops being valid, and
    /// with it whatever was being shown.
    pub fn reset(&mut self, scale: f32, images: &[(ImageSource, (u32, u32))]) {
        self.generation += 1;
        self.scale = scale;
        self.layouts.clear();
        self.requested.clear();
        self.last.clear();
        self.images.clear();
        self.sources = images.to_vec();
        self.live.clear();
        self.live_requested.clear();
        self.last_live.clear();
        self.pending_upload.clear();
        let _ = self.to_workshop.send(Job::Clear(scale));
        if !images.is_empty() {
            let _ = self.to_workshop.send(Job::Images(images.to_vec(), self.generation));
        }
    }

    pub fn receive(&mut self, p: Parcel) {
        let generation = match &p.delivery {
            Delivery::Layout { generation, .. } | Delivery::Images { generation, .. } | Delivery::Live { generation, .. } => *generation,
        };
        if generation != self.generation {
            return; // from an atlas that no longer exists
        }
        self.pending_upload.extend(p.pending_upload);
        match p.delivery {
            Delivery::Layout { key, layout, .. } => {
                self.requested.remove(&key);
                if self.layouts.len() > 512 {
                    self.layouts.clear();
                }
                self.layouts.insert(key, layout);
            }
            Delivery::Images { slots, .. } => self.images = slots,
            Delivery::Live { name, size, slot, .. } => {
                self.live_requested.remove(&(name.clone(), size));
                self.live.insert((name, size), slot);
            }
        }
    }

    /// A text's layout, if it is ready; if not, it is ordered and the last one
    /// seen at that place is returned. `place` is the position in the draw list.
    pub fn layout(&mut self, place: usize, key: LayoutKey) -> Option<Arc<Layout>> {
        if let Some(m) = self.layouts.get(&key) {
            self.last.insert(place, m.clone());
            return Some(m.clone());
        }
        if self.requested.insert(key.clone()) {
            let _ = self.to_workshop.send(Job::Layout(key, self.generation));
        }
        self.last.get(&place).cloned()
    }

    /// Image number `k` of the scene. If it comes from a live text, the one that
    /// text says now; while it arrives, the one that was there. An empty text is none.
    pub fn image(&mut self, k: usize, texts: &[String]) -> Option<AtlasSlot> {
        let Some((ImageSource::Live(t), size)) = self.sources.get(k) else {
            return self.images.get(k).copied().flatten();
        };
        let name = texts.get(t.0 as usize).filter(|n| !n.is_empty())?;
        let key = (name.clone(), *size);
        match self.live.get(&key) {
            Some(Some(slot)) => {
                self.last_live.insert(k, *slot);
                Some(*slot)
            }
            Some(None) => None,
            None => {
                if self.live_requested.insert(key.clone()) {
                    let _ = self.to_workshop.send(Job::Live(key.0, key.1, self.generation));
                }
                self.last_live.get(&k).copied()
            }
        }
    }
}

/// Premultiplied RGBA, at exactly `px`, fitted without distorting.
fn rasterize_image(path: &std::path::Path, px: (u32, u32)) -> Option<Vec<u8>> {
    let data = std::fs::read(path).ok()?;
    let is_svg = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"));
    if is_svg {
        let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
        let mut canvas = resvg::tiny_skia::Pixmap::new(px.0, px.1)?;
        let t = tree.size();
        let k = (px.0 as f32 / t.width()).min(px.1 as f32 / t.height());
        let (dx, dy) = ((px.0 as f32 - t.width() * k) * 0.5, (px.1 as f32 - t.height() * k) * 0.5);
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(k, k).post_translate(dx, dy), &mut canvas.as_mut());
        return Some(canvas.take()); // tiny-skia already premultiplies
    }
    let img = image::load_from_memory(&data).ok()?.to_rgba8();
    let k = (px.0 as f32 / img.width() as f32).min(px.1 as f32 / img.height() as f32);
    let (w, h) = (((img.width() as f32 * k).round() as u32).clamp(1, px.0), ((img.height() as f32 * k).round() as u32).clamp(1, px.1));
    let reduced = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle);
    let mut img = image::RgbaImage::new(px.0, px.1);
    image::imageops::overlay(&mut img, &reduced, ((px.0 - w) / 2) as i64, ((px.1 - h) / 2) as i64);
    Some(img.pixels().flat_map(|p| {
        let a = p[3] as u16;
        [(p[0] as u16 * a / 255) as u8, (p[1] as u16 * a / 255) as u8, (p[2] as u16 * a / 255) as u8, p[3]]
    }).collect())
}
