//! Bounded local artwork from the active GSMTC session (including browser media).
//! No title searches, network requests, or access to browser history.
use std::{io::Cursor, path::PathBuf, sync::Mutex, time::{Duration, Instant}};
use windows::{Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties as Properties,
    Storage::Streams::DataReader};

struct Cached { key: [String; 4], art: String, checked: Instant }
static CACHE: Mutex<Option<Cached>> = Mutex::new(None);
const MAX_BYTES: u64 = 4 * 1024 * 1024;

pub(super) fn read(properties: &Properties, key: [&str; 4]) -> String {
    let mut cache = CACHE.lock().unwrap();
    if let Some(old) = cache.as_ref() {
        if old.key.iter().zip(key).all(|(a, b)| a == b) && old.checked.elapsed() < Duration::from_secs(15) {
            return old.art.clone();
        }
    }
    // Missing, malformed or delayed thumbnails must not disable the controls
    // or leave the preceding track's image visible.
    let art = thumbnail(properties).unwrap_or_default();
    *cache = Some(Cached { key: key.map(str::to_owned), art: art.clone(), checked: Instant::now() });
    art
}

fn thumbnail(properties: &Properties) -> Option<String> {
    let stream = complete!(properties.Thumbnail().and_then(|t| t.OpenReadAsync())).ok()?;
    let size = stream.Size().ok()?;
    if size == 0 || size > MAX_BYTES { return None; }
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    let count = complete!(reader.LoadAsync(size as u32)).ok()?;
    if count as u64 != size { return None; }
    let mut bytes = vec![0; count as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    let _ = reader.Close();
    let _ = stream.Close();
    save(&bytes)
}

fn reduced(bytes: &[u8]) -> Option<image::RgbaImage> {
    if bytes.len() as u64 > MAX_BYTES { return None; }
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    Some(reader.decode().ok()?.thumbnail(192, 192).to_rgba8())
}

fn save(bytes: &[u8]) -> Option<String> {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hash);
    let dir = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("pleamar/media-artwork");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("cover-{:016x}.png", hash.finish()));
    if !path.is_file() {
        let image = reduced(bytes)?;
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        if image.save_with_format(&temporary, image::ImageFormat::Png).is_err() { let _ = std::fs::remove_file(&temporary); return None; }
        if std::fs::rename(&temporary, &path).is_err() { let _ = std::fs::remove_file(&temporary); if !path.is_file() { return None; } }
    }
    // Retain a small trailing set for asynchronous image loads and other scenes.
    let mut files: Vec<_> = std::fs::read_dir(&dir).ok()?.flatten().filter_map(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with("cover-") || !name.ends_with(".png") || e.path() == path { return None; }
        Some((e.metadata().ok()?.modified().ok()?, e.path()))
    }).collect();
    files.sort_by_key(|(time, _)| *time);
    let excess = files.len().saturating_sub(31);
    for (_, old) in files.into_iter().take(excess) { let _ = std::fs::remove_file(old); }
    Some(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artwork_is_decoded_bounded_and_keeps_its_aspect_ratio() {
        let mut bytes = Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(640, 360, image::Rgba([12, 34, 56, 255]))
            .write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        assert_eq!(reduced(bytes.get_ref()).unwrap().dimensions(), (192, 108));
        assert!(reduced(b"not an image").is_none());
        assert!(reduced(&vec![0; MAX_BYTES as usize + 1]).is_none());
    }
}
