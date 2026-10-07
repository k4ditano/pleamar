//! Bounded image previews for desktop libraries, without a shell or ImageMagick.
use super::SysValue;
use image::{DynamicImage, ImageDecoder, ImageReader};
use std::{collections::hash_map::DefaultHasher, fs, hash::{Hash, Hasher}, io::{Cursor, Read}, path::{Path, PathBuf}, sync::Mutex};

const INPUT_LIMIT: u64 = 32 * 1024 * 1024;
const CACHE_LIMIT: u64 = 64 * 1024 * 1024;
// All callers share the decoder budget, including separate scenes/async workers.
static DECODER: Mutex<()> = Mutex::new(());

pub fn query(args: &[SysValue]) -> Result<SysValue, String> {
    let [SysValue::Text(source), SysValue::Num(width), SysValue::Num(height), SysValue::Text(mode)] = args else {
        return Err("images.thumbnail takes an absolute path, width, height and crop/fit".into());
    };
    if source.contains('\0') || !Path::new(source).is_absolute() {
        return Err("images.thumbnail needs an absolute image path".into());
    }
    let valid = |v: f64| v.is_finite() && (1.0..=1024.0).contains(&v) && v.fract() == 0.0;
    if !valid(*width) || !valid(*height) || !matches!(mode.as_str(), "crop" | "fit") {
        return Err("images.thumbnail size must be 1..1024 and mode crop or fit".into());
    }
    let base = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?;
    let _budget = DECODER.lock().map_err(|_| "image decoder failed")?;
    thumbnail(Path::new(source), *width as u32, *height as u32, mode,
        &PathBuf::from(base).join("pleamar/image-previews"))
        .map(|path| SysValue::Text(path.to_string_lossy().into_owned()))
}

fn thumbnail(source: &Path, width: u32, height: u32, mode: &str, cache: &Path) -> Result<PathBuf, String> {
    let mut file = fs::File::open(source).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > INPUT_LIMIT { return Err("image is not a file below 32 MiB".into()); }
    // Include bytes rather than just a basename or timestamp: two libraries can
    // hold different images with the same name, or replace a file in place.
    let mut bytes = Vec::new();
    (&mut file).take(INPUT_LIMIT + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > INPUT_LIMIT { return Err("image exceeds 32 MiB".into()); }
    let mut hash = DefaultHasher::new();
    ("v1", &bytes, width, height, mode).hash(&mut hash);
    fs::create_dir_all(cache).map_err(|e| e.to_string())?;
    let output = cache.join(format!("{:016x}.jpg", hash.finish()));
    if output.is_file() {
        if let Ok(file) = fs::OpenOptions::new().write(true).open(&output) {
            let _ = file.set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::now()));
        }
        return Ok(output);
    }
    let decoded = decode(&bytes, width, height, mode)?;
    let mut encoded = Vec::new();
    // Enrichment accepts at most half a MiB. Reduce quality if a detailed image
    // at the largest requested size would exceed the worker's real limit.
    for quality in [85, 72, 55, 35] {
        encoded.clear();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, quality)
            .encode_image(&decoded).map_err(|e| e.to_string())?;
        if encoded.len() <= 512 * 1024 { break; }
    }
    if encoded.len() > 512 * 1024 { return Err("preview exceeds 512 KiB".into()); }
    // Each process owns its partial file; the completed cache key is shared.
    let partial = output.with_extension(format!("{}.partial", std::process::id()));
    fs::write(&partial, encoded).map_err(|e| e.to_string())?;
    if let Err(error) = fs::rename(&partial, &output) {
        let _ = fs::remove_file(&partial);
        if !output.is_file() { return Err(error.to_string()); }
    }
    trim(cache, &output);
    Ok(output)
}

fn decode(bytes: &[u8], width: u32, height: u32, mode: &str) -> Result<image::RgbImage, String> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    let image = if reader.format().is_some() {
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(16384);
        limits.max_image_height = Some(16384);
        limits.max_alloc = Some(128 * 1024 * 1024);
        reader.limits(limits);
        let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
        if decoder.total_bytes() > 96 * 1024 * 1024 { return Err("decoded image exceeds 96 MiB".into()); }
        let orientation = decoder.orientation().map_err(|e| e.to_string())?;
        let mut image = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
        image.apply_orientation(orientation);
        image
    } else {
        // Rasterize self-contained vectors at preview size, not their claimed
        // size. Nested images would bypass the raster decoder's memory limit.
        if bytes.len() > 2 * 1024 * 1024 { return Err("SVG exceeds 2 MiB".into()); }
        let xml = roxmltree::Document::parse(std::str::from_utf8(bytes).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        if xml.descendants().any(|node| matches!(node.tag_name().name(), "image" | "feImage" | "text")) {
            return Err("SVG previews require self-contained vectors without images or text".into());
        }
        let mut options = resvg::usvg::Options::default();
        options.image_href_resolver.resolve_string = Box::new(|_, _| None);
        options.image_href_resolver.resolve_data = Box::new(|_, _, _| None);
        let tree = resvg::usvg::Tree::from_data(bytes, &options).map_err(|e| e.to_string())?;
        let size = tree.size();
        let scale = if mode == "crop" { (width as f32 / size.width()).max(height as f32 / size.height()) }
            else { (width as f32 / size.width()).min(height as f32 / size.height()).min(1.0) };
        let (w, h) = if mode == "crop" { (width, height) }
            else { ((size.width() * scale).round().max(1.0) as u32, (size.height() * scale).round().max(1.0) as u32) };
        let mut canvas = resvg::tiny_skia::Pixmap::new(w, h).ok_or("invalid SVG size")?;
        canvas.fill(resvg::tiny_skia::Color::from_rgba8(24, 25, 26, 255));
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale)
            .post_translate((w as f32 - size.width() * scale) / 2.0, (h as f32 - size.height() * scale) / 2.0), &mut canvas.as_mut());
        DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, canvas.take()).ok_or("invalid SVG pixels")?)
    };
    let preview = if mode == "crop" { image.resize_to_fill(width, height, image::imageops::FilterType::Triangle) }
        else if image.width() > width || image.height() > height { image.resize(width, height, image::imageops::FilterType::Triangle) }
        else { image };
    let rgba = preview.to_rgba8();
    Ok(image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let p = rgba.get_pixel(x, y).0;
        let a = p[3] as u32;
        image::Rgb(std::array::from_fn(|i| ((p[i] as u32 * a + (24 + i as u32) * (255 - a) + 127) / 255) as u8))
    }))
}

fn trim(cache: &Path, keep: &Path) {
    let Ok(entries) = fs::read_dir(cache) else { return; };
    let mut entries: Vec<_> = entries.flatten().filter_map(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        let stem = name.strip_suffix(".jpg")?;
        if stem.len() != 16 || !stem.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
        let metadata = entry.metadata().ok()?;
        if !metadata.is_file() { return None; }
        Some((metadata.modified().ok()?, entry.path(), metadata.len()))
    }).collect();
    entries.sort_by_key(|entry| entry.0);
    let mut bytes: u64 = entries.iter().map(|entry| entry.2).sum();
    let mut count = entries.len();
    for (_, path, length) in entries {
        if bytes <= CACHE_LIMIT && count <= 1024 { break; }
        if path != keep && fs::remove_file(path).is_ok() { bytes -= length; count -= 1; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    #[test]
    fn crops_centres_fits_without_upscaling_and_mattes_alpha() {
        let source = image::RgbaImage::from_fn(100, 50, |x, _| if (25..75).contains(&x) { image::Rgba([0, 255, 0, 255]) } else { image::Rgba([255, 0, 0, 255]) });
        let mut bytes = Cursor::new(Vec::new());
        source.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let crop = decode(bytes.get_ref(), 20, 20, "crop").unwrap();
        assert_eq!(crop.dimensions(), (20, 20));
        assert_eq!(crop.get_pixel(10, 10).0, [0, 255, 0]);
        assert_eq!(decode(bytes.get_ref(), 40, 40, "fit").unwrap().dimensions(), (40, 20));
        assert_eq!(decode(bytes.get_ref(), 200, 100, "fit").unwrap().dimensions(), (100, 50));
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="50000"><rect width="100000" height="50000" fill="#00ff00"/></svg>"##;
        let small = decode(svg, 20, 20, "fit").unwrap();
        assert_eq!(small.dimensions(), (20, 10));
        assert_eq!(small.get_pixel(10, 5).0, [0, 255, 0]);
        assert!(decode(b"not a picture", 20, 20, "crop").is_err());
    }
    #[test]
    fn unicode_cache_uses_content_and_retains_only_bounded_owned_files() {
        let root = std::env::temp_dir().join(format!("pleamar images ñ 海 {}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("picture ñ.png");
        let cache = root.join("cache");
        image::RgbImage::from_pixel(30, 60, image::Rgb([255, 0, 0])).save(&source).unwrap();
        let first = thumbnail(&source, 10, 10, "crop", &cache).unwrap();
        assert_eq!(thumbnail(&source, 10, 10, "crop", &cache).unwrap(), first);
        assert_eq!(image::open(&first).unwrap().dimensions(), (10, 10));
        image::RgbImage::from_pixel(30, 60, image::Rgb([0, 255, 0])).save(&source).unwrap();
        assert_ne!(thumbnail(&source, 10, 10, "crop", &cache).unwrap(), first);
        fs::write(cache.join("unrelated.txt"), b"keep").unwrap();
        for n in 0..1030 { fs::write(cache.join(format!("{n:016x}.jpg")), b"x").unwrap(); }
        trim(&cache, &first);
        assert!(first.exists());
        assert!(cache.join("unrelated.txt").exists());
        assert!(fs::read_dir(&cache).unwrap().count() <= 1025);
        // The test owns this exact directory beneath the OS temporary folder.
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_relative_paths_nonfinite_and_unbounded_sizes() {
        for size in [0.0, -1.0, 1025.0, f64::NAN, f64::INFINITY, 1.5] {
            assert!(query(&[SysValue::Text("C:/missing.png".into()), SysValue::Num(size), SysValue::Num(10.0), SysValue::Text("crop".into())]).is_err());
        }
        assert!(query(&[SysValue::Text("relative.png".into()), SysValue::Num(10.0), SysValue::Num(10.0), SysValue::Text("crop".into())]).is_err());
    }
}
